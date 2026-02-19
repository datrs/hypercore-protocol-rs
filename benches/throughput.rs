#[path = "../src/test_utils.rs"]
mod test_utils;
use async_std::{
    net::{TcpListener, TcpStream},
    task,
};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use futures::{
    future::Either,
    stream::{FuturesUnordered, StreamExt},
};
use hypercore_handshake::{
    Cipher,
    state_machine::{SecStream, hc_specific::generate_keypair},
};
use hypercore_protocol::{Channel, Event, Message, Protocol, schema::*};
use hypercore_schema::DataBlock;
use std::time::Instant;
use tracing::{debug, info, trace};
use uint24le_framing::Uint24LELengthPrefixedFraming;

const PORT: usize = 11011;
const SIZE: u64 = 1000;
const COUNT: u64 = 200;
const CLIENTS: usize = 1;

fn bench_throughput(c: &mut Criterion) {
    test_utils::log();
    let address = format!("localhost:{PORT}");

    let mut group = c.benchmark_group("throughput");
    let data = vec![1u8; SIZE as usize];

    // let _server = task::block_on(start_server(&address));

    group.sample_size(10);
    group.throughput(Throughput::Bytes(
        data.len() as u64 * COUNT * CLIENTS as u64,
    ));
    group.bench_function("echo", |b| {
        b.iter_with_setup(
            || {
                let (server, streams) = task::block_on(async {
                    let server = start_server(&address).await;
                    let mut streams = vec![];
                    for _i in 0..CLIENTS {
                        streams.push(TcpStream::connect(&address).await.unwrap())
                    }
                    (server, streams)
                });
                (server, streams)
            },
            |(server, streams)| {
                task::block_on(async move {
                    let mut futures: FuturesUnordered<_> = streams
                        .into_iter()
                        .map(|s| async move {
                            onconnection(s, true).await;
                        })
                        .collect();
                    while let Some(_res) = futures.next().await {}
                    server.send(())
                })
            },
        );
    });

    group.finish();
}

criterion_group!(server_benches, bench_throughput);
criterion_main!(server_benches);

async fn start_server(address: &str) -> futures::channel::oneshot::Sender<()> {
    let listener = TcpListener::bind(&address).await.unwrap();
    info!("listening on {}", listener.local_addr().unwrap());
    let (kill_tx, mut kill_rx) = futures::channel::oneshot::channel();
    task::spawn(async move {
        let mut incoming = listener.incoming();
        // let kill_rx = &mut kill_rx;
        loop {
            match futures::future::select(incoming.next(), &mut kill_rx).await {
                Either::Left((next, _)) => {
                    if let Some(Ok(stream)) = next {
                        let peer_addr = stream.peer_addr().unwrap();
                        debug!("new connection from {}", peer_addr);
                        task::spawn(async move {
                            onconnection(stream.clone(), false).await;
                        });
                    }
                }
                Either::Right((_, _)) => return,
            }
        }
    });
    kill_tx
}

async fn onconnection(reader: TcpStream, is_initiator: bool) {
    let key = [0u8; 32];
    let framed = Uint24LELengthPrefixedFraming::new(reader);
    let cipher = if is_initiator {
        let ss = SecStream::new_initiator_xx(&[]).unwrap();
        Cipher::new(Some(Box::new(framed)), ss.into())
    } else {
        let keypair = generate_keypair().unwrap();
        let ss = SecStream::new_responder_xx(&keypair, &[]).unwrap();
        Cipher::new(Some(Box::new(framed)), ss.into())
    };
    let mut protocol = Protocol::new(Box::new(cipher));
    while let Some(Ok(event)) = protocol.next().await {
        // eprintln!("RECV EVENT [{}] {:?}", protocol.is_initiator(), event);
        match event {
            Event::Handshake(_) => {
                protocol.open(key).await.unwrap();
            }
            Event::DiscoveryKey(_) => {}
            Event::Channel(channel) => {
                task::spawn(onchannel(channel, is_initiator));
            }
            Event::Close(_dkey) => {
                return;
            }
            _ => {}
        }
    }
}

async fn onchannel(mut channel: Channel, is_initiator: bool) {
    if is_initiator {
        channel_client(&mut channel).await
    } else {
        channel_server(&mut channel).await
    }
    let _res = channel.close().await;
}

async fn channel_server(channel: &mut Channel) {
    while let Some(message) = channel.next().await {
        if let Message::Data(_) = message {
            channel.send(message).await.unwrap()
        }
    }
}

async fn channel_client(channel: &mut Channel) {
    let data = vec![0u8; SIZE as usize];
    let start = Instant::now();
    let message = msg_data(0, data.clone());
    channel.send(message).await.unwrap();
    while let Some(message) = channel.next().await {
        if let Message::Data(ref msg) = message {
            if index(msg) < COUNT {
                let message = msg_data(index(msg) + 1, data.clone());
                channel.send(message).await.unwrap();
            } else {
                let time = start.elapsed();
                let bytes = COUNT * SIZE;
                trace!(
                    "client completed. {} blocks, {} bytes, {:?}",
                    index(msg),
                    bytes,
                    time
                );
                break;
            }
        }
    }
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

fn index(msg: &Data) -> u64 {
    msg.request
}
