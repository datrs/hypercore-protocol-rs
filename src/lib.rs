mod channels;
pub mod constants;
mod handlers;
pub mod handshake;
mod message;
pub mod protocol;
mod util;
pub mod schema {
    include!(concat!(env!("OUT_DIR"), "/hypercore.schema.rs"));
}

pub use channels::Channel;
pub use handlers::{ChannelContext, ChannelHandlers, StreamContext, StreamHandlers};
pub use message::{ExtensionMessage, Message};
pub use protocol::Protocol;
pub use schema::*;
pub use util::discovery_key;

// pub async fn create_from_tcp_stream(stream: TcpStream, is_initiator: bool) -> Result<()> {
//     let (reader, writer) = tcp_stream_to_reader_writer(stream);
//     let (reader, writer, handshake) = handshake::handshake(reader, writer, is_initiator).await?;
//     eprintln!("handshake complete! now init hypercore protocol");
//     protocol::handle_connection(reader, writer, Some(handshake)).await?;
//     Ok(())
// }

// pub fn tcp_stream_to_reader_writer(stream: TcpStream) -> (CloneableStream, CloneableStream) {
//     let reader = stream.clone();
//     let writer = stream.clone();
//     (reader, writer)
// }
