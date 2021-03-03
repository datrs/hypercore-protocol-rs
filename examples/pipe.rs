use anyhow::Result;
use async_std::task;
use futures::prelude::*;
use futures::stream::StreamExt;
use log::*;
use pretty_bytes::converter::convert as pretty_bytes;
use sluice::pipe::pipe;
use std::env;
use std::time::Instant;

use hypercore_protocol::schema::*;
use hypercore_protocol::{Channel, Event, Message, Protocol, ProtocolBuilder};

fn main() {
    env_logger::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = Config::from_env();
    task::block_on(run_echo_pipes(config)).unwrap();
}

#[derive(Clone)]
struct Config {
    pub connections: u64,
    pub blocksize: u64,
    pub length: u64,
    pub no_encrypt: bool,
}

impl Config {
    pub fn total_bytes(&self) -> u64 {
        self.connections * self.blocksize * self.length * 2
    }

    pub fn from_env() -> Self {
        Config {
            connections: parse_env_u64("CONNECTIONS", 10),
            blocksize: parse_env_u64("BLOCKSIZE", 1000),
            length: parse_env_u64("LENGTH", 10000),
            no_encrypt: env::var("NO_ENCRYPT").is_ok(),
        }
    }
}

async fn run_echo_pipes(config: Config) -> Result<()> {
    let start = std::time::Instant::now();
    let mut futs = vec![];
    for i in 0..config.connections {
        futs.push(run_echo(config.clone(), i));
    }
    futures::future::join_all(futs).await;
    print_stats("total", start, config.total_bytes() as f64);
    Ok(())
}

async fn run_echo(config: Config, i: u64) -> Result<()> {
    // let cap: usize = config.blocksize as usize * 10;
    let (ar, bw) = pipe();
    let (br, aw) = pipe();

    let mut a = ProtocolBuilder::new(true);
    let mut b = ProtocolBuilder::new(false);
    if config.no_encrypt {
        a = a.set_encrypted(false);
        b = b.set_encrypted(false);
    }
    let a = a.connect_rw(ar, aw);
    let b = b.connect_rw(br, bw);
    let c = config.clone();
    let ta = task::spawn(async move { onconnection(c, i, a).await });
    let c = config.clone();
    let tb = task::spawn(async move { onconnection(c, i, b).await });
    let _lena = ta.await?;
    let _lenb = tb.await?;
    Ok(())
}

// The onconnection handler is called for each incoming connection (if server)
// or once when connected (if client).
async fn onconnection<R, W>(config: Config, i: u64, mut protocol: Protocol<R, W>) -> Result<u64>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let key = vec![0u8; 24];
    let is_initiator = protocol.is_initiator();
    // let mut len: u64 = 0;
    while let Some(event) = protocol.next().await {
        match event {
            Ok(event) => {
                debug!("[init {}] EVENT {:?}", is_initiator, event);
                match event {
                    Event::Handshake(_) => {
                        protocol.open(key.clone()).await?;
                    }
                    Event::DiscoveryKey(_dkey) => {}
                    Event::Channel(channel) => {
                        let config = config.clone();
                        task::spawn(async move {
                            if is_initiator {
                                on_channel_init(config, i, channel).await
                            } else {
                                on_channel_resp(config, i, channel).await
                            }
                        });
                    }
                    Event::Close(_) => {
                        return Ok(0);
                    }
                    Event::Error(e) => return Err(e.into()),
                }
            }
            Err(err) => {
                error!("ERROR {:?}", err);
                return Err(err.into());
            }
        }
    }
    Ok(0)
}

async fn on_channel_resp(_config: Config, _i: u64, mut channel: Channel) -> Result<u64> {
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

async fn on_channel_init(config: Config, i: u64, mut channel: Channel) -> Result<u64> {
    let data = vec![1u8; config.blocksize as usize];
    let mut len: u64 = 0;
    let message = msg_data(0, data);
    channel.send(message).await?;

    let start = std::time::Instant::now();

    while let Some(message) = channel.next().await {
        match message {
            Message::Data(mut data) => {
                len += data.value.as_ref().map_or(0, |v| v.len() as u64);
                debug!("[a] recv {}", data.index);
                if data.index >= config.length {
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

fn parse_env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .map(|v| v.parse().unwrap())
        .unwrap_or(default)
}
