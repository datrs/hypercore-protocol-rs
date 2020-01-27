use async_std::net::TcpStream;
use async_std::task::{Context, Poll};
use futures::io::{AsyncRead, AsyncWrite};
use std::io::Result;
use std::pin::Pin;
use std::sync::Arc;

pub mod noise;
pub mod protocol;
pub mod schema {
    include!(concat!(env!("OUT_DIR"), "/hypercore.schema.rs"));
}

pub async fn create_from_tcp_stream(stream: TcpStream, is_initiator: bool) -> Result<()> {
    let stream = CloneableStream(Arc::new(stream));
    protocol::handle_connection(stream.clone(), stream.clone(), is_initiator).await?;
    Ok(())
}

#[derive(Clone)]
pub(crate) struct CloneableStream(Arc<TcpStream>);
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
