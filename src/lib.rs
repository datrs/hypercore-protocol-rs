//! ## Introduction
//!
//! Hypercore protocol is a streaming, message based protocol. This is a rust port of the wire
//! protocol implementation in [the original Javascript version][holepunch-hypercore] aiming
//! for interoperability with LTS version.
//!
//! This crate is built on top of the [hypercore] crate, which defines some structs used here.
//!
//! ## Design
//!
//! This crate does not include any IO related code, it is up to the user to supply a streaming IO
//! handler that implements the [AsyncRead] and [AsyncWrite] traits.
//!
//! When opening a Hypercore protocol stream on an IO handler, the protocol will perform a Noise
//! handshake followed by libsodium's [crypto_secretstream] to setup a secure and authenticated
//! connection. After that, each side can request any number of channels on the protocol. A
//! channel is opened with a [Key], a 32 byte buffer. Channels are only opened if both peers
//! opened a channel for the same key. It is automatically verified that both parties know the
//! key without transmitting the key itself.
//!
//! On a channel, the predefined messages, including a custom Extension message, of the Hypercore
//! protocol can be sent and received.
//!
//! ## Example
//!
//! The following example opens a TCP server on localhost and connects to that server. Both ends
//! then open a channel with the same key and exchange a message.
//!
//! ```no_run
//! # async_std::task::block_on(async {
//! use hypercore_protocol::{ProtocolBuilder, Event, Message};
//! use hypercore_protocol::schema::*;
//! use async_std::prelude::*;
//! // Start a tcp server.
//! let listener = async_std::net::TcpListener::bind("localhost:8000").await.unwrap();
//! async_std::task::spawn(async move {
//!     let mut incoming = listener.incoming();
//!     while let Some(Ok(stream)) = incoming.next().await {
//!         async_std::task::spawn(async move {
//!             onconnection(stream, false).await
//!         });
//!     }
//! });
//!
//! // Connect a client.
//! let stream = async_std::net::TcpStream::connect("localhost:8000").await.unwrap();
//! onconnection(stream, true).await;
//!
//! /// Start Hypercore protocol on a TcpStream.
//! async fn onconnection (stream: async_std::net::TcpStream, is_initiator: bool) {
//!     // A peer either is the initiator or a connection or is being connected to.
//!     let name = if is_initiator { "dialer" } else { "listener" };
//!     // A key for the channel we want to open. Usually, this is a pre-shared key that both peers
//!     // know about.
//!     let key = [3u8; 32];
//!     // Create the protocol.
//!     let mut protocol = ProtocolBuilder::new(is_initiator).connect(stream);
//!
//!     // Iterate over the protocol events. This is required to "drive" the protocol.
//!
//!     while let Some(Ok(event)) = protocol.next().await {
//!         eprintln!("{} received event {:?}", name, event);
//!         match event {
//!             // The handshake event is emitted after the protocol is fully established.
//!             Event::Handshake(_remote_key) => {
//!                 protocol.open(key.clone()).await;
//!             },
//!             // A Channel event is emitted for each established channel.
//!             Event::Channel(mut channel) => {
//!                 // A Channel can be sent to other tasks.
//!                 async_std::task::spawn(async move {
//!                     // A Channel can both send messages and is a stream of incoming messages.
//!                     channel.send(Message::Want(Want { start: 0, length: 1 })).await;
//!                     while let Some(message) = channel.next().await {
//!                         eprintln!("{} received message: {:?}", name, message);
//!                     }
//!                 });
//!             },
//!             _ => {}
//!         }
//!     }
//! }
//! # })
//! ```
//!
//! Find more examples in the [Github repository][examples].
//!
//! [holepunch-hypercore]: https://github.com/holepunchto/hypercore
//! [datrs-hypercore]: https://github.com/datrs/hypercore
//! [AsyncRead]: futures_lite::AsyncRead
//! [AsyncWrite]: futures_lite::AsyncWrite
//! [examples]: https://github.com/datrs/hypercore-protocol-rs#examples

#![forbid(unsafe_code, future_incompatible, rust_2018_idioms)]
#![deny(missing_debug_implementations, nonstandard_style)]
#![warn(missing_docs, unreachable_pub)]

mod builder;
mod channels;
mod constants;
mod crypto;
mod duplex;
mod message;
mod protocol;
mod reader;
mod util;
mod writer;

/// The wire messages used by the protocol.
pub mod schema;

pub use builder::Builder as ProtocolBuilder;
pub use channels::Channel;
// Export the needed types for Channel::take_receiver, and Channel::local_sender()
pub use async_channel::{
    Receiver as ChannelReceiver, SendError as ChannelSendError, Sender as ChannelSender,
};
pub use duplex::Duplex;
pub use hypercore; // Re-export hypercore
pub use message::Message;
pub use protocol::{DiscoveryKey, Event, Key, Protocol};
pub use util::discovery_key;
