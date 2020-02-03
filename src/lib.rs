use async_std::net::TcpStream;
use async_std::task::{Context, Poll};
use futures::io::{AsyncRead, AsyncWrite};
use std::io::Result;
use std::pin::Pin;
use std::sync::Arc;

pub mod handshake;
pub mod protocol;
pub mod schema {
    include!(concat!(env!("OUT_DIR"), "/hypercore.schema.rs"));
}

pub use protocol::{Protocol, Message, Handlers};
pub use schema::*;

pub async fn create_from_tcp_stream(stream: TcpStream, is_initiator: bool) -> Result<()> {
    let (reader, writer) = tcp_stream_to_reader_writer(stream);
    let (reader, writer, handshake) = handshake::handshake(reader, writer, is_initiator).await?;
    eprintln!("handshake complete! now init hypercore protocol");
    protocol::handle_connection(reader, writer, Some(handshake)).await?;
    Ok(())
}

pub fn tcp_stream_to_reader_writer(stream: TcpStream) -> (CloneableStream, CloneableStream) {
    let stream = CloneableStream(Arc::new(stream));
    let reader = stream.clone();
    let writer = stream.clone();
    (reader, writer)
}

#[derive(Clone)]
pub struct CloneableStream(Arc<TcpStream>);
impl AsyncRead for CloneableStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context, buf: &mut [u8]) -> Poll<Result<usize>> {
        Pin::new(&mut &*self.0).poll_read(cx, buf)
    }
}
impl AsyncWrite for CloneableStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<Result<usize>> {
        Pin::new(&mut &*self.0).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<()>> {
        Pin::new(&mut &*self.0).poll_flush(cx)
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<()>> {
        Pin::new(&mut &*self.0).poll_close(cx)
    }
}
