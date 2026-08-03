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

use crate::message::{ChannelMessage, Message};

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

            // Batch queued messages, but never batch an `Open`/`Close` message together
            // with anything else. `Vec<ChannelMessage>`'s multi-message wire encoding
            // groups messages by channel number to avoid repeating it per message, but
            // `Open`/`Close` don't have an outer channel number at all (it's embedded in
            // their own payload, framed via a dedicated 2-byte prefix) — only the
            // single-message path encodes them correctly. So an `Open`/`Close` at the
            // front of the queue is sent alone; a batch otherwise stops right before one.
            let mut messages = vec![];
            while let Some(front) = self.write_queue.front() {
                let front_is_open_or_close =
                    matches!(front.message, Message::Open(_) | Message::Close(_));
                if front_is_open_or_close && !messages.is_empty() {
                    break;
                }
                messages.push(self.write_queue.pop_front().expect("front just checked"));
                if front_is_open_or_close {
                    break;
                }
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
                    return Poll::Pending;
                }
                Poll::Ready(Ok(())) => {}
            }
        }

        if pending {
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
        loop {
            match Pin::new(&mut self.stream).poll_next(cx) {
                Poll::Ready(Some(event)) => match event {
                    // Skip handshake payloads: loop so the next poll registers a waker.
                    hypercore_handshake::CipherEvent::HandshakePayload(_x) => {}
                    hypercore_handshake::CipherEvent::Message(msg) => {
                        return match <Vec<ChannelMessage>>::decode(&msg) {
                            Ok((messages, _rest)) => {
                                for m in messages.iter() {
                                    trace!("RX ChannelMessage::{m}");
                                }
                                Poll::Ready(Some(Ok(messages)))
                            }
                            Err(e) => Poll::Ready(Some(Err(e.into()))),
                        };
                    }
                    hypercore_handshake::CipherEvent::ErrStuff(e) => {
                        return Poll::Ready(Some(Err(e)));
                    }
                },
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
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
