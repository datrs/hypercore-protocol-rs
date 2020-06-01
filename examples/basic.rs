use anyhow::Result;
use async_std::io;
use async_std::net::TcpStream;
use async_std::prelude::*;
use async_std::sync::Arc;
use async_std::task;
use futures::stream::StreamExt;
use log::*;
use std::collections::HashMap;
use std::env;

use hypercore_protocol::schema::*;
use hypercore_protocol::{discovery_key, Channel, Event, Message, ProtocolBuilder};

mod util;
use util::{tcp_client, tcp_server};

/// Print usage and exit.
fn usage() {
    println!("usage: cargo run --example basic -- [client|server] [port] [key]");
    std::process::exit(1);
}

fn main() {
    util::init_logger();
    if env::args().count() < 3 {
        usage();
    }
    let mode = env::args().nth(1).unwrap();
    let port = env::args().nth(2).unwrap();
    let address = format!("127.0.0.1:{}", port);

    let key = env::args().nth(3);
    let key = key.map_or(None, |key| hex::decode(key).ok());

    let mut feedstore = FeedStore::new();
    if let Some(key) = key {
        feedstore.add(Feed::new(key));
    } else {
        let key = vec![9u8; 32];
        feedstore.add(Feed::new(key.clone()));
        println!("KEY={}", hex::encode(&key));
    }
    let feedstore = Arc::new(feedstore);

    task::block_on(async move {
        let result = match mode.as_ref() {
            "server" => tcp_server(address, onconnection, feedstore).await,
            "client" => tcp_client(address, onconnection, feedstore).await,
            _ => panic!(usage()),
        };
        util::log_if_error(&result);
    });
}

// The onconnection handler is called for each incoming connection (if server)
// or once when connected (if client).
async fn onconnection(
    stream: TcpStream,
    is_initiator: bool,
    feedstore: Arc<FeedStore>,
) -> Result<()> {
    let mut protocol = ProtocolBuilder::new(is_initiator)
        .connect(stream)
        .into_stream();
    while let Some(event) = protocol.next().await {
        let event = event?;
        debug!("EVENT {:?}", event);
        match event {
            Event::Handshake(_) => {
                if is_initiator {
                    for feed in feedstore.feeds.values() {
                        protocol.open(feed.key.clone()).await?;
                    }
                }
            }
            Event::DiscoveryKey(dkey) => {
                if let Some(feed) = feedstore.get(&dkey) {
                    protocol.open(feed.key.clone()).await?;
                }
            }
            Event::Channel(mut channel) => {
                if let Some(feed) = feedstore.get(&channel.discovery_key()) {
                    let feed = feed.clone();
                    let mut state = FeedState::default();
                    task::spawn(async move {
                        while let Some(message) = channel.next().await {
                            onmessage(&*feed, &mut state, &mut channel, message).await;
                        }
                    });
                }
            }
            _ => {}
        }
    }
    Ok(())
}

struct FeedStore {
    pub feeds: HashMap<String, Arc<Feed>>,
}
impl FeedStore {
    pub fn new() -> Self {
        let feeds = HashMap::new();
        Self { feeds }
    }

    pub fn add(&mut self, feed: Feed) {
        let hdkey = hex::encode(&feed.discovery_key);
        self.feeds.insert(hdkey, Arc::new(feed));
    }

    pub fn get(&self, discovery_key: &[u8]) -> Option<&Arc<Feed>> {
        let hdkey = hex::encode(discovery_key);
        self.feeds.get(&hdkey)
    }
}

/// A Feed is a single unit of replication, an append-only log.
/// This toy feed can only read sequentially and does not save or buffer anything.
#[derive(Debug)]
struct Feed {
    key: Vec<u8>,
    discovery_key: Vec<u8>,
}
impl Feed {
    pub fn new(key: Vec<u8>) -> Self {
        Feed {
            discovery_key: discovery_key(&key),
            key,
        }
    }
}

/// A FeedState stores the head seq of the remote.
/// This would have a bitfield to support sparse sync in the actual impl.
#[derive(Debug)]
struct FeedState {
    remote_head: Option<u64>,
}
impl Default for FeedState {
    fn default() -> Self {
        FeedState { remote_head: None }
    }
}

async fn onmessage(_feed: &Feed, state: &mut FeedState, channel: &mut Channel, message: Message) {
    match message {
        Message::Open(_) => {
            let msg = Want {
                start: 0,
                length: None,
            };
            channel
                .send(Message::Want(msg))
                .await
                .expect("failed to send");
        }
        Message::Have(msg) => {
            if state.remote_head == None {
                state.remote_head = Some(msg.start);
                let msg = Request {
                    index: 0,
                    bytes: None,
                    hash: None,
                    nodes: None,
                };
                channel.send(Message::Request(msg)).await.unwrap();
            } else if let Some(remote_head) = state.remote_head {
                if remote_head < msg.start {
                    state.remote_head = Some(msg.start)
                }
            }
        }
        Message::Data(msg) => {
            debug!(
                "receive data: idx {}, {} bytes (remote_head {:?})",
                msg.index,
                msg.value.as_ref().map_or(0, |v| v.len()),
                state.remote_head
            );

            if let Some(value) = msg.value {
                let mut stdout = io::stdout();
                stdout.write_all(&value).await.unwrap();
                stdout.flush().await.unwrap();
            }

            let next = msg.index + 1;
            if let Some(remote_head) = state.remote_head {
                if remote_head >= next {
                    // Request next data block.
                    let msg = Request {
                        index: next,
                        bytes: None,
                        hash: None,
                        nodes: None,
                    };
                    channel.send(Message::Request(msg)).await.unwrap();
                }
            }
        }
        _ => {}
    }
}
