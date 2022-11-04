#[cfg(feature = "v9")]
use crate::noise::Cipher;
#[cfg(feature = "v10")]
use crate::noise::DecryptCipher;
use crate::noise::HandshakeResult;
use futures_lite::io::AsyncRead;
use futures_timer::Delay;
use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::constants::{DEFAULT_TIMEOUT, MAX_MESSAGE_SIZE};
use crate::message::FrameType;
#[cfg(feature = "v10")]
use crate::message_v10::Frame;
#[cfg(feature = "v9")]
use crate::message_v9::Frame;
#[cfg(feature = "v10")]
use crate::util::stat_uint24_le;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(DEFAULT_TIMEOUT as u64);
const READ_BUF_INITIAL_SIZE: usize = 1024 * 128;

#[derive(Debug)]
pub struct ReadState {
    /// The read buffer.
    buf: Vec<u8>,
    /// The start of the not-yet-processed byte range in the read buffer.
    start: usize,
    /// The end of the not-yet-processed byte range in the read buffer.
    end: usize,
    /// The logical state of the reading (either header or body).
    step: Step,
    /// The timeout after which the connection is closed.
    timeout: Delay,
    /// Optional decryption cipher.
    #[cfg(feature = "v9")]
    cipher: Option<Cipher>,
    #[cfg(feature = "v10")]
    cipher: Option<DecryptCipher>,
    /// The frame type to be passed to the decoder.
    frame_type: FrameType,
}

impl ReadState {
    pub fn new() -> ReadState {
        ReadState {
            buf: vec![0u8; READ_BUF_INITIAL_SIZE as usize],
            start: 0,
            end: 0,
            step: Step::Header,
            timeout: Delay::new(TIMEOUT),
            cipher: None,
            frame_type: FrameType::Raw,
        }
    }
}

#[derive(Debug)]
enum Step {
    Header,
    Body {
        header_len: usize,
        body_len: usize,
    },
    /// Multiple messages one after another
    #[cfg(feature = "v10")]
    Batch,
}

impl ReadState {
    #[cfg(feature = "v9")]
    pub fn upgrade_with_handshake(&mut self, handshake: &HandshakeResult) -> Result<()> {
        let mut cipher = Cipher::from_handshake_rx(handshake)?;
        cipher.apply(&mut self.buf[self.start..self.end]);
        self.cipher = Some(cipher);
        Ok(())
    }

    #[cfg(feature = "v10")]
    pub fn upgrade_with_decrypt_cipher(&mut self, decrypt_cipher: DecryptCipher) {
        self.cipher = Some(decrypt_cipher);
    }

    /// Decrypts a given buf with stored cipher, if present. Used to correct
    /// the rare mistake that more than two messages came in where the first
    /// one created the cipher, and the next one should have been decrypted
    /// but wasn't.
    #[cfg(feature = "v10")]
    pub fn decrypt_buf(&mut self, buf: &[u8]) -> Result<Vec<u8>> {
        if let Some(cipher) = self.cipher.as_mut() {
            Ok(cipher.decrypt_buf(&buf)?.0)
        } else {
            Ok(buf.to_vec())
        }
    }

    pub fn set_frame_type(&mut self, frame_type: FrameType) {
        self.frame_type = frame_type;
    }

    pub fn poll_reader<R>(
        &mut self,
        cx: &mut Context<'_>,
        mut reader: &mut R,
    ) -> Poll<Result<Frame>>
    where
        R: AsyncRead + Unpin,
    {
        #[cfg(feature = "v9")]
        loop {
            if let Some(result) = self.process() {
                return Poll::Ready(result);
            }

            let n = match Pin::new(&mut reader).poll_read(cx, &mut self.buf[self.end..]) {
                Poll::Ready(Ok(n)) if n > 0 => n,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                // If the reader is pending, poll the timeout.
                Poll::Pending | Poll::Ready(Ok(_)) => {
                    // Return Pending if the timeout is pending, or an error if the
                    // timeout expired (i.e. returned Poll::Ready).
                    return Pin::new(&mut self.timeout)
                        .poll(cx)
                        .map(|()| Err(Error::new(ErrorKind::TimedOut, "Remote timed out")));
                }
            };

            let end = self.end + n;
            if let Some(ref mut cipher) = self.cipher {
                cipher.apply(&mut self.buf[self.end..end]);
            }
            self.end = end;
            self.timeout.reset(TIMEOUT);
        }

        #[cfg(feature = "v10")]
        loop {
            if let Some(result) = self.process() {
                return Poll::Ready(result);
            }
            let n = match Pin::new(&mut reader).poll_read(cx, &mut self.buf[self.end..]) {
                Poll::Ready(Ok(n)) if n > 0 => n,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                // If the reader is pending, poll the timeout.
                Poll::Pending | Poll::Ready(Ok(_)) => {
                    // Return Pending if the timeout is pending, or an error if the
                    // timeout expired (i.e. returned Poll::Ready).
                    return Pin::new(&mut self.timeout)
                        .poll(cx)
                        .map(|()| Err(Error::new(ErrorKind::TimedOut, "Remote timed out")));
                }
            };

            let end = self.end + n;
            if let Some(ref mut cipher) = self.cipher {
                let mut dec_end = self.end;
                let mut enc_end = self.end;
                // Go through the whole section, encrypt all sections if there are many
                while enc_end < end {
                    let (de, ee) = cipher.decrypt(&mut self.buf[enc_end..end])?;
                    dec_end = enc_end + de;
                    enc_end += ee;
                }
                self.end = dec_end;
            } else {
                self.end = end;
            }

            self.timeout.reset(TIMEOUT);
        }
    }

    fn cycle_buf_if_needed(&mut self) {
        // TODO: It would be great if we wouldn't have to allocate here.
        if self.end == self.buf.len() {
            let temp = self.buf[self.start..self.end].to_vec();
            let len = temp.len();
            self.buf[..len].copy_from_slice(&temp[..]);
            self.end = len;
            self.start = 0;
        }
    }

    fn process(&mut self) -> Option<Result<Frame>> {
        if self.start == self.end {
            return None;
        }
        loop {
            match self.step {
                #[cfg(feature = "v9")]
                Step::Header => {
                    let varint = varint_decode(&self.buf[self.start..self.end]);
                    if let Some((header_len, body_len)) = varint {
                        let body_len = body_len as usize;
                        if body_len > MAX_MESSAGE_SIZE as usize {
                            return Some(Err(Error::new(
                                ErrorKind::InvalidData,
                                "Message length above max allowed size",
                            )));
                        }
                        self.step = Step::Body {
                            header_len,
                            body_len,
                        };
                    } else {
                        self.cycle_buf_if_needed();
                        return None;
                    }
                }
                #[cfg(feature = "v10")]
                Step::Header => {
                    let stat = stat_uint24_le(&self.buf[self.start..self.end]);
                    if let Some((header_len, body_len)) = stat {
                        if (self.start + header_len + body_len as usize) < self.end {
                            // There are more than one message here, create a batch from all of
                            // then
                            self.step = Step::Batch;
                        } else if body_len == 0 {
                            // This is a keepalive message, just remain in Step::Header
                            self.cycle_buf_if_needed();
                            return None;
                        } else {
                            let body_len = body_len as usize;
                            if body_len > MAX_MESSAGE_SIZE as usize {
                                return Some(Err(Error::new(
                                    ErrorKind::InvalidData,
                                    "Message length above max allowed size",
                                )));
                            }
                            self.step = Step::Body {
                                header_len,
                                body_len,
                            };
                        }
                    } else {
                        self.cycle_buf_if_needed();
                        return None;
                    }
                }

                Step::Body {
                    header_len,
                    body_len,
                } => {
                    let message_len = header_len + body_len;
                    if message_len > self.buf.len() {
                        self.buf.resize(message_len, 0u8);
                    }
                    if (self.end - self.start) < message_len {
                        self.cycle_buf_if_needed();
                        return None;
                    } else {
                        let range = self.start + header_len..self.start + message_len;
                        let frame = Frame::decode(&self.buf[range], &self.frame_type);
                        self.start += message_len;
                        self.step = Step::Header;
                        return Some(frame);
                    }
                }
                #[cfg(feature = "v10")]
                Step::Batch => {
                    let frame =
                        Frame::decode_multiple(&self.buf[self.start..self.end], &self.frame_type);
                    self.start = self.end;
                    self.step = Step::Header;
                    return Some(frame);
                }
            }
        }
    }
}

#[cfg(feature = "v9")]
fn varint_decode(buf: &[u8]) -> Option<(usize, u64)> {
    let mut value = 0u64;
    let mut m = 1u64;
    let mut offset = 0usize;
    for _i in 0..8 {
        if offset >= buf.len() {
            return None;
        }
        let byte = buf[offset];
        offset += 1;
        value += m * u64::from(byte & 127);
        m *= 128;
        if byte & 128 == 0 {
            break;
        }
    }
    Some((offset, value))
}
