use async_std::net::{Shutdown, TcpListener, TcpStream};
use async_std::task;
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use futures::future::Either;
use futures::io::{AsyncRead, AsyncWrite};
use futures::stream::{FuturesUnordered, StreamExt};
use hypercore_protocol::{schema::*, Duplex};
use hypercore_protocol::{Channel, Event, Message, ProtocolBuilder};
use log::*;
use std::time::Instant;

const PORT: usize = 11011;
const SIZE: u64 = 1000;
const COUNT: u64 = 200;
const CLIENTS: usize = 1;

fn bench_throughput(c: &mut Criterion) {
    env_logger::from_env(env_logger::Env::default().default_filter_or("error")).init();
    let address = format!("localhost:{}", PORT);

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
                            onconnection(s.clone(), s.clone(), true).await;
                            s.shutdown(Shutdown::Both)
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
    log::info!("listening on {}", listener.local_addr().unwrap());
    let (kill_tx, mut kill_rx) = futures::channel::oneshot::channel();
    task::spawn(async move {
        let mut incoming = listener.incoming();
        // let kill_rx = &mut kill_rx;
        loop {
            match futures::future::select(incoming.next(), &mut kill_rx).await {
                Either::Left((next, _)) => match next {
                    Some(Ok(stream)) => {
                        let peer_addr = stream.peer_addr().unwrap();
                        debug!("new connection from {}", peer_addr);
                        task::spawn(async move {
                            onconnection(stream.clone(), stream, false).await;
                        });
                    }
                    _ => {}
                },
                Either::Right((_, _)) => return,
            }
        }
    });
    kill_tx
}

async fn onconnection<R, W>(reader: R, writer: W, is_initiator: bool) -> Duplex<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let key = [0u8; 32];
    let mut protocol = ProtocolBuilder::new(is_initiator)
        .set_encrypted(false)
        .connect_rw(reader, writer);
    while let Some(Ok(event)) = protocol.next().await {
        // eprintln!("RECV EVENT [{}] {:?}", protocol.is_initiator(), event);
        match event {
            Event::Handshake(_) => {
                protocol.open(key.clone()).await.unwrap();
            }
            Event::DiscoveryKey(_) => {}
            Event::Channel(channel) => {
                task::spawn(onchannel(channel, is_initiator));
            }
            Event::Close(_dkey) => {
                return protocol.release();
            }
            _ => {}
        }
    }
    protocol.release()
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
        match message {
            Message::Data(_) => channel.send(message).await.unwrap(),
            _ => {}
        }
    }
}

async fn channel_client(channel: &mut Channel) {
    let data = vec![0u8; SIZE as usize];
    let start = Instant::now();
    let message = msg_data(0, data.clone());
    channel.send(message).await.unwrap();
    while let Some(message) = channel.next().await {
        match message {
            Message::Data(ref msg) => {
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
            _ => {}
        }
    }
}

fn msg_data(index: u64, value: Vec<u8>) -> Message {
    use hypercore::DataBlock;

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
