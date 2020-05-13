use anyhow::Result;
use async_std::task;
use futures::prelude::*;
use futures::stream::StreamExt;
use log::*;
use piper::pipe;
use pretty_bytes::converter::convert as pretty_bytes;
use std::time::Instant;

use hypercore_protocol::schema::*;
use hypercore_protocol::{Channel, Event, Message, Protocol, ProtocolBuilder};

fn main() {
    env_logger::from_env(env_logger::Env::default().default_filter_or("info")).init();
    task::block_on(async_main()).unwrap();
}

const COUNT: u64 = 40_000;
const SIZE: u64 = 1000;
const CONNS: u64 = 10;

async fn async_main() -> Result<()> {
    let start = std::time::Instant::now();
    let mut futs = vec![];
    for i in 0..CONNS {
        futs.push(run_echo(i));
    }
    futures::future::join_all(futs).await;
    let bytes = (COUNT * SIZE * CONNS * 2) as f64;
    print_stats("final", start, bytes);
    Ok(())
}

async fn run_echo(i: u64) -> Result<()> {
    let cap: usize = SIZE as usize * 10;
    let (ar, bw) = pipe(cap);
    let (br, aw) = pipe(cap);

    let a = ProtocolBuilder::new(true).build_from_io(ar, aw);
    let b = ProtocolBuilder::new(false).build_from_io(br, bw);
    let ta = task::spawn(async move { onconnection(i, a).await });
    let tb = task::spawn(async move { onconnection(i, b).await });
    let _lena = ta.await?;
    let _lenb = tb.await?;
    Ok(())
}

// The onconnection handler is called for each incoming connection (if server)
// or once when connected (if client).
async fn onconnection<R, W>(i: u64, mut protocol: Protocol<R, W>) -> Result<u64>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let key = vec![0u8; 24];
    let is_initiator = protocol.is_initiator();
    // let mut len: u64 = 0;
    loop {
        match protocol.loop_next().await {
            Ok(event) => {
                debug!("[init {}] EVENT {:?}", is_initiator, event);
                match event {
                    Event::Handshake(_) => {
                        protocol.open(key.clone()).await?;
                    }
                    Event::DiscoveryKey(_dkey) => {}
                    Event::Channel(channel) => {
                        task::spawn(async move {
                            if is_initiator {
                                on_channel_init(i, channel).await
                            } else {
                                on_channel_resp(i, channel).await
                            }
                        });
                    }
                    Event::Close(_) => {
                        return Ok(0);
                    }
                }
            }
            Err(err) => {
                error!("ERROR {:?}", err);
                return Err(err.into());
            }
        }
    }
}

async fn on_channel_resp(_i: u64, mut channel: Channel) -> Result<u64> {
    let mut len: u64 = 0;
    while let Some(message) = channel.next().await {
        match message {
            Message::Data(ref data) => {
                len += data.value.as_ref().map_or(0, |v| v.len() as u64);
                debug!("[b] echo {}", data.index);
                channel.send(message).await?;
            }
            Message::Close(_) => {
                break;
            }
            _ => {}
        }
    }
    debug!("[b] ch close");
    Ok(len)
}

async fn on_channel_init(i: u64, mut channel: Channel) -> Result<u64> {
    let data = vec![1u8; SIZE as usize];
    let mut len: u64 = 0;
    let message = msg_data(0, data);
    channel.send(message).await?;

    let start = std::time::Instant::now();

    while let Some(message) = channel.next().await {
        match message {
            Message::Data(mut data) => {
                len += data.value.as_ref().map_or(0, |v| v.len() as u64);
                debug!("[a] recv {}", data.index);
                if data.index >= COUNT {
                    debug!("close at {}", data.index);
                    channel
                        .send(Message::Close(Close {
                            discovery_key: None,
                        }))
                        .await?;
                    break;
                } else {
                    data.index += 1;
                    channel.send(Message::Data(data)).await?;
                }
            }
            _ => {}
        }
    }
    // let bytes = (COUNT * SIZE) as f64;
    print_stats(i, start, len as f64);
    Ok(len)
}

fn msg_data(index: u64, value: Vec<u8>) -> Message {
    Message::Data(Data {
        index,
        value: Some(value),
        nodes: vec![],
        signature: None,
    })
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
