use crate::crypto::DecryptCipher;
use futures_lite::io::AsyncRead;
use futures_timer::Delay;
use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::constants::{DEFAULT_TIMEOUT, MAX_MESSAGE_SIZE};
use crate::message::{Frame, FrameType};
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
    cipher: Option<DecryptCipher>,
    /// The frame type to be passed to the decoder.
    frame_type: FrameType,
}

impl ReadState {
    pub fn new() -> ReadState {
        ReadState {
            buf: vec![0u8; READ_BUF_INITIAL_SIZE],
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
    Batch,
}

impl ReadState {
    pub fn upgrade_with_decrypt_cipher(&mut self, decrypt_cipher: DecryptCipher) {
        self.cipher = Some(decrypt_cipher);
    }

    /// Decrypts a given buf with stored cipher, if present. Used to correct
    /// the rare mistake that more than two messages came in where the first
    /// one created the cipher, and the next one should have been decrypted
    /// but wasn't.
    pub fn decrypt_buf(&mut self, buf: &[u8]) -> Result<Vec<u8>> {
        if let Some(cipher) = self.cipher.as_mut() {
            Ok(cipher.decrypt_buf(buf)?.0)
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
        let mut incomplete = true;
        loop {
            if !incomplete {
                if let Some(result) = self.process() {
                    return Poll::Ready(result);
                }
            } else {
                incomplete = false;
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
            let (success, segments) = create_segments(&self.buf[self.start..end])?;
            if success {
                if let Some(ref mut cipher) = self.cipher {
                    let mut dec_end = self.start;
                    for (index, header_len, body_len) in segments {
                        let de = cipher.decrypt(
                            &mut self.buf[self.start + index..end],
                            header_len,
                            body_len,
                        )?;
                        dec_end = self.start + index + de;
                    }
                    self.end = dec_end;
                } else {
                    self.end = end;
                }
            } else {
                // Could not segment due to buffer being full, need to cycle the buffer
                // and possibly resize it too if the message is too big.
                self.cycle_buf_and_resize_if_needed(segments[segments.len() - 1]);

                // Set incomplete flag to skip processing and instead poll more data
                incomplete = true;
            }
            self.timeout.reset(TIMEOUT);
        }
    }

    fn cycle_buf_and_resize_if_needed(&mut self, last_segment: (usize, usize, usize)) {
        let (last_index, last_header_len, last_body_len) = last_segment;
        let total_incoming_length = last_index + last_header_len + last_body_len;
        if self.buf.len() < total_incoming_length {
            // The incoming segments will not fit into the buffer, need to resize it
            self.buf.resize(total_incoming_length, 0u8);
        }
        let temp = self.buf[self.start..].to_vec();
        let len = temp.len();
        self.buf[..len].copy_from_slice(&temp[..]);
        self.end = len;
        self.start = 0;
    }

    fn process(&mut self) -> Option<Result<Frame>> {
        loop {
            match self.step {
                Step::Header => {
                    let stat = stat_uint24_le(&self.buf[self.start..self.end]);
                    if let Some((header_len, body_len)) = stat {
                        if body_len == 0 {
                            // This is a keepalive message, just remain in Step::Header
                            self.start += header_len;
                            return None;
                        } else if (self.start + header_len + body_len as usize) < self.end {
                            // There are more than one message here, create a batch from all of
                            // then
                            self.step = Step::Batch;
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
                        return Some(Err(Error::new(ErrorKind::InvalidData, "Invalid header")));
                    }
                }

                Step::Body {
                    header_len,
                    body_len,
                } => {
                    let message_len = header_len + body_len;
                    let range = self.start + header_len..self.start + message_len;
                    let frame = Frame::decode(&self.buf[range], &self.frame_type);
                    self.start += message_len;
                    self.step = Step::Header;
                    return Some(frame);
                }
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

#[allow(clippy::type_complexity)]
pub fn create_segments(buf: &[u8]) -> Result<(bool, Vec<(usize, usize, usize)>)> {
    let mut index: usize = 0;
    let len = buf.len();
    let mut segments: Vec<(usize, usize, usize)> = vec![];
    while index < len {
        if let Some((header_len, body_len)) = stat_uint24_le(&buf[index..]) {
            let body_len = body_len as usize;
            segments.push((index, header_len, body_len));
            if len < index + header_len + body_len {
                // The segments will not fit, return false to indicate that more needs to be read
                return Ok((false, segments));
            }
            index += header_len + body_len;
        } else {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Could not read header while decrypting",
            ));
        }
    }
    Ok((true, segments))
}
