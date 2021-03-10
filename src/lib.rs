//! Speak hypercore-protocol.
//!
//! Most basic example:
//! ```no_run
//! # async_std::task::block_on(async {
//! #
//! use hypercore_protocol::{ProtocolBuilder, Event, Message};
//! use hypercore_protocol::schema::*;
//! use async_std::prelude::*;
//!
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
//!     let key = [3u8; 32];
//!     let mut protocol = ProtocolBuilder::new(is_initiator).connect(stream);
//!
//!     while let Some(Ok(event)) = protocol.next().await {
//!         eprintln!("[{}] received event {:?}", is_initiator, event);
//!         match event {
//!             Event::Handshake(_remote_key) => {
//!                 protocol.open(key.clone()).await;
//!             },
//!             Event::Channel(mut channel) => {
//!                 async_std::task::spawn(async move {
//!                     channel.want(Want { start: 0, length: None }).await;
//!                     while let Some(message) = channel.next().await {
//!                         eprintln!("[{}] received message: {:?}", is_initiator, message);
//!                     }
//!                 });
//!             },
//!             _ => {}
//!         }
//!     }
//! }
//!
//! #
//! # })
//! ```
//! See [examples/basic.rs](https://github.com/Frando/hypercore-protocol-rust-experiments/blob/master/examples/basic.rs) for an actual example of how to use a protocol stream.

#![forbid(unsafe_code, future_incompatible, rust_2018_idioms)]
#![deny(missing_debug_implementations, nonstandard_style)]
// #![warn(missing_docs, missing_doc_code_examples, unreachable_pub)]
#![warn(missing_docs, missing_doc_code_examples)]

mod builder;
mod channels;
mod constants;
mod duplex;
mod extension;
mod message;
mod noise;
mod protocol;
mod reader;
mod util;
mod writer;

/// The wire messages used by the protocol.
#[allow(missing_docs)]
pub mod schema {
    include!(concat!(env!("OUT_DIR"), "/hypercore.schema.rs"));
    pub use crate::message::ExtensionMessage;
}

pub use builder::{Builder as ProtocolBuilder, Options};
pub use channels::Channel;
pub use duplex::Duplex;
pub use extension::Extension;
pub use message::Message;
pub use protocol::{DiscoveryKey, Event, Key, Protocol};
pub use util::discovery_key;
