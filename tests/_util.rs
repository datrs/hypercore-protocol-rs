#![allow(unused)]
use async_compat::CompatExt;
use futures_lite::StreamExt;
use hypercore_handshake::{Cipher, state_machine::SecStream};
use hypercore_protocol::{Channel, DiscoveryKey, Event, Protocol};
use instant::Duration;
use std::io;
use tokio::task::JoinHandle;
use uint24le_framing::Uint24LELengthPrefixedFraming;

/// Create a connected pair of test streams.
pub fn create_connected_streams() -> (Cipher, Cipher) {
    let (left, right) = tokio::io::duplex(10_000);
    let left = Uint24LELengthPrefixedFraming::new(left.compat());
    let right = Uint24LELengthPrefixedFraming::new(right.compat());

    let kp = hypercore_handshake::state_machine::hc_specific::generate_keypair().unwrap();

    let initiator = Cipher::new_init(
        Box::new(left),
        SecStream::new_initiator_ik(&kp.public.clone().try_into().unwrap(), &[]).unwrap(),
    );
    let responder = Cipher::new_resp(
        Box::new(right),
        SecStream::new_responder_ik(&kp, &[]).unwrap(),
    );
    dbg!(responder.get_remote_static());

    (initiator, responder)
}

/// Create a connected pair of protocols for testing.
pub fn create_pair() -> (Protocol, Protocol) {
    let (stream_a, stream_b) = create_connected_streams();

    let proto_a = Protocol::new(Box::new(stream_a));
    let proto_b = Protocol::new(Box::new(stream_b));

    (proto_a, proto_b)
}

pub fn next_event(mut proto: Protocol) -> JoinHandle<(Protocol, io::Result<Event>)> {
    tokio::task::spawn(async move {
        let e1 = proto.next().await;
        let e1 = e1.unwrap();
        (proto, e1)
    })
}

pub fn event_discovery_key(event: Event) -> DiscoveryKey {
    if let Event::DiscoveryKey(dkey) = event {
        dkey
    } else {
        panic!("Expected discovery key event");
    }
}

pub fn event_channel(event: Event) -> Channel {
    if let Event::Channel(channel) = event {
        channel
    } else {
        panic!("Expected channel event");
    }
}

/// Drive a protocol stream until the first channel arrives.
pub fn drive_until_channel(mut proto: Protocol) -> JoinHandle<io::Result<(Protocol, Channel)>> {
    tokio::task::spawn(async move {
        while let Some(event) = proto.next().await {
            let event = event?;
            if let Event::Channel(channel) = event {
                return Ok((proto, channel));
            }
        }
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Protocol closed before a channel was opened",
        ))
    })
}

#[allow(unused)]
pub async fn wait_for_localhost_port(port: u32) {
    use async_std::net::TcpStream;
    const RETRY_TIMEOUT: u64 = 100_u64;
    const NO_RESPONSE_TIMEOUT: u64 = 1000_u64;
    loop {
        let timeout = async_std::future::timeout(
            Duration::from_millis(NO_RESPONSE_TIMEOUT),
            TcpStream::connect(format!("localhost:{port}")),
        )
        .await;
        if timeout.is_err() {
            continue;
        }
        if timeout.unwrap().is_err() {
            async_std::task::sleep(Duration::from_millis(RETRY_TIMEOUT)).await;
        } else {
            break;
        }
    }
}
