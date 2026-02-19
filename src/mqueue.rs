//! Interface for reading and writing messages to a Stream/Sink
//!
//! This module handles encoding/decoding of `ChannelMessage` to/from raw bytes
//! over an already-encrypted connection (e.g., from hyperswarm).

use std::{
    collections::VecDeque,
    fmt::Debug,
    io::Result,
    pin::Pin,
    task::{Context, Poll},
};

use compact_encoding::CompactEncoding as _;
use futures::{Sink, Stream};
use hypercore_handshake::{CipherTrait, state_machine::PUBLIC_KEYLEN};
use tracing::{error, instrument, trace};

use crate::message::ChannelMessage;

/// Message IO layer that encodes/decodes `ChannelMessage` over a byte stream.
///
/// This expects the underlying stream to already be encrypted and framed
/// (e.g., a hyperswarm `Connection`).
pub(crate) struct MessageIo {
    stream: Box<dyn CipherTrait>,
    write_queue: VecDeque<ChannelMessage>,
}

impl Debug for MessageIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageIo")
            .field("write_queue", &self.write_queue)
            .finish()
    }
}

impl MessageIo {
    /// Create a new MessageIo from a stream.
    ///
    /// The stream should be an already-encrypted, message-framed connection
    /// (e.g., hyperswarm's `Connection` which implements `Stream<Item = CipherEvent>`
    /// where `CipherEvent::Message` contains the decrypted bytes).
    pub(crate) fn new(stream: Box<dyn CipherTrait>) -> Self {
        Self {
            stream,
            write_queue: Default::default(),
        }
    }
    pub(crate) fn remote_public_key(&self) -> Option<[u8; PUBLIC_KEYLEN]> {
        self.stream.remote_public_key()
    }
    pub(crate) fn local_public_key(&self) -> [u8; PUBLIC_KEYLEN] {
        self.stream.local_public_key()
    }
    pub(crate) fn handshake_hash(&self) -> Option<Vec<u8>> {
        self.stream.handshake_hash()
    }

    /// Enqueue an outgoing message
    pub(crate) fn enqueue(&mut self, msg: ChannelMessage) {
        self.write_queue.push_back(msg)
    }

    /// Drive outgoing messages
    #[instrument(skip_all)]
    pub(crate) fn poll_outbound(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let mut pending = true;

        while let Poll::Ready(Ok(())) = Sink::poll_ready(Pin::new(&mut self.stream), cx) {
            pending = false;
            if self.write_queue.is_empty() {
                break;
            }

            // Batch all queued messages
            let mut messages = vec![];
            while let Some(msg) = self.write_queue.pop_front() {
                messages.push(msg);
            }

            let buf = match messages.to_encoded_bytes() {
                Ok(x) => x,
                Err(e) => {
                    error!(error = ?e, "error encoding messages");
                    return Poll::Ready(Err(e.into()));
                }
            };

            if let Err(e) = Sink::start_send(Pin::new(&mut self.stream), buf.to_vec()) {
                return Poll::Ready(Err(e));
            }

            match Sink::poll_flush(Pin::new(&mut self.stream), cx) {
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Poll::Ready(Ok(())) => {}
            }
        }

        if pending {
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    /// Poll for incoming messages
    #[instrument(skip_all)]
    pub(crate) fn poll_inbound(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Vec<ChannelMessage>>>> {
        match Pin::new(&mut self.stream).poll_next(cx) {
            Poll::Ready(Some(event)) => match event {
                hypercore_handshake::CipherEvent::HandshakePayload(_x) => Poll::Pending,
                hypercore_handshake::CipherEvent::Message(msg) => {
                    match <Vec<ChannelMessage>>::decode(&msg) {
                        Ok((messages, _rest)) => {
                            for m in messages.iter() {
                                trace!("RX ChannelMessage::{m}");
                            }
                            Poll::Ready(Some(Ok(messages)))
                        }
                        Err(e) => Poll::Ready(Some(Err(e.into()))),
                    }
                }
                hypercore_handshake::CipherEvent::ErrStuff(e) => Poll::Ready(Some(Err(e))),
            },
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Stream for MessageIo {
    type Item = Result<Vec<ChannelMessage>>;

    #[instrument(skip_all)]
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Drive outbound messages
        let _ = self.poll_outbound(cx);
        // Poll for inbound messages
        self.poll_inbound(cx)
    }
}
