use async_std::io;
use async_std::net::TcpStream;
use async_std::prelude::*;
use async_std::sync::{Arc, RwLock};
use async_std::task;
use async_trait::async_trait;
use pretty_hash::fmt as pretty_fmt;
use std::env;
use std::io::Result;

use simple_hypercore_protocol::schema::*;
use simple_hypercore_protocol::{discovery_key, ProtocolBuilder};
use simple_hypercore_protocol::{ChannelContext, ChannelHandlers, StreamContext, StreamHandlers};

mod util;
use util::{tcp_client, tcp_server};

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

/// Print usage and exit.
fn usage() {
    println!("usage: cargo run --example basic -- [client|server] [port] [key]");
    std::process::exit(1);
}

// The onconnection handler is called for each incoming connection (if server)
// or once when connected (if client).
async fn onconnection(
    stream: TcpStream,
    is_initiator: bool,
    feedstore: Arc<FeedStore>,
) -> Result<()> {
    let mut protocol = ProtocolBuilder::new(is_initiator)
        .set_handlers(feedstore)
        .from_stream(stream);

    protocol.listen().await
}

struct FeedStore {
    feeds: Vec<Arc<Feed>>,
}
impl FeedStore {
    pub fn new() -> Self {
        Self { feeds: vec![] }
    }

    pub fn add(&mut self, feed: Feed) {
        self.feeds.push(Arc::new(feed));
    }
}

#[derive(Debug)]
struct Feed {
    key: Vec<u8>,
    discovery_key: Vec<u8>,
    state: RwLock<FeedState>,
}
impl Feed {
    pub fn new(key: Vec<u8>) -> Self {
        Feed {
            discovery_key: discovery_key(&key),
            key,
            state: RwLock::new(FeedState::default()),
        }
    }
}

/// A FeedState stores the head seq of the remote.
/// This would have a bitfield to support sparse sync in the actual impl.
#[derive(Debug)]
struct FeedState {
    remote_head: u64,
    started: bool,
}
impl Default for FeedState {
    fn default() -> Self {
        FeedState {
            remote_head: 0,
            started: false,
        }
    }
}

#[async_trait]
impl StreamHandlers for FeedStore {
    async fn on_discoverykey(
        &self,
        protocol: &mut StreamContext,
        discovery_key: &[u8],
    ) -> Result<()> {
        log::trace!(
            "resolve discovery_key: {}",
            pretty_fmt(discovery_key).unwrap()
        );
        let feed = self
            .feeds
            .iter()
            .find(|feed| feed.discovery_key == discovery_key);
        match feed {
            Some(feed) => {
                let key = feed.key.clone();
                let feed_handler = Arc::clone(&feed);
                protocol.open(key, feed_handler).await
            }
            None => Ok(()),
        }
    }
}

#[async_trait]
impl ChannelHandlers for Feed {
    async fn on_open<'a>(
        &self,
        channel: &mut ChannelContext<'a>,
        discovery_key: &[u8],
    ) -> Result<()> {
        log::info!("open channel {}", pretty_fmt(&discovery_key).unwrap());

        let msg = Want {
            start: 0,
            length: None, // length: Some(1048576),
        };
        channel.want(msg).await?;

        Ok(())
    }

    async fn on_have<'a>(&self, channel: &mut ChannelContext<'a>, msg: Have) -> Result<()> {
        let mut state = self.state.write().await;
        // Check if the remote announces a new head.
        log::info!("receive have: {} (state {})", msg.start, state.remote_head);
        if msg.start > state.remote_head {
            // Store the new remote head.
            state.remote_head = msg.start;
            // If we didn't start reading, request first data block.
            if !state.started {
                state.started = true;
                let msg = Request {
                    index: 0,
                    bytes: None,
                    hash: None,
                    nodes: None,
                };
                channel.request(msg).await?;
            }
        }
        Ok(())
    }

    async fn on_data<'a>(&self, channel: &mut ChannelContext<'a>, msg: Data) -> Result<()> {
        let state = self.state.read().await;
        log::info!(
            "receive data: idx {}, {} bytes (remote_head {})",
            msg.index,
            msg.value.as_ref().map_or(0, |v| v.len()),
            state.remote_head
        );

        if let Some(value) = msg.value {
            let mut stdout = io::stdout();
            stdout.write_all(&value).await?;
            stdout.flush().await?;
        }

        let next = msg.index + 1;
        if state.remote_head >= next {
            // Request next data block.
            let msg = Request {
                index: next,
                bytes: None,
                hash: None,
                nodes: None,
            };
            channel.request(msg).await?;
        }

        Ok(())
    }
}
