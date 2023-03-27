use crate::Protocol;
use crate::{duplex::Duplex, protocol::Options};
use futures_lite::io::{AsyncRead, AsyncWrite};

/// Build a Protocol instance with options.
#[derive(Debug)]
pub struct Builder(Options);

impl Builder {
    /// Create a protocol builder as initiator (true) or responder (false).
    pub fn new(initiator: bool) -> Self {
        Self(Options::new(initiator))
    }

    /// Set encrypted option. Defaults to true.
    pub fn encrypted(mut self, encrypted: bool) -> Self {
        self.0.encrypted = encrypted;
        self
    }

    /// Set handshake option. Defaults to true.
    pub fn handshake(mut self, handshake: bool) -> Self {
        self.0.noise = handshake;
        self
    }

    /// Create the protocol from a stream that implements AsyncRead + AsyncWrite + Clone.
    pub fn connect<IO>(self, io: IO) -> Protocol<IO>
    where
        IO: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        Protocol::new(io, self.0)
    }

    /// Create the protocol from an AsyncRead reader and AsyncWrite writer.
    pub fn connect_rw<R, W>(self, reader: R, writer: W) -> Protocol<Duplex<R, W>>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let io = Duplex::new(reader, writer);
        Protocol::new(io, self.0)
    }
}
