#![allow(dead_code, unused_imports)]

use async_std::net::TcpStream;
use async_std::prelude::*;
use async_std::task::{self, JoinHandle};
use futures_lite::io::{AsyncRead, AsyncWrite};
use hypercore_protocol::schema::*;
use hypercore_protocol::{discovery_key, Channel, Event, Message, Protocol, ProtocolBuilder};
use std::io;

mod _util;
use _util::*;

// Drive a stream to completion in a task.
fn drive<S>(mut proto: S) -> JoinHandle<()>
where
    S: Stream + Send + Unpin + 'static,
{
    task::spawn(async move { while let Some(_event) = proto.next().await {} })
}

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

#[async_std::test]
async fn stream_extension() -> anyhow::Result<()> {
    // env_logger::init();
    let (mut proto_a, mut proto_b) = create_pair_memory().await?;

    let mut ext_a = proto_a.register_extension("ext");
    let mut ext_b = proto_b.register_extension("ext");

    drive(proto_a);
    drive(proto_b);

    task::spawn(async move {
        while let Some(message) = ext_b.next().await {
            assert_eq!(message, b"hello".to_vec());
            // eprintln!("B received: {:?}", String::from_utf8(message));
            ext_b.send(b"ack".to_vec()).await;
        }
    });

    ext_a.send(b"hello".to_vec()).await;
    let response = ext_a.next().await;
    assert_eq!(response, Some(b"ack".to_vec()));
    // eprintln!("A received: {:?}", response.map(String::from_utf8));
    Ok(())
}

#[async_std::test]
async fn channel_extension() -> anyhow::Result<()> {
    // env_logger::init();
    let (mut proto_a, mut proto_b) = create_pair_memory().await?;
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

    task::spawn(async move {
        while let Some(message) = ext_b.next().await {
            // eprintln!("B received: {:?}", String::from_utf8(message));
            assert_eq!(message, b"hello".to_vec());
            ext_b.send(b"ack".to_vec()).await;
        }
    });

    ext_a.send(b"hello".to_vec()).await;
    let response = ext_a.next().await;
    assert_eq!(response, Some(b"ack".to_vec()));
    // eprintln!("A received: {:?}", response.map(String::from_utf8));
    Ok(())
}
