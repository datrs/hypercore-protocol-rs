use async_std::net::{TcpListener, TcpStream};
use async_std::task;
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use futures::io::{AsyncRead, AsyncWrite};
use futures::stream::StreamExt;
use hypercore_protocol::schema::*;
use hypercore_protocol::{Channel, Event, Message, ProtocolBuilder};
use log::*;
use std::time::Instant;

const PORT: usize = 11011;
const SIZE: u64 = 1000;
const COUNT: u64 = 1000;
// const CLIENTS: usize = 5;

fn bench_throughput(c: &mut Criterion) {
    env_logger::from_env(env_logger::Env::default().default_filter_or("error")).init();
    let address = format!("localhost:{}", PORT);

    let mut group = c.benchmark_group("throughput");
    let data = vec![1u8; SIZE as usize];

    let _server = task::block_on(start_server(&address));

    group.sample_size(10);
    group.throughput(Throughput::Bytes(data.len() as u64 * COUNT));
    group.bench_function("echo", |b| {
        b.iter_with_setup(
            || {
                let stream = task::block_on(TcpStream::connect(&address)).unwrap();
                stream
            },
            |stream| {
                task::block_on(async move {
                    onconnection(stream.clone(), stream, true).await;
                })
            },
        );
    });

    group.finish();
}

criterion_group!(server_benches, bench_throughput);
criterion_main!(server_benches);

async fn start_server(address: &str) {
    let listener = TcpListener::bind(&address).await.unwrap();
    log::info!("listening on {}", listener.local_addr().unwrap());
    task::spawn(async move {
        let mut incoming = listener.incoming();
        while let Some(Ok(stream)) = incoming.next().await {
            let peer_addr = stream.peer_addr().unwrap();
            debug!("new connection from {}", peer_addr);
            task::spawn(async move {
                onconnection(stream.clone(), stream, false).await;
            });
        }
    });
}

async fn onconnection<R, W>(reader: R, writer: W, is_initiator: bool) -> (R, W)
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let key = vec![0u8; 32];
    let mut protocol = ProtocolBuilder::new(is_initiator)
        .set_encrypted(false)
        .build_from_io(reader, writer);
    while let Ok(event) = protocol.loop_next().await {
        match event {
            Event::Handshake(_) => {
                protocol.open(key.clone()).await.unwrap();
            }
            Event::DiscoveryKey(_) => {}
            Event::Channel(channel) => {
                task::spawn(onchannel(channel, is_initiator));
            }
            Event::Close(_dkey) => {
                if is_initiator {
                    return protocol.release();
                }
            }
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
    channel
        .send(Message::Close(Close {
            discovery_key: None,
        }))
        .await
        .unwrap();
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
    let message = Message::Data(Data {
        index: 0,
        value: Some(data.clone()),
        nodes: vec![],
        signature: None,
    });
    channel.send(message).await.unwrap();
    while let Some(message) = channel.next().await {
        match message {
            Message::Data(msg) => {
                if msg.index < COUNT {
                    let message = Message::Data(Data {
                        index: msg.index + 1,
                        value: Some(data.clone()),
                        nodes: vec![],
                        signature: None,
                    });
                    channel.send(message).await.unwrap();
                } else {
                    let time = start.elapsed();
                    let bytes = COUNT * SIZE;
                    trace!(
                        "client completed. {} blocks, {} bytes, {:?}",
                        msg.index,
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
