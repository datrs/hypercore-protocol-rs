use crate::Protocol;
use futures_lite::io::{AsyncRead, AsyncWrite};

/// Options for a Protocol instance.
#[derive(Debug)]
pub struct Options {
    pub is_initiator: bool,
    pub noise: bool,
    pub encrypted: bool,
}

/// Build a Protocol instance with options.
pub struct Builder(Options);

impl Builder {
    // Create a protocol builder.
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
    pub fn connect<S>(self, stream: S) -> Protocol<S, S>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + Clone + 'static,
    {
        Protocol::new(stream.clone(), stream, self.0)
    }

    /// Create the protocol from an AsyncRead reader and AsyncWrite writer.
    pub fn connect_rw<R, W>(self, reader: R, writer: W) -> Protocol<R, W>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Protocol::new(reader, writer, self.0)
    }

    #[deprecated(since = "0.0.1", note = "Use connect_rw")]
    pub fn build_from_io<R, W>(self, reader: R, writer: W) -> Protocol<R, W>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        self.connect_rw(reader, writer)
    }

    #[deprecated(since = "0.0.1", note = "Use connect")]
    pub fn build_from_stream<S>(self, stream: S) -> Protocol<S, S>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + Clone + 'static,
    {
        self.connect(stream)
    }
}
