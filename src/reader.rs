use crate::noise::{Cipher, HandshakeResult};
use futures_lite::io::AsyncRead;
use futures_lite::stream::Stream;
use futures_timer::Delay;
use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::constants::{DEFAULT_TIMEOUT, MAX_MESSAGE_SIZE};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(DEFAULT_TIMEOUT as u64);

pub struct ProtocolReader<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    reader: R,
    state: State,
}

impl<R> ProtocolReader<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            state: State::new(),
        }
    }

    pub fn upgrade_with_handshake(&mut self, handshake: &HandshakeResult) -> Result<()> {
        self.state.upgrade_with_handshake(handshake)
    }

    pub fn into_inner(self) -> R {
        self.reader
    }
}

impl<R> Stream for ProtocolReader<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    type Item = Result<Vec<u8>>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let state = &mut this.state;
        let reader = &mut this.reader;
        let result = state.poll_reader(reader, cx);
        result.map(Some)
    }
}

#[derive(Debug)]
struct State {
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
    /// Optional encryption cipher.
    cipher: Option<Cipher>,
}

impl State {
    pub fn new() -> State {
        State {
            buf: vec![0u8; MAX_MESSAGE_SIZE as usize],
            start: 0,
            end: 0,
            step: Step::Header,
            timeout: Delay::new(TIMEOUT),
            cipher: None,
        }
    }
}

#[derive(Debug)]
enum Step {
    Header,
    Body { header_len: usize, body_len: usize },
}

impl State {
    pub fn upgrade_with_handshake(&mut self, handshake: &HandshakeResult) -> Result<()> {
        let mut cipher = Cipher::from_handshake_rx(handshake)?;
        cipher.apply(&mut self.buf[self.start..self.end]);
        self.cipher = Some(cipher);
        Ok(())
    }

    pub fn poll_reader<R>(&mut self, mut reader: &mut R, cx: &mut Context) -> Poll<Result<Vec<u8>>>
    where
        R: AsyncRead + Unpin,
    {
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

    fn process(&mut self) -> Option<Result<Vec<u8>>> {
        if self.start == self.end {
            return None;
        }
        loop {
            match self.step {
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
                Step::Body {
                    header_len,
                    body_len,
                } => {
                    let message_len = header_len + body_len;
                    if (self.end - self.start) < message_len {
                        self.cycle_buf_if_needed();
                        return None;
                    } else {
                        let range = (self.start + header_len)..(self.start + message_len);
                        let message = self.buf[range].to_vec();
                        self.start += message_len;
                        self.step = Step::Header;
                        return Some(Ok(message));
                    }
                }
            }
        }
    }
}

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
