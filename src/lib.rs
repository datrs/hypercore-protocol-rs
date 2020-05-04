//! Speak hypercore-protocol.
//!
//! Most basic example:
//! ```rust
//! # async_std::task::block_on(async {
//! #
//! use hypercore_protocol::{ProtocolBuilder, Event};
//! let stream = async_std::net::connect("localhost:8000").await?;
//!
//! let key = vec![0u8; 32];
//! let protocol = ProtocolBuilder::new(true)
//!     .build_from_stream(stream);
//!     .into_stream();
//!
//! while let Some(event) = protocol.next().await {
//!     let event = event?;
//!     eprintln!("received event {:?}", event);
//!     match event {
//!         Event::Handshake(_remote_key) => {
//!             protocol.open(key)
//!         },
//!         Event::DiscoveryKey(_discovery_key) => {
//!         },
//!         Event::Channel(channel) => {
//!             async_std::task::spawn(async move {
//!                 while let Some(message) = channel.next().await {
//!                     eprintln!("received message: {:?}", message)
//!                 }
//!             })
//!         }
//!     }
//! }
//!
//! #
//! # })
//! ```
//! See [examples/basic.rs](https://github.com/Frando/hypercore-protocol-rust-experiments/blob/master/examples/basic.rs) for an actual example of how to use a protocol stream.

// Otherwise some macro calls in the next_loop are prohibited.
#![recursion_limit = "256"]

mod channels;
mod constants;
mod encrypt;
mod handshake;
mod message;
mod protocol;
mod util;
mod wire_message;

/// The wire messages used by the protocol.
pub mod schema {
    include!(concat!(env!("OUT_DIR"), "/hypercore.schema.rs"));
    pub use crate::message::ExtensionMessage;
}

pub use message::Message;
pub use protocol::{Channel, Event, Protocol, ProtocolBuilder, ProtocolOptions, ProtocolStream};
pub use util::discovery_key;
