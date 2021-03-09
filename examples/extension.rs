use async_std::net::TcpStream;
use async_std::prelude::*;
use async_std::task::{self, JoinHandle};
use futures_lite::io::{AsyncRead, AsyncWrite};
use hypercore_protocol::{Channel, DiscoveryKey, Event, Protocol, ProtocolBuilder};
use log::*;
use pretty_bytes::converter::convert as pretty_bytes;
use std::io;
use std::time::Instant;

pub type MemoryProtocol = Protocol<sluice::pipe::PipeReader, sluice::pipe::PipeWriter>;
pub async fn create_pair_memory() -> std::io::Result<(MemoryProtocol, MemoryProtocol)> {
    let (ar, bw) = sluice::pipe::pipe();
    let (br, aw) = sluice::pipe::pipe();

    let a = ProtocolBuilder::new(true);
    let b = ProtocolBuilder::new(false);
    let a = a.connect_rw(ar, aw);
    let b = b.connect_rw(br, bw);
    Ok((a, b))
}

pub type TcpProtocol = Protocol<TcpStream, TcpStream>;
pub async fn create_pair_tcp(port: u16) -> std::io::Result<(TcpProtocol, TcpProtocol)> {
    let (stream_a, stream_b) = tcp::pair(port).await?;
    let encrypted = true;
    let a = ProtocolBuilder::new(true)
        .set_encrypted(encrypted)
        .connect(stream_a);
    let b = ProtocolBuilder::new(false)
        .set_encrypted(encrypted)
        .connect(stream_b);
    Ok((a, b))
}

pub fn next_event<R, W>(
    mut proto: Protocol<R, W>,
) -> impl Future<Output = (std::io::Result<Event>, Protocol<R, W>)>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin,
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
// Drive a stream to completion in a task.
fn drive<S>(mut proto: S) -> JoinHandle<()>
where
    S: Stream + Send + Unpin + 'static,
{
    task::spawn(async move { while let Some(_event) = proto.next().await {} })
}

// Drive a number of streams to completion.
// fn drive_all<S>(streams: Vec<S>) -> JoinHandle<()>
// where
//     S: Stream + Send + Unpin + 'static,
// {
//     let join_handles = streams.into_iter().map(drive);
//     task::spawn(async move {
//         for join_handle in join_handles {
//             join_handle.await;
//         }
//     })
// }

// Drive a protocol stream until the first channel arrives.
fn drive_until_channel<R, W>(
    mut proto: Protocol<R, W>,
) -> JoinHandle<io::Result<(Protocol<R, W>, Channel)>>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    task::spawn(async move {
        while let Some(event) = proto.next().await {
            let event = event?;
            match event {
                Event::Channel(channel) => return Ok((proto, channel)),
                _ => {}
            }
        }
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Protocol closed before a channel was opened",
        ))
    })
}

pub mod tcp {
    use async_std::net::{TcpListener, TcpStream};
    use async_std::prelude::*;
    use async_std::task;
    use std::io::{Error, ErrorKind, Result};
    pub async fn pair(port: u16) -> Result<(TcpStream, TcpStream)> {
        let address = format!("localhost:{}", port);
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

#[async_std::main]
pub async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let port = 9000u16;
    let n = 5;
    // let n = 1;
    let mut tasks = vec![];
    for n in 0..n {
        let port = port + n;
        let task = task::spawn(async move {
            let _ = channel_extension_async_read_write(port).await;
        });
        tasks.push(task);
    }
    for task in tasks.iter_mut() {
        task.await;
    }
    Ok(())
}

async fn channel_extension_async_read_write(port: u16) -> anyhow::Result<()> {
    // let (mut proto_a, mut proto_b) = create_pair_memory().await?;
    let (mut proto_a, mut proto_b) = create_pair_tcp(port).await?;
    let key = [1u8; 32];

    proto_a.open(key).await?;
    proto_b.open(key).await?;

    let next_a = drive_until_channel(proto_a);
    let next_b = drive_until_channel(proto_b);
    let (proto_a, mut channel_a) = next_a.await?;
    let (proto_b, mut channel_b) = next_b.await?;

    let mut ext_a = channel_a.register_extension("ext");
    let mut ext_b = channel_b.register_extension("ext");

    drive(proto_a);
    drive(proto_b);
    drive(channel_a);
    drive(channel_b);

    let instant = Instant::now();
    let limit = 1024 * 1024 * 64;
    // let limit = 1024 * 64 * 3;

    task::spawn(async move {
        let mut read_buf = vec![0u8; 1024 * 64];
        let mut len = 0;
        loop {
            let n = ext_b.read(&mut read_buf).await.unwrap();
            len += n;
            debug!("B READ: {}", len);
            ext_b.write_all(&read_buf[..n]).await.unwrap();
        }
    });

    let mut len = 0;
    task::spawn({
        let mut ext_a = ext_a.clone();
        let buf = vec![0u8; 1024 * 64];
        async move {
            while len < limit + 10 {
                ext_a.write_all(&buf).await.unwrap();
                len += buf.len();
                debug!("A WRITE: {}", len);
            }
        }
    });

    let mut read_buf = vec![0u8; 1024 * 64];
    let mut len = 0;
    while len < limit {
        let n = ext_a.read(&mut read_buf).await.unwrap();
        len += n;
        debug!("A READ {}", len);
    }

    print_stats("done", instant, limit as f64);
    // eprintln!("total len: {}", len);
    // eprintln!("total time: {}", len);

    Ok(())
}

fn print_stats(msg: impl ToString, instant: Instant, bytes: f64) {
    let msg = msg.to_string();
    let time = instant.elapsed();
    let secs = time.as_secs_f64();
    let bs = bytes / secs;
    eprintln!(
        "[{}] time {:?} bytes {} throughput {}/s",
        msg,
        time,
        pretty_bytes(bytes),
        pretty_bytes(bs)
    );
}
