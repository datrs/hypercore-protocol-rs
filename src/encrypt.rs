use crate::handshake::HandshakeResult;
use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use futures::stream::{FusedStream, Stream};
use futures_timer::Delay;
use salsa20::stream_cipher::{NewStreamCipher, SyncStreamCipher};
use salsa20::XSalsa20;
use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::constants::{DEFAULT_TIMEOUT, MAX_MESSAGE_SIZE};
use std::time::Duration;

// TODO: Don't define here but use the values from the XSalsa20 impl.
const KEY_SIZE: usize = 32;
const NONCE_SIZE: usize = 24;

pub struct Cipher(XSalsa20);

impl Cipher {
    pub fn from_handshake_rx(handshake: &HandshakeResult) -> Result<Self> {
        let cipher = XSalsa20::new_var(
            &handshake.split_rx[..KEY_SIZE],
            &handshake.remote_nonce[..NONCE_SIZE],
        )
        .map_err(|e| {
            Error::new(
                ErrorKind::PermissionDenied,
                format!("Cannot initialize cipher: {}", e),
            )
        })?;
        Ok(Self(cipher))
    }

    pub fn from_handshake_tx(handshake: &HandshakeResult) -> Result<Self> {
        let cipher = XSalsa20::new_var(
            &handshake.split_tx[..KEY_SIZE],
            &handshake.local_nonce[..NONCE_SIZE],
        )
        .map_err(|e| {
            Error::new(
                ErrorKind::PermissionDenied,
                format!("Cannot initialize cipher: {}", e),
            )
        })?;
        Ok(Self(cipher))
    }

    pub fn apply(&mut self, buffer: &mut [u8]) {
        self.0.apply_keystream(buffer);
    }
}

pub struct EncryptedReader<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    cipher: Option<Cipher>,
    reader: R,
    state: Option<State>,
}

impl<R> EncryptedReader<R>
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

pub struct EncryptedWriter<W>
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    cipher: Option<Cipher>,
    writer: W,
}

impl<W> EncryptedWriter<W>
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    pub fn new(writer: W) -> Self {
        Self {
            cipher: None,
            writer,
        }
    }

    pub fn into_inner(self) -> W {
        self.writer
    }

    pub fn upgrade_with_handshake(&mut self, handshake: &HandshakeResult) -> Result<()> {
        let cipher = Cipher::from_handshake_tx(handshake)?;
        self.cipher = Some(cipher);
        Ok(())
    }

    pub async fn send_raw(&mut self, buf: &[u8]) -> Result<()> {
        self.write_all(&buf).await?;
        self.flush().await
    }

    pub async fn send_prefixed(&mut self, buf: &[u8]) -> Result<()> {
        let len = buf.len();
        let prefix_len = varinteger::length(len as u64);
        let mut prefix_buf = vec![0u8; prefix_len];
        varinteger::encode(len as u64, &mut prefix_buf[..prefix_len]);
        // trace!("send len {} {:?}", buf.len(), buf);
        self.write_all(&prefix_buf).await?;
        self.write_all(&buf).await?;
        self.flush().await
    }

    pub async fn ping(&mut self) -> Result<()> {
        let buf = vec![0u8];
        self.send_raw(&buf).await
    }
}

impl<R> AsyncRead for EncryptedReader<R>
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

impl<W> AsyncWrite for EncryptedWriter<W>
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<Result<usize>> {
        let mut buffer = buf.to_vec();

        if let Some(ref mut cipher) = &mut self.cipher {
            cipher.apply(&mut buffer);
        }

        let sent = futures::ready!(Pin::new(&mut self.writer).poll_write(cx, &buffer))?;
        Poll::Ready(Ok(sent))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<()>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<()>> {
        Pin::new(&mut self.writer).poll_close(cx)
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
    // closed: bool,
    // last_recv: Option<instant::Instant>,
    timeout: Delay,
    decrypted: bool,
}

impl Default for State {
    fn default() -> State {
        State {
            buf: vec![0u8; MAX_MESSAGE_SIZE as usize],
            cap: 0,
            step: Step::default(),
            timeout: Delay::new(Duration::from_secs(DEFAULT_TIMEOUT as u64)),
            decrypted: false, // last_recv: None, // closed: false,
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

impl<R> Stream for EncryptedReader<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    type Item = Result<Vec<u8>>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.state.is_none() {
            return Poll::Ready(None);
        }
        let mut state = self.state.take().unwrap();
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
            Poll::Ready(n) => n?,
            // If the reader is pending, poll the timeout.
            Poll::Pending => match Pin::new(&mut state.timeout).poll(cx) {
                // If the timeout is pending, return Pending.
                Poll::Pending => {
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
            },
        };

        state.cap += n;
        state
            .timeout
            .reset(Duration::from_secs(DEFAULT_TIMEOUT as u64));

        // Now process our buffer again.
        let result = process_state(&mut state);
        self.state = Some(state);
        match result {
            Some(_) => Poll::Ready(result),
            None => Poll::Pending,
        }
    }
}

impl<R> FusedStream for EncryptedReader<R>
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
