//! ## Introduction
//!
//! Hypercore protocol is a streaming, message based protocol. This is a rust port of the wire
//! protocol implementation in [the original Javascript version][holepunch-hypercore] aiming
//! for interoperability with LTS version.
//!
//! This crate is built on top of the [hypercore](https://crates.io/crates/hypercore) crate, which defines some structs used here.
//!
//! ## Design
//!
//! This crate expects to receive a pre-encrypted, message-framed connection (e.g., from hyperswarm).
//! The underlying stream should implement `Stream<Item = Vec<u8>> + Sink<Vec<u8>>`.
//!
//! After construction, each side can request any number of channels on the protocol. A
//! channel is opened with a [Key], a 32 byte buffer. Channels are only opened if both peers
//! opened a channel for the same key. It is automatically verified that both parties know the
//! key without transmitting the key itself using the handshake hash from the underlying connection.
//!
//! On a channel, the predefined messages, including a custom Extension message, of the Hypercore
//! protocol can be sent and received.
//!
//! [holepunch-hypercore]: https://github.com/holepunchto/hypercore

#![forbid(unsafe_code)]
#![deny(missing_debug_implementations, nonstandard_style)]
#![warn(missing_docs, clippy::needless_pass_by_ref_mut, unreachable_pub)]

mod channels;
mod constants;
mod crypto;
mod error;
mod message;
mod mqueue;
mod protocol;
mod stream;
#[cfg(test)]
mod test_utils;
mod util;

/// The wire messages used by the protocol.
pub mod schema;
pub use error::Error;

pub use channels::Channel;
// Export the needed types for Channel::take_receiver, and Channel::local_sender()
pub use async_channel::{
    Receiver as ChannelReceiver, SendError as ChannelSendError, Sender as ChannelSender,
};
pub use message::Message;
pub use protocol::{Command, CommandTx, DiscoveryKey, Event, Key, Protocol};
pub use stream::BoxedStream;
pub use util::discovery_key;
// Export handshake result for constructing Protocol
pub use crypto::handshake::HandshakeResult;
