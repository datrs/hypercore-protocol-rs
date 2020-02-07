//! Speak hypercore-protocol.
//!
//! Most basic example:
//! ```
//! # async_std::task::block_on(async {
//! #
//! use simple_hypercore_protocol::{ProtocolBuilder, StreamHandlers};
//! let stream = async_std::net::connect("localhost:8000").await?;
//!
//! struct FeedStore {}
//! #[async_trait]
//! impl StreamHandlers for FeedStore {
//!     async fn on_discoverykey(proto: &mut StreamContext, discovey_key: &[u8]) -> Result<()> {
//!         // If you have the key for this discovery_key, open a channel with
//!         // proto.open(key, channel_handlers).await?.
//!     }
//! }
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

/// The wire messages used by the protocol.
pub mod schema {
    include!(concat!(env!("OUT_DIR"), "/hypercore.schema.rs"));
    pub use crate::message::ExtensionMessage;
}

pub use handlers::{ChannelContext, ChannelHandlers, DynProtocol, StreamContext, StreamHandlers};
pub use handshake::{handshake, HandshakeResult};
pub use message::Message;
pub use protocol::{Protocol, ProtocolBuilder, ProtocolOptions};
pub use util::discovery_key;
