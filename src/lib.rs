mod channels;
mod constants;
mod encrypt;
mod handlers;
mod handshake;
mod message;
mod protocol;
mod util;

/// The wire messages used by the protocol.
pub mod schema {
    include!(concat!(env!("OUT_DIR"), "/hypercore.schema.rs"));
    pub use crate::message::ExtensionMessage;
}

pub use handlers::{ChannelContext, ChannelHandlers, DynProtocol, StreamContext, StreamHandlers};
pub use message::Message;
pub use protocol::Protocol;
pub use util::discovery_key;
