use crate::noise::{Cipher, HandshakeResult};
use futures::io::AsyncWrite;
use futures::ready;
use std::io::Result;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct ProtocolWriter<W>
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    cipher: Option<Cipher>,
    writer: W,
    pos: usize,
}

impl<W> ProtocolWriter<W>
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    pub fn new(writer: W) -> Self {
        Self {
            cipher: None,
            writer,
            pos: 0,
        }
    }

    pub fn poll_write_message(
        mut self: Pin<&mut Self>,
        message: &[u8],
        mut cx: &mut Context,
    ) -> Poll<Result<()>> {
        while self.pos < message.len() - 1 {
            let pos = self.pos;
            let n = ready!(ProtocolWriter::poll_write(
                Pin::new(&mut self),
                cx,
                &message[pos..]
            ));
            let n = n?;
            self.pos += n;
        }
        self.poll_flush(&mut cx)
    }

    pub fn reset(self: &mut Self) {
        self.pos = 0;
    }

    pub fn into_inner(self) -> W {
        self.writer
    }

    pub fn upgrade_with_handshake(&mut self, handshake: &HandshakeResult) -> Result<()> {
        let cipher = Cipher::from_handshake_tx(handshake)?;
        self.cipher = Some(cipher);
        Ok(())
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
