use async_std::io;
use async_std::net::{TcpListener, TcpStream};
use async_std::prelude::*;
use async_std::stream::StreamExt;
use async_std::sync::{Arc, RwLock};
use async_std::task;
use async_trait::async_trait;
use pretty_hash::fmt as pretty_fmt;
use std::env;
use std::io::Result;

use simple_hypercore_protocol::{
    discovery_key, schema, schema::*, ChannelContext, ChannelHandlers, Message, Protocol,
    StreamContext, StreamHandlers,
};

fn main() {
    env_logger::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if env::args().count() < 3 {
        usage();
    }
    let mode = env::args().nth(1).unwrap();
    let port = env::args().nth(2).unwrap();
    let key = env::args().nth(3);
    let address = format!("127.0.0.1:{}", port);

    task::block_on(async move {
        let result = match mode.as_ref() {
            "server" => tcp_server(address, key).await,
            "client" => tcp_client(address, key).await,
            _ => panic!(usage()),
        };
        log_if_error(&result);
    });
}

/// Print usage and exit.
fn usage() {
    println!("usage: cargo run --example basic -- [client|server] [port] [key]");
    std::process::exit(1);
}

fn log_if_error(result: &Result<()>) {
    if let Err(err) = result.as_ref() {
        log::error!("error: {}", err);
    }
}

async fn tcp_server(address: String, key: Option<String>) -> Result<()> {
    let listener = TcpListener::bind(&address).await?;
    log::info!("listening on {}", listener.local_addr()?);
    let mut incoming = listener.incoming();
    while let Some(Ok(stream)) = incoming.next().await {
        let key = key.clone();
        let peer_addr = stream.peer_addr().unwrap();
        log::info!("new connection from {}", peer_addr);
        task::spawn(async move {
            let result = onconnection(stream, false, key).await;
            log_if_error(&result);
            log::info!("connection closed from {}", peer_addr);
        });
    }
    Ok(())
}

async fn tcp_client(address: String, key: Option<String>) -> Result<()> {
    let stream = TcpStream::connect(&address).await?;
    onconnection(stream, true, key).await
}

// This is where our actual application code starts.

async fn onconnection(stream: TcpStream, is_initiator: bool, key: Option<String>) -> Result<()> {
    let mut feedstore = FeedStore::new();

    let key = key.map_or(None, |key| Some(hex::decode(key)));
    if let Some(Ok(key)) = key {
        let feed = Feed::new(key);
        feedstore.add(feed);
    }

    let reader = stream.clone();
    let writer = stream.clone();
    let mut protocol = Protocol::from_rw(reader, writer, is_initiator).await?;
    protocol.set_handlers(Arc::new(feedstore));
    protocol.listen().await?;

    Ok(())
}

/// A FeedState stores the head seq of the remote.
/// This would have a bitfield to support sparse sync in the actual impl.
#[derive(Debug)]
struct FeedState {
    remote_head: u64,
    started: bool
}
impl Default for FeedState {
    fn default() -> Self {
        FeedState { remote_head: 0, started: false }
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

#[async_trait]
impl ChannelHandlers for Feed {
    async fn on_open<'a>(
        &self,
        channel: &mut ChannelContext<'a>,
        discovery_key: &[u8],
    ) -> Result<()> {
        log::info!("open channel {}", pretty_fmt(&discovery_key).unwrap());

        let msg = Message::Want(Want {
            start: 0,
            length: None, // length: Some(1048576),
        });
        channel.send(msg).await?;


        Ok(())
    }

    async fn on_have<'a>(&self, channel: &mut ChannelContext<'a>, msg: Have) -> Result<()> {
        let mut state = self.state.write().await;
        // Check if the remote announces a new head.
        log::info!(
            "receive have: start {:} (state {:?})",
            msg.start,
            state.remote_head
        );
        if msg.start > state.remote_head {
            // Store the new remote head.
            state.remote_head = msg.start;
            // If we didn't start reading, request first data block. 
            if !state.started {
                state.started = true;
                let msg = Message::Request(Request {
                    index: 0,
                    bytes: None,
                    hash: None,
                    nodes: None,
                });
                channel.send(msg).await?;
            }
        }
        Ok(())
    }

    async fn on_data<'a>(&self, channel: &mut ChannelContext<'a>, msg: schema::Data) -> Result<()> {
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
            let msg = Message::Request(Request {
                index: next,
                bytes: None,
                hash: None,
                nodes: None,
            });
            channel.send(msg).await?;
        }

        Ok(())
    }
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
        let feed = self.feeds.iter().find(|feed| feed.discovery_key == discovery_key);
        match feed {
            None => Ok(()),
            Some(feed) => {
                let feed = Arc::clone(&feed);
                protocol.open(feed.key.clone(), feed).await
            }
        }
    }
}
