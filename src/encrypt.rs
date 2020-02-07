use crate::handshake::HandshakeResult;
use futures::io::{AsyncRead, AsyncWrite};
use salsa20::stream_cipher::{NewStreamCipher, SyncStreamCipher};
use salsa20::XSalsa20;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::{Context, Poll};

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
}

impl<R> EncryptedReader<R>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    pub fn new(reader: R) -> Self {
        Self {
            cipher: None,
            reader,
        }
    }

    pub fn upgrade_with_handshake(&mut self, handshake: &HandshakeResult) -> Result<()> {
        let cipher = Cipher::from_handshake_rx(handshake)?;
        self.cipher = Some(cipher);
        Ok(())
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

    pub fn upgrade_with_handshake(&mut self, handshake: &HandshakeResult) -> Result<()> {
        let cipher = Cipher::from_handshake_tx(handshake)?;
        self.cipher = Some(cipher);
        Ok(())
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
