#![allow(dead_code, unused_imports)]

use async_std::net::TcpStream;
use async_std::prelude::*;
use async_std::task;
use futures_lite::io::{AsyncRead, AsyncWrite};
use hypercore_protocol::schema::*;
use hypercore_protocol::{discovery_key, Channel, Event, Message, Protocol, ProtocolBuilder};

mod _util;
use _util::*;

#[async_std::test]
async fn basic_protocol() -> anyhow::Result<()> {
    // env_logger::init();
    let (proto_a, proto_b) = create_pair_memory().await?;

    let next_a = next_event(proto_a);
    let next_b = next_event(proto_b);
    let (event_a, mut proto_a) = next_a.await;
    let (event_b, proto_b) = next_b.await;

    assert!(matches!(event_a, Ok(Event::Handshake(_))));
    assert!(matches!(event_b, Ok(Event::Handshake(_))));

    assert_eq!(proto_a.public_key(), proto_b.remote_public_key());
    assert_eq!(proto_b.public_key(), proto_a.remote_public_key());

    let key = [3u8; 32];

    proto_a.open(key.clone()).await?;

    let next_a = next_event(proto_a);
    let next_b = next_event(proto_b);

    let (event_b, mut proto_b) = next_b.await;
    assert!(matches!(event_b, Ok(Event::DiscoveryKey(_))));
    assert_eq!(event_discovery_key(event_b.unwrap()), discovery_key(&key));

    proto_b.open(key.clone()).await?;

    let next_b = next_event(proto_b);
    let (event_b, proto_b) = next_b.await;
    assert!(matches!(event_b, Ok(Event::Channel(_))));
    let mut channel_b = event_channel(event_b.unwrap());

    let (event_a, proto_a) = next_a.await;
    assert!(matches!(event_a, Ok(Event::Channel(_))));
    let mut channel_a = event_channel(event_a.unwrap());

    assert_eq!(channel_a.discovery_key(), channel_b.discovery_key());

    channel_a
        .want(Want {
            start: 0,
            length: Some(10),
        })
        .await?;

    channel_b
        .want(Want {
            start: 10,
            length: Some(5),
        })
        .await?;

    let next_a = next_event(proto_a);
    let next_b = next_event(proto_b);

    let channel_event_b = channel_b.next().await;
    assert_eq!(
        channel_event_b,
        Some(Message::Want(Want {
            start: 0,
            length: Some(10)
        }))
    );
    // eprintln!("channel_event_b: {:?}", channel_event_b);

    let channel_event_a = channel_a.next().await;
    assert_eq!(
        channel_event_a,
        Some(Message::Want(Want {
            start: 10,
            length: Some(5)
        }))
    );

    channel_a
        .close(Close {
            discovery_key: None,
        })
        .await?;
    channel_b
        .close(Close {
            discovery_key: None,
        })
        .await?;

    let (event_a, _proto_a) = next_a.await;
    let (event_b, _proto_b) = next_b.await;

    assert!(matches!(event_a, Ok(Event::Close(_))));
    assert!(matches!(event_b, Ok(Event::Close(_))));
    return Ok(());
}
