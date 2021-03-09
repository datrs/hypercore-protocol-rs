use crate::message::{EncodeError, Encoder, Frame};
use crate::noise::{Cipher, HandshakeResult};
use futures_lite::{ready, AsyncWrite};
use std::collections::VecDeque;
use std::fmt;
use std::io::Result;
use std::pin::Pin;
use std::task::{Context, Poll};

const BUF_SIZE: usize = 1024 * 64;

#[derive(Debug)]
pub struct ProtocolWriter<W>
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    writer: W,
    queue: VecDeque<Frame>,
    state: State,
}

impl<W> ProtocolWriter<W>
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            state: State::new(),
            queue: VecDeque::new(),
        }
    }

    pub fn queue_frame<F>(&mut self, frame: F)
    where
        F: Into<Frame>,
    {
        self.queue.push_back(frame.into())
    }

    pub fn can_park_frame(&self) -> bool {
        self.state.can_park_frame()
    }

    pub fn park_frame<F>(&mut self, frame: F)
    where
        F: Into<Frame>,
    {
        self.state.park_frame(frame)
    }

    pub fn try_queue_direct<T: Encoder>(
        &mut self,
        frame: &T,
    ) -> std::result::Result<bool, EncodeError> {
        self.state.try_queue_direct(frame)
    }

    pub fn poll_send(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let this = self.get_mut();
        let state = &mut this.state;
        let writer = &mut this.writer;
        let queue = &mut this.queue;
        state.poll_send(cx, writer, queue)
    }

    pub fn into_inner(self) -> W {
        self.writer
    }

    pub fn upgrade_with_handshake(&mut self, handshake: &HandshakeResult) -> Result<()> {
        self.state.upgrade_with_handshake(handshake)
    }
}

#[derive(Debug)]
pub enum Step {
    Flushing,
    Writing,
    Processing,
}

pub struct State {
    buf: Vec<u8>,
    current_frame: Option<Frame>,
    start: usize,
    end: usize,
    cipher: Option<Cipher>,
    step: Step,
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State")
            .field("step", &self.step)
            .field("buf (len)", &self.buf.len())
            .field("current_frame", &self.current_frame)
            .field("start", &self.start)
            .field("end", &self.end)
            .field("cipher", &self.cipher.is_some())
            .finish()
    }
}

impl State {
    fn new() -> Self {
        Self {
            buf: vec![0u8; BUF_SIZE],
            current_frame: None,
            start: 0,
            end: 0,
            cipher: None,
            step: Step::Processing,
        }
    }

    fn try_queue_direct<T: Encoder>(
        &mut self,
        frame: &T,
    ) -> std::result::Result<bool, EncodeError> {
        let len = frame.encoded_len();
        if self.buf.len() < len {
            self.buf.resize(len, 0u8);
        }
        if len > self.remaining() {
            return Ok(false);
        }
        let len = frame.encode(&mut self.buf[self.end..])?;
        self.advance(len);
        Ok(true)
    }

    fn can_park_frame(&self) -> bool {
        self.current_frame.is_none()
    }

    fn park_frame<F>(&mut self, frame: F)
    where
        F: Into<Frame>,
    {
        if self.current_frame.is_none() {
            self.current_frame = Some(frame.into())
        }
    }

    fn advance(&mut self, n: usize) {
        let end = self.end + n;
        if let Some(ref mut cipher) = self.cipher {
            cipher.apply(&mut self.buf[self.end..end]);
        }
        self.end = end;
    }

    fn upgrade_with_handshake(&mut self, handshake: &HandshakeResult) -> Result<()> {
        let cipher = Cipher::from_handshake_tx(handshake)?;
        self.cipher = Some(cipher);
        Ok(())
    }
    fn remaining(&self) -> usize {
        self.buf.len() - self.end
    }

    fn pending(&self) -> usize {
        self.end - self.start
    }

    fn poll_send<W>(
        &mut self,
        cx: &mut Context<'_>,
        mut writer: &mut W,
        queue: &mut VecDeque<Frame>,
    ) -> Poll<Result<()>>
    where
        W: AsyncWrite + Unpin,
    {
        loop {
            self.step = match self.step {
                Step::Processing => {
                    if self.current_frame.is_none() && !queue.is_empty() {
                        self.current_frame = queue.pop_front();
                    }

                    if let Some(frame) = self.current_frame.take() {
                        if !self.try_queue_direct(&frame)? {
                            self.current_frame = Some(frame);
                        }
                    }
                    if self.pending() == 0 {
                        return Poll::Ready(Ok(()));
                    }
                    Step::Writing
                }
                Step::Writing => {
                    let n = ready!(
                        Pin::new(&mut writer).poll_write(cx, &self.buf[self.start..self.end])
                    )?;
                    self.start += n;
                    if self.start == self.end {
                        self.start = 0;
                        self.end = 0;
                    }
                    Step::Flushing
                }
                Step::Flushing => {
                    ready!(Pin::new(&mut writer).poll_flush(cx))?;
                    Step::Processing
                }
            }
        }
    }
}
