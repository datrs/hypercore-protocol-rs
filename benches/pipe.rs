#[path = "../src/test_utils.rs"]
mod test_utils;
use async_std::task;
#[path = "../tests/_util.rs"]
mod _util;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use futures::stream::StreamExt;
use hypercore_protocol::{Channel, Event, Message, Protocol, schema::*};
use hypercore_schema::DataBlock;
use pretty_bytes::converter::convert as pretty_bytes;
use std::{io::Result, time::Instant};
use tracing::{debug, error};

use crate::_util::create_pair;

const COUNT: u64 = 1000;
const SIZE: u64 = 100;
const CONNS: u64 = 10;

fn bench_throughput(c: &mut Criterion) {
    test_utils::log();
    let mut group = c.benchmark_group("pipe");
    group.sample_size(10);
    group.throughput(Throughput::Bytes(SIZE * COUNT * CONNS));
    group.bench_function("pipe_echo", |b| {
        b.iter(|| {
            task::block_on(async move {
                let mut futs = vec![];
                for i in 0..CONNS {
                    futs.push(run_echo(i));
                }
                futures::future::join_all(futs).await;
            })
        });
    });

    group.finish();
}

criterion_group!(benches, bench_throughput);
criterion_main!(benches);

async fn run_echo(i: u64) -> Result<()> {
    let (a, b) = create_pair();
    let ta = task::spawn(async move { onconnection(i, a).await });
    let tb = task::spawn(async move { onconnection(i, b).await });
    ta.await?;
    tb.await?;
    Ok(())
}

// The onconnection handler is called for each incoming connection (if server)
// or once when connected (if client).
async fn onconnection(i: u64, mut protocol: Protocol) -> Result<u64> {
    let key = [0u8; 32];
    let is_initiator = protocol.is_initiator();
    // let mut len: u64 = 0;
    loop {
        match protocol.next().await {
            Some(Ok(event)) => {
                debug!("[{}] EVENT {:?}", is_initiator, event);
                match event {
                    Event::Handshake(_) => {
                        protocol.open(key).await?;
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
                    _ => {}
                }
            }
            Some(Err(err)) => {
                error!("ERROR {:?}", err);
                return Err(err);
            }
            None => return Ok(0),
        }
    }
}

async fn on_channel_resp(_i: u64, mut channel: Channel) -> Result<u64> {
    let mut len: u64 = 0;
    while let Some(message) = channel.next().await {
        match message {
            Message::Data(ref data) => {
                len += value_len(data);
                debug!("[b] echo {}", index(data));
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
        if let Message::Data(mut data) = message {
            len += value_len(&data);
            debug!("[a] recv {}", index(&data));
            if index(&data) >= COUNT {
                debug!("close at {}", index(&data));
                channel.close().await?;
                break;
            } else {
                increment_index(&mut data);
                channel.send(Message::Data(data)).await?;
            }
        }
    }
    // let bytes = (COUNT * SIZE) as f64;
    print_stats(i, start, len as f64);
    Ok(len)
}

fn msg_data(index: u64, value: Vec<u8>) -> Message {
    Message::Data(Data {
        request: index,
        fork: 0,
        block: Some(DataBlock {
            index,
            value,
            nodes: vec![],
        }),
        hash: None,
        seek: None,
        upgrade: None,
    })
}

fn index(data: &Data) -> u64 {
    data.request
}

fn increment_index(data: &mut Data) {
    data.request += 1;
}

fn value_len(data: &Data) -> u64 {
    data.block.as_ref().map_or(0, |b| b.value.len() as u64)
}

fn print_stats(msg: impl ToString, instant: Instant, bytes: f64) {
    let msg = msg.to_string();
    let time = instant.elapsed();
    let secs = time.as_secs_f64();
    let bs = bytes / secs;
    debug!(
        "[{}] time {:?} bytes {} throughput {}/s",
        msg,
        time,
        pretty_bytes(bytes),
        pretty_bytes(bs)
    );
}
