cfg_if::cfg_if! {
    if #[cfg(feature = "v9")] {
use async_std::net::TcpStream;
use async_std::prelude::*;
use async_std::task::{self, JoinHandle};
use futures_lite::io::{AsyncRead, AsyncWrite};
use hypercore_protocol::{Channel, Event, Protocol, ProtocolBuilder};
use log::*;
use pretty_bytes::converter::convert as pretty_bytes;
use std::io;
use std::time::Instant;

// This example sets up a pair of protocols connected over TCP,
// registers the same extension on each, and then uses the AsyncWrite
// and AsyncRead implementations on the extension to pipe binary data
// from A to B, where B will echo it back, and then A will read the
// echoed stream. Then, the throughput is calculated and reported.
//
// Note: Run in release mode, otherwise it will be *very* slow.
//
// Adjust the consts below for different "settings".

const BYTES: usize = 1024 * 1024 * 64;
const PARALLEL: usize = 5;
const ENCRYPT: bool = false;

#[async_std::main]
pub async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let port = 9000u16;
    let mut tasks = vec![];
    let instant = Instant::now();
    for n in 0..PARALLEL {
        let port = port + n as u16;
        let task = task::spawn(async move {
            let _ = channel_extension_async_read_write(BYTES, port, ENCRYPT).await;
        });
        tasks.push(task);
    }
    for task in tasks.iter_mut() {
        task.await;
    }
    print_stats("total", instant, (BYTES * PARALLEL) as f64);
    Ok(())
}

async fn channel_extension_async_read_write(
    limit: usize,
    port: u16,
    encrypted: bool,
) -> anyhow::Result<()> {
    // let (mut proto_a, mut proto_b) = create_pair_memory().await?;
    let (mut proto_a, mut proto_b) = create_pair_tcp(port, encrypted).await?;
    let key = [1u8; 32];
    proto_a.open(key).await?;
    proto_b.open(key).await?;

    let next_a = drive_until_channel(proto_a);
    let next_b = drive_until_channel(proto_b);
    let (proto_a, mut channel_a) = next_a.await?;
    let (proto_b, mut channel_b) = next_b.await?;

    let mut ext_a = channel_a.register_extension("ext").await;
    let mut ext_b = channel_b.register_extension("ext").await;

    // Drive the protocols and channels.
    drive(proto_a);
    drive(proto_b);
    drive(channel_a);
    drive(channel_b);

    let instant = Instant::now();

    // On B, run an echo loop.
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

    // On A, write BYTES bytes.
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

    // On A, read BYTES bytes back (from the echo on B).
    let mut read_buf = vec![0u8; 1024 * 64];
    let mut len = 0;
    while len < limit {
        let n = ext_a.read(&mut read_buf).await.unwrap();
        len += n;
        debug!("A READ {}", len);
    }

    // Now report how long it all took.
    print_stats("done", instant, limit as f64);
    Ok(())
}

fn print_stats(msg: impl ToString, instant: Instant, bytes: f64) {
    let msg = msg.to_string();
    let time = instant.elapsed();
    let secs = time.as_secs_f64();
    let bs = bytes / secs;
    eprintln!(
        "[{}] time {:.3?} bytes {} throughput {}/s",
        msg,
        time,
        pretty_bytes(bytes),
        pretty_bytes(bs)
    );
}

pub type TcpProtocol = Protocol<TcpStream>;
pub async fn create_pair_tcp(
    port: u16,
    encrypted: bool,
) -> std::io::Result<(TcpProtocol, TcpProtocol)> {
    let (stream_a, stream_b) = tcp::pair(port).await?;
    let a = ProtocolBuilder::new(true)
        .set_encrypted(encrypted)
        .connect(stream_a);
    let b = ProtocolBuilder::new(false)
        .set_encrypted(encrypted)
        .connect(stream_b);
    Ok((a, b))
}

/// Drive a stream to completion in a task.
fn drive<S>(mut proto: S) -> JoinHandle<()>
where
    S: Stream + Send + Unpin + 'static,
{
    task::spawn(async move { while let Some(_event) = proto.next().await {} })
}

// Drive a protocol stream until the first channel arrives.
fn drive_until_channel<IO>(
    mut proto: Protocol<IO>,
) -> JoinHandle<io::Result<(Protocol<IO>, Channel)>>
where
    IO: AsyncRead + AsyncWrite + Send + Unpin + 'static,
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
} else {
        fn main() {}
    }
}
