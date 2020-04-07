//! Speak hypercore-protocol.
//!
//! Most basic example:
//! ```rust
//! # async_std::task::block_on(async {
//! #
//! use hypercore_protocol::{ProtocolBuilder, StreamHandlers};
//! let stream = async_std::net::connect("localhost:8000").await?;
//!
//! // let handlers = ...
//!
//! let protocol = ProtocolBuilder::new(true)
//!     .set_handlers(handlers)
//!     .from_stream(stream);
//!
//! protocol.listen.await();
//!
//! #
//! # })
//! ```
//! The `handlers` variable refers to an application specific struct that implements
//! the [StreamHandlers](StreamHandlers) to optionally open channels if the remote announces
//! discovery keys. When opening a channel, pass a struct implementing [ChannelHandlers] to react
//! to the actual messages. All channel handling is done by the [Protocol](Protocol), including the
//! Noise handshake and the capability verification.
//!
//! See [examples/basic.rs](https://github.com/Frando/hypercore-protocol-rust-experiments/blob/master/examples/basic.rs) for an actual example of how to use a protocol stream.

mod channels;
mod constants;
mod encrypt;
mod handlers;
mod handshake;
mod message;
mod prefixed;
mod protocol;
mod util;
mod wire_message;

/// The wire messages used by the protocol.
pub mod schema {
    include!(concat!(env!("OUT_DIR"), "/hypercore.schema.rs"));
    pub use crate::message::ExtensionMessage;
}

pub use handlers::{Channel, ChannelHandler, DynProtocol, StreamContext, StreamHandler};
pub use handshake::{handshake, HandshakeResult};
pub use message::Message;
pub use protocol::{Protocol, ProtocolBuilder, ProtocolOptions};
pub use util::discovery_key;
