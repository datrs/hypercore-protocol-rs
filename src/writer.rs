use crate::noise::{Cipher, HandshakeResult};
use futures::io::{AsyncWrite, AsyncWriteExt};
use std::io::Result;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct ProtocolWriter<W>
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    cipher: Option<Cipher>,
    writer: W,
}

impl<W> ProtocolWriter<W>
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

impl<W> AsyncWrite for ProtocolWriter<W>
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
