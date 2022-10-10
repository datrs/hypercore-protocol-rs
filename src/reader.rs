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
    Body { header_len: usize, body_len: usize },
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
    pub fn upgrade_with_handshake_result(
        &mut self,
        handshake_result: &HandshakeResult,
    ) -> Result<()> {
        let cipher = DecryptCipher::from_handshake_rx(handshake_result)?;
        self.cipher = Some(cipher);
        Ok(())
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
        loop {
            println!("reader.rs: poll_reader loop");
            if let Some(result) = self.process() {
                println!("reader.rs: returning Poll:Ready");
                return Poll::Ready(result);
            }
            println!("reader.rs: process not ready");

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
            println!(
                "reader.rs: got n={}, start={}, end={}, data={:02X?}",
                n,
                self.end,
                end,
                &self.buf[self.end..end]
            );
            let decoded_end = if let Some(ref mut cipher) = self.cipher {
                #[cfg(feature = "v9")]
                {
                    cipher.apply(&mut self.buf[self.end..end]);
                    end
                }
                #[cfg(feature = "v10")]
                {
                    self.end + cipher.decrypt(&mut self.buf[self.end..end])?
                }
            } else {
                end
            };
            self.end = decoded_end;
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
                    println!(
                        "reader: Step:Header: start={}, end={}, buf={:02X?}",
                        self.start,
                        self.end,
                        &self.buf[self.start..self.end]
                    );
                    let stat = stat_uint24_le(&self.buf[self.start..self.end]);
                    if let Some((header_len, body_len)) = stat {
                        println!(
                            "reader: Step:Header: header_len={}, body_len={}",
                            header_len, body_len
                        );
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

                Step::Body {
                    header_len,
                    body_len,
                } => {
                    println!(
                        "reader: Step::Body header_len={}, body_len={}",
                        header_len, body_len
                    );
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
