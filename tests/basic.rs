#![allow(dead_code, unused_imports)]

use async_std::net::TcpStream;
use async_std::prelude::*;
use async_std::task;
use futures_lite::io::{AsyncRead, AsyncWrite};
use hypercore_protocol::{discovery_key, Channel, Event, Message, Protocol, ProtocolBuilder};
use hypercore_protocol::{schema::*, DiscoveryKey};
use std::io;
use test_log::test;

mod _util;
use _util::*;

#[test(async_std::test)]
async fn basic_protocol() -> anyhow::Result<()> {
    // env_logger::init();
    let (proto_a, proto_b) = create_pair_memory().await?;

    let next_a = next_event(proto_a);
    let next_b = next_event(proto_b);
    let (mut proto_a, event_a) = next_a.await;
    let (proto_b, event_b) = next_b.await;

    assert!(matches!(event_a, Ok(Event::Handshake(_))));
    assert!(matches!(event_b, Ok(Event::Handshake(_))));

    assert_eq!(proto_a.public_key(), proto_b.remote_public_key());
    assert_eq!(proto_b.public_key(), proto_a.remote_public_key());

    let key = [3u8; 32];

    proto_a.open(key).await?;

    let next_a = next_event(proto_a);
    let next_b = next_event(proto_b);

    let (mut proto_b, event_b) = next_b.await;
    assert!(matches!(event_b, Ok(Event::DiscoveryKey(_))));
    assert_eq!(event_discovery_key(event_b.unwrap()), discovery_key(&key));

    proto_b.open(key).await?;

    let next_b = next_event(proto_b);
    let (proto_b, event_b) = next_b.await;
    assert!(matches!(event_b, Ok(Event::Channel(_))));
    let mut channel_b = event_channel(event_b.unwrap());

    let (proto_a, event_a) = next_a.await;
    assert!(matches!(event_a, Ok(Event::Channel(_))));
    let mut channel_a = event_channel(event_a.unwrap());

    assert_eq!(channel_a.discovery_key(), channel_b.discovery_key());

    channel_a.want(want(0, 10)).await?;

    channel_b.want(want(10, 5)).await?;

    let next_a = next_event(proto_a);
    let next_b = next_event(proto_b);

    let channel_event_b = channel_b.next().await;
    assert_eq!(channel_event_b, Some(Message::Want(want(0, 10))));
    // eprintln!("channel_event_b: {:?}", channel_event_b);

    let channel_event_a = channel_a.next().await;
    assert_eq!(channel_event_a, Some(Message::Want(want(10, 5))));

    channel_a.close().await?;
    channel_b.close().await?;

    let (_, event_a) = next_a.await;
    let (_, event_b) = next_b.await;

    assert!(matches!(event_a, Ok(Event::Close(_))));
    assert!(matches!(event_b, Ok(Event::Close(_))));
    Ok(())
}

#[test(async_std::test)]
async fn open_close_channels() -> anyhow::Result<()> {
    let (mut proto_a, mut proto_b) = create_pair_memory().await?;

    let key1 = [0u8; 32];
    let key2 = [1u8; 32];

    proto_a.open(key1).await?;
    proto_b.open(key1).await?;

    let next_a = drive_until_channel(proto_a);
    let next_b = drive_until_channel(proto_b);

    let (mut proto_a, mut channel_a1) = next_a.await?;
    let (mut proto_b, mut channel_b1) = next_b.await?;

    proto_a.open(key2).await?;
    proto_b.open(key2).await?;

    let next_a = drive_until_channel(proto_a);
    let next_b = drive_until_channel(proto_b);

    let (proto_a, mut channel_a2) = next_a.await?;
    let (proto_b, mut channel_b2) = next_b.await?;

    eprintln!(
        "got channels: {:?}",
        (&channel_a1, &channel_a2, &channel_b1, &channel_b2)
    );

    let channels_a: Vec<&DiscoveryKey> = proto_a.channels().collect();
    let channels_b: Vec<&DiscoveryKey> = proto_b.channels().collect();
    eprintln!("open channels a {:?}", channels_a.len());
    eprintln!("open channels b {:?}", channels_b.len());
    assert_eq!(channels_a.len(), 2);
    assert_eq!(channels_b.len(), 2);

    channel_a1.close().await?;

    let next_a = next_event(proto_a);
    let next_b = next_event(proto_b);
    let (mut proto_a, ev_a) = next_a.await;
    let (mut proto_b, ev_b) = next_b.await;
    let ev_a = ev_a?;
    let ev_b = ev_b?;
    eprintln!("next a: {:?}", ev_a);
    eprintln!("next b: {:?}", ev_b);

    let channels_a: Vec<&DiscoveryKey> = proto_a.channels().collect();
    let channels_b: Vec<&DiscoveryKey> = proto_b.channels().collect();
    eprintln!("open channels a {:?}", channels_a.len());
    eprintln!("open channels b {:?}", channels_b.len());
    assert_eq!(channels_a.len(), 1);
    assert_eq!(channels_b.len(), 1);

    let res = channel_a1.want(want(0, 1)).await;
    assert!(matches!(res, Err(ref e) if e.kind() == io::ErrorKind::ConnectionAborted));

    let res = channel_b1.want(want(0, 2)).await;
    assert!(matches!(res, Err(ref e) if e.kind() == io::ErrorKind::ConnectionAborted));

    // Test that channel 2 still works
    let res = channel_a2.want(want(0, 10)).await;
    assert!(matches!(res, Ok(())));

    let res = channel_b2.want(want(0, 20)).await;
    assert!(matches!(res, Ok(())));

    // Check that the message arrives.
    task::spawn(async move {
        loop {
            proto_a.next().await.unwrap().unwrap();
        }
    });
    task::spawn(async move {
        loop {
            proto_b.next().await.unwrap().unwrap();
        }
    });

    let msg_a = channel_a2.next().await;
    let msg_b = channel_b2.next().await;

    assert_eq!(msg_a, Some(Message::Want(want(0, 20))));
    assert_eq!(msg_b, Some(Message::Want(want(0, 10))));

    eprintln!("all good!");

    Ok(())
}

fn want(start: u64, length: u64) -> Want {
    Want { start, length }
}
