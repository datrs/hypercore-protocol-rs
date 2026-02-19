//! Type-erased bidirectional byte stream for the protocol.
//!
//! This module provides [`BoxedStream`], which hides the concrete stream type
//! from the public API while still supporting any `Stream<Item = Vec<u8>> + Sink<Vec<u8>>`.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{Sink, Stream};

/// A type-erased bidirectional byte stream for protocol communication.
///
/// This wrapper allows [`Protocol`](crate::Protocol) to have a non-generic public interface
/// while still accepting any stream type that implements the required traits.
pub struct BoxedStream {
    inner: Box<dyn StreamSink + Send>,
}

impl std::fmt::Debug for BoxedStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxedStream").finish_non_exhaustive()
    }
}

/// Internal trait combining Stream + Sink operations for type erasure.
trait StreamSink: Send + Sync {
    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<Vec<u8>>>;
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
    fn start_send(&mut self, item: Vec<u8>) -> io::Result<()>;
    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
}

/// Wrapper to implement StreamSink for any compatible type.
struct StreamSinkWrapper<S>(S);

impl<S> StreamSink for StreamSinkWrapper<S>
where
    S: Stream<Item = Vec<u8>> + Sink<Vec<u8>> + Unpin + Send + Sync,
    <S as Sink<Vec<u8>>>::Error: Into<io::Error>,
{
    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<Vec<u8>>> {
        Pin::new(&mut self.0).poll_next(cx)
    }

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_ready(cx).map_err(Into::into)
    }

    fn start_send(&mut self, item: Vec<u8>) -> io::Result<()> {
        Pin::new(&mut self.0).start_send(item).map_err(Into::into)
    }

    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx).map_err(Into::into)
    }

    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_close(cx).map_err(Into::into)
    }
}

impl BoxedStream {
    /// Create a new `BoxedStream` from any compatible stream.
    ///
    /// The stream must implement:
    /// - `Stream<Item = Vec<u8>>` for receiving messages
    /// - `Sink<Vec<u8>>` for sending messages
    /// - `Unpin` and `Send`
    pub fn new<S>(stream: S) -> Self
    where
        S: Stream<Item = Vec<u8>> + Sink<Vec<u8>> + Unpin + Send + Sync + 'static,
        <S as Sink<Vec<u8>>>::Error: Into<io::Error>,
    {
        BoxedStream {
            inner: Box::new(StreamSinkWrapper(stream)),
        }
    }
}

impl Stream for BoxedStream {
    type Item = Vec<u8>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_next(cx)
    }
}

impl Sink<Vec<u8>> for BoxedStream {
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Vec<u8>) -> Result<(), Self::Error> {
        self.inner.start_send(item)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_close(cx)
    }
}
