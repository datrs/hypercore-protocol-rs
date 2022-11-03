use crate::message::EncodeError;

#[cfg(feature = "v10")]
use crate::message_v10::{Encoder, Frame};
#[cfg(feature = "v9")]
use crate::message_v9::{Encoder, Frame};
#[cfg(feature = "v9")]
use crate::noise::Cipher;
#[cfg(feature = "v10")]
use crate::noise::EncryptCipher;
use crate::noise::HandshakeResult;

use futures_lite::{ready, AsyncWrite};
use std::collections::VecDeque;
use std::fmt;
use std::io::Result;
use std::pin::Pin;
use std::task::{Context, Poll};

const BUF_SIZE: usize = 1024 * 64;

#[derive(Debug)]
pub enum Step {
    Flushing,
    Writing,
    Processing,
}

pub struct WriteState {
    queue: VecDeque<Frame>,
    buf: Vec<u8>,
    current_frame: Option<Frame>,
    start: usize,
    end: usize,
    #[cfg(feature = "v9")]
    cipher: Option<Cipher>,
    #[cfg(feature = "v10")]
    cipher: Option<EncryptCipher>,
    step: Step,
}

impl fmt::Debug for WriteState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteState")
            .field("queue (len)", &self.queue.len())
            .field("step", &self.step)
            .field("buf (len)", &self.buf.len())
            .field("current_frame", &self.current_frame)
            .field("start", &self.start)
            .field("end", &self.end)
            .field("cipher", &self.cipher.is_some())
            .finish()
    }
}

impl WriteState {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            buf: vec![0u8; BUF_SIZE],
            current_frame: None,
            start: 0,
            end: 0,
            cipher: None,
            step: Step::Processing,
        }
    }

    pub fn queue_frame<F>(&mut self, frame: F)
    where
        F: Into<Frame>,
    {
        self.queue.push_back(frame.into())
    }

    #[cfg(feature = "v9")]
    pub fn try_queue_direct<T: Encoder>(
        &mut self,
        frame: &T,
    ) -> std::result::Result<bool, EncodeError> {
        let len = frame.encoded_len();
        if self.buf.len() < len {
            self.buf.resize(len, 0u8);
        }
        if len > self.remaining() {
            return Ok(false);
        }
        let len = frame.encode(&mut self.buf[self.end..])?;
        self.advance(len)?;
        Ok(true)
    }

    #[cfg(feature = "v10")]
    pub fn try_queue_direct<T: Encoder>(
        &mut self,
        frame: &mut T,
    ) -> std::result::Result<bool, EncodeError> {
        let promised_len = frame.encoded_len();
        let padded_promised_len = self.safe_encrypted_len(promised_len);
        println!(
            "Writer::try_queue_direct, promised={}, with_padding={}",
            promised_len, padded_promised_len
        );
        if self.buf.len() < padded_promised_len {
            self.buf.resize(padded_promised_len, 0u8);
        }
        if padded_promised_len > self.remaining() {
            return Ok(false);
        }
        let actual_len = frame.encode(&mut self.buf[self.end..])?;
        if actual_len != promised_len {
            panic!(
                "encoded_len() did not return that right size, expected={}, actual={}",
                promised_len, actual_len
            );
        }
        self.advance(padded_promised_len)?;
        Ok(true)
    }

    pub fn can_park_frame(&self) -> bool {
        self.current_frame.is_none()
    }

    pub fn park_frame<F>(&mut self, frame: F)
    where
        F: Into<Frame>,
    {
        if self.current_frame.is_none() {
            self.current_frame = Some(frame.into())
        }
    }

    fn advance(&mut self, n: usize) -> std::result::Result<(), EncodeError> {
        let end = self.end + n;

        let encrypted_end = if let Some(ref mut cipher) = self.cipher {
            #[cfg(feature = "v9")]
            {
                cipher.apply(&mut self.buf[self.end..end]);
                end
            }
            #[cfg(feature = "v10")]
            {
                println!(
                    "Writer::advance: encrypting buf range {}..{}({}): {:02X?}",
                    self.end,
                    end,
                    end - self.end,
                    &self.buf[self.end..end]
                );
                self.end + cipher.encrypt(&mut self.buf[self.end..end])?
            }
        } else {
            end
        };

        self.end = encrypted_end;
        Ok(())
    }

    #[cfg(feature = "v9")]
    pub fn upgrade_with_handshake(&mut self, handshake: &HandshakeResult) -> Result<()> {
        let cipher = Cipher::from_handshake_tx(handshake)?;
        self.cipher = Some(cipher);
        Ok(())
    }

    #[cfg(feature = "v10")]
    pub fn upgrade_with_encrypt_cipher(&mut self, encrypt_cipher: EncryptCipher) {
        self.cipher = Some(encrypt_cipher);
    }

    fn remaining(&self) -> usize {
        self.buf.len() - self.end
    }

    fn pending(&self) -> usize {
        self.end - self.start
    }

    pub fn poll_send<W>(&mut self, cx: &mut Context<'_>, mut writer: &mut W) -> Poll<Result<()>>
    where
        W: AsyncWrite + Unpin,
    {
        loop {
            self.step = match self.step {
                Step::Processing => {
                    if self.current_frame.is_none() && !self.queue.is_empty() {
                        self.current_frame = self.queue.pop_front();
                    }

                    #[cfg(feature = "v9")]
                    if let Some(frame) = self.current_frame.take() {
                        if !self.try_queue_direct(&frame)? {
                            self.current_frame = Some(frame);
                        }
                    }
                    #[cfg(feature = "v10")]
                    if let Some(mut frame) = self.current_frame.take() {
                        if !self.try_queue_direct(&mut frame)? {
                            self.current_frame = Some(frame);
                        }
                    }

                    if self.pending() == 0 {
                        return Poll::Ready(Ok(()));
                    }
                    Step::Writing
                }
                Step::Writing => {
                    let n = ready!(
                        Pin::new(&mut writer).poll_write(cx, &self.buf[self.start..self.end])
                    )?;
                    self.start += n;
                    if self.start == self.end {
                        self.start = 0;
                        self.end = 0;
                    }
                    Step::Flushing
                }
                Step::Flushing => {
                    ready!(Pin::new(&mut writer).poll_flush(cx))?;
                    Step::Processing
                }
            }
        }
    }

    fn safe_encrypted_len(&self, encoded_len: usize) -> usize {
        if let Some(cipher) = &self.cipher {
            #[cfg(feature = "v9")]
            {
                encoded_len
            }
            #[cfg(feature = "v10")]
            {
                cipher.safe_encrypted_len(encoded_len)
            }
        } else {
            encoded_len
        }
    }
}
