use async_std::net::TcpStream;
use async_std::prelude::*;
use async_std::task;
use futures_lite::io::{AsyncRead, AsyncWrite};
use hypercore_protocol::{Channel, DiscoveryKey, Duplex, Event, Protocol, ProtocolBuilder};

pub type MemoryProtocol = Protocol<Duplex<sluice::pipe::PipeReader, sluice::pipe::PipeWriter>>;
pub async fn create_pair_memory() -> std::io::Result<(MemoryProtocol, MemoryProtocol)> {
    let (ar, bw) = sluice::pipe::pipe();
    let (br, aw) = sluice::pipe::pipe();

    let a = ProtocolBuilder::new(true);
    let b = ProtocolBuilder::new(false);
    let a = a.connect_rw(ar, aw);
    let b = b.connect_rw(br, bw);
    Ok((a, b))
}

pub type TcpProtocol = Protocol<TcpStream>;
pub async fn create_pair_tcp() -> std::io::Result<(TcpProtocol, TcpProtocol)> {
    let (stream_a, stream_b) = tcp::pair().await?;
    let a = ProtocolBuilder::new(true).connect(stream_a);
    let b = ProtocolBuilder::new(false).connect(stream_b);
    Ok((a, b))
}

pub fn next_event<IO>(
    mut proto: Protocol<IO>,
) -> impl Future<Output = (std::io::Result<Event>, Protocol<IO>)>
where
    IO: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let task = task::spawn(async move {
        let e1 = proto.next().await;
        let e1 = e1.unwrap();
        (e1, proto)
    });
    task
}

pub fn event_discovery_key(event: Event) -> DiscoveryKey {
    if let Event::DiscoveryKey(dkey) = event {
        dkey
    } else {
        panic!("Expected discovery key event");
    }
}

pub fn event_channel(event: Event) -> Channel {
    if let Event::Channel(channel) = event {
        channel
    } else {
        panic!("Expected channel event");
    }
}

pub mod tcp {
    use async_std::net::{TcpListener, TcpStream};
    use async_std::prelude::*;
    use async_std::task;
    use std::io::{Error, ErrorKind, Result};
    pub async fn pair() -> Result<(TcpStream, TcpStream)> {
        let address = "localhost:9999";
        let listener = TcpListener::bind(&address).await?;
        let mut incoming = listener.incoming();

        let connect_task = task::spawn(async move { TcpStream::connect(&address).await });

        let server_stream = incoming.next().await;
        let server_stream =
            server_stream.ok_or_else(|| Error::new(ErrorKind::Other, "Stream closed"))?;
        let server_stream = server_stream?;
        let client_stream = connect_task.await?;
        Ok((server_stream, client_stream))
    }
}
