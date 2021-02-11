use crate::noise::{Cipher, HandshakeResult};
use crate::util::length_prefix;
use futures::io::AsyncWrite;
use futures::ready;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{collections::VecDeque, io::Result};

pub struct ProtocolWriter<W>
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    cipher: Option<Cipher>,
    writer: W,
    pos: usize,
    queue: VecDeque<Vec<u8>>,
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
            queue: VecDeque::new(),
        }
    }

    pub fn queue_message(&mut self, message: Vec<u8>) {
        let prefix = length_prefix(&message);
        self.queue_raw(prefix);
        self.queue_raw(message)
    }

    pub fn queue_raw(&mut self, mut message: Vec<u8>) {
        self.encrypt(&mut message);
        self.queue.push_back(message);
    }

    pub fn poll_write_all(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<()>> {
        let this = self.get_mut();
        while let Some(message) = this.queue.pop_front() {
            match this.poll_write_message(&message, cx) {
                Poll::Ready(Ok(_)) => {}
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => {
                    this.queue.push_front(message);
                    return Poll::Pending;
                }
            }
        }
        Poll::Ready(Ok(()))
    }

    fn poll_write_message(
        mut self: &mut Self,
        message: &[u8],
        cx: &mut Context,
    ) -> Poll<Result<()>> {
        while self.pos < message.len() {
            let pos = self.pos;
            let n = ready!(Pin::new(&mut self.writer).poll_write(cx, &message[pos..]));
            let n = n?;
            self.pos += n;
        }
        let res = ready!(Pin::new(&mut self.writer).poll_flush(cx));
        self.reset();
        Poll::Ready(res)
    }

    fn reset(self: &mut Self) {
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

    fn encrypt(&mut self, mut buf: &mut [u8]) {
        if let Some(ref mut cipher) = &mut self.cipher {
            cipher.apply(&mut buf);
        }
    }
}
