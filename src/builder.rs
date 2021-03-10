use crate::duplex::Duplex;
use crate::Protocol;
use futures_lite::io::{AsyncRead, AsyncWrite};

/// Options for a Protocol instance.
#[derive(Debug)]
pub struct Options {
    /// Whether this peer initiated the IO connection for this protoccol
    pub is_initiator: bool,
    /// Enable or disable the handshake.
    /// Disabling the handshake will also disable capabilitity verification.
    /// Don't disable this if you're not 100% sure you want this.
    pub noise: bool,
    /// Enable or disable transport encryption.
    pub encrypted: bool,
}

/// Build a Protocol instance with options.
#[derive(Debug)]
pub struct Builder(Options);

impl Builder {
    /// Create a protocol builder.
    pub fn new(is_initiator: bool) -> Self {
        Self(Options {
            is_initiator,
            noise: true,
            encrypted: true,
        })
    }

    /// Default options for an initiating endpoint.
    pub fn initiator() -> Self {
        Self::new(true)
    }

    /// Default options for a responding endpoint.
    pub fn responder() -> Self {
        Self::new(false)
    }

    /// Set encrypted option.
    pub fn set_encrypted(mut self, encrypted: bool) -> Self {
        self.0.encrypted = encrypted;
        self
    }

    /// Set handshake option.
    pub fn set_noise(mut self, noise: bool) -> Self {
        self.0.noise = noise;
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
