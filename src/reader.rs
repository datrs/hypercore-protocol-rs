use crate::noise::{Cipher, HandshakeResult};
use futures::io::AsyncRead;
use futures::stream::{FusedStream, Stream};
use futures_timer::Delay;
use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::constants::{DEFAULT_TIMEOUT, MAX_MESSAGE_SIZE};
use std::time::Duration;

pub struct ProtocolReader<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    cipher: Option<Cipher>,
    reader: R,
    state: Option<State>,
}

impl<R> ProtocolReader<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    pub fn new(reader: R) -> Self {
        Self {
            cipher: None,
            reader,
            state: Some(State::default()),
        }
    }

    pub fn upgrade_with_handshake(&mut self, handshake: &HandshakeResult) -> Result<()> {
        let cipher = Cipher::from_handshake_rx(handshake)?;
        self.cipher = Some(cipher);
        Ok(())
    }

    pub fn into_inner(self) -> R {
        self.reader
    }
}

impl<R> AsyncRead for ProtocolReader<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        let len = futures::ready!(Pin::new(&mut self.reader).poll_read(cx, buf))?;

        if let Some(ref mut cipher) = &mut self.cipher {
            cipher.apply(&mut buf[..len]);
        }

        Poll::Ready(Ok(len))
    }
}

#[derive(Debug)]
struct State {
    /// The read buffer.
    buf: Vec<u8>,
    /// The number of relevant bytes in the read buffer.
    cap: usize,
    /// The logical state of the reading (either header or body).
    step: Step,
    /// The timeout after which the connection is closed.
    timeout: Delay,
    /// Whether the read buffer is already decrypted.
    decrypted: bool,
}

impl Default for State {
    fn default() -> State {
        State {
            buf: vec![0u8; MAX_MESSAGE_SIZE as usize],
            cap: 0,
            step: Step::default(),
            timeout: Delay::new(Duration::from_secs(DEFAULT_TIMEOUT as u64)),
            decrypted: false,
        }
    }
}

#[derive(Debug)]
enum Step {
    Header { factor: u64, varint: u64 },
    Body { header_len: usize, body_len: usize },
}
impl Default for Step {
    fn default() -> Step {
        Step::Header {
            factor: 1,
            varint: 0,
        }
    }
}

impl<R> Stream for ProtocolReader<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    type Item = Result<Vec<u8>>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.state.is_none() {
            return Poll::Ready(None);
        }

        loop {
            let mut state = self.state.take().unwrap();
            // eprintln!("READER NEXT cap {} state {:?}", state.cap, state.step);
            if !state.decrypted {
                if let Some(cipher) = &mut self.cipher {
                    cipher.apply(&mut state.buf[..state.cap]);
                    state.decrypted = true
                }
            }

            // First process our existing buffer, if any.
            let result = process_state(&mut state);
            if result.is_some() {
                self.state = Some(state);
                return Poll::Ready(result);
            }

            // Try to read from our reader.
            let n = match Pin::new(&mut self).poll_read(cx, &mut state.buf[state.cap..]) {
                Poll::Ready(Ok(n)) if n > 0 => n,
                Poll::Ready(Err(e)) => return Poll::Ready(Some(Err(e))),
                // If the reader is pending, poll the timeout.
                Poll::Pending | Poll::Ready(Ok(_)) => {
                    match Pin::new(&mut state.timeout).poll(cx) {
                        // If the timeout is pending, return Pending.
                        Poll::Pending => {
                            // eprintln!("READER PENDING");
                            self.state = Some(state);
                            return Poll::Pending;
                        }
                        // If the timeout is ready, return a timeout error and close by not resetting state.
                        Poll::Ready(_) => {
                            return Poll::Ready(Some(Err(Error::new(
                                ErrorKind::TimedOut,
                                "Remote timed out",
                            ))));
                        }
                    }
                }
            };

            state.cap += n;
            state
                .timeout
                .reset(Duration::from_secs(DEFAULT_TIMEOUT as u64));

            // Now process our buffer again.
            let result = process_state(&mut state);
            self.state = Some(state);
            if result.is_some() {
                return Poll::Ready(result);
            }
        }
    }
}

impl<R> FusedStream for ProtocolReader<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    fn is_terminated(&self) -> bool {
        self.state.is_none()
    }
}

fn process_state(state: &mut State) -> Option<Result<Vec<u8>>> {
    // Keep processing our current buffer until we need more bytes or have a full message.
    if state.cap == 0 {
        return None;
    }
    let mut needs_more_bytes = false;
    let mut result = None;
    while !needs_more_bytes && result.is_none() {
        match state.step {
            // Read a varint.
            Step::Header {
                ref mut factor,
                ref mut varint,
            } => {
                needs_more_bytes = true;
                for (i, byte) in state.buf[..state.cap].iter().enumerate() {
                    // Ignore empty keepalive bytes.
                    if byte == &0 {
                        continue;
                    }
                    *varint += (*byte as u64 & 127) * *factor;
                    if *varint > MAX_MESSAGE_SIZE {
                        return Some(Err(Error::new(ErrorKind::InvalidInput, "Message too long")));
                    }
                    if byte < &128 {
                        state.step = Step::Body {
                            header_len: i + 1,
                            body_len: *varint as usize,
                        };
                        needs_more_bytes = false;
                        break;
                    }
                    *factor *= 128;
                }
            }
            // Read the actual message.
            Step::Body {
                header_len,
                body_len,
            } => {
                let message_len = header_len + body_len;
                if message_len > state.cap {
                    // Not enough bytes for a full message, return to reading.
                    needs_more_bytes = true
                } else {
                    // We have enough bytes for a full message!
                    let message_buf = &state.buf[header_len..message_len];

                    result = Some(Ok(message_buf.to_vec()));
                    // self.on_message(message_buf).await?;

                    // If we have even more bytes, copy them to the beginning of our read
                    // buffer and adjust the cap accordingly.
                    if state.cap > message_len {
                        // TODO: If we were using a ring buffer we wouldn't have to copy and
                        // allocate here.
                        let overflow_buf = &state.buf[message_len..state.cap].to_vec();
                        state.cap -= message_len;
                        state.buf[..state.cap].copy_from_slice(&overflow_buf[..]);
                    // Otherwise, read again!
                    } else {
                        state.cap = 0;
                        needs_more_bytes = true;
                    }
                    // In any case, after reading a message the next step is to read a varint.
                    state.step = Step::Header {
                        factor: 1,
                        varint: 0,
                    };
                }
            }
        }
    }
    result
}
