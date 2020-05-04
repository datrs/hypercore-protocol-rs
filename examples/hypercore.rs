use async_std::net::TcpStream;
use async_std::sync::{Arc, Mutex};
use async_std::task;
use futures::stream::StreamExt;
use hypercore::{Feed, Node, Proof, PublicKey, Signature, Storage};
use log::*;
use random_access_memory::RandomAccessMemory;
use random_access_storage::RandomAccess;
use std::collections::HashMap;
use std::env;
use std::fmt::Debug;
use std::io::Result;

use hypercore_protocol::schema::*;
use hypercore_protocol::{discovery_key, Channel, Event, Message, ProtocolBuilder};

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

    task::block_on(async move {
        let mut feedstore: FeedStore<RandomAccessMemory> = FeedStore::new();
        if let Some(key) = key {
            // Create a hypercore.
            let storage = Storage::new_memory().await.unwrap();
            let public_key = PublicKey::from_bytes(&key).unwrap();
            let feed = Feed::builder(public_key, storage).build().unwrap();

            // Wrap it and add to the feed store.
            let feed_wrapper = FeedWrapper::from_memory_feed(feed);
            feedstore.add(feed_wrapper);
        }
        let feedstore = Arc::new(feedstore);

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
// Unfortunately, everything that touches the feedstore or a feed has to be generic
// at the moment.
async fn onconnection<T: 'static>(
    stream: TcpStream,
    is_initiator: bool,
    feedstore: Arc<FeedStore<T>>,
) -> Result<()>
where
    T: RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>> + Debug + Send,
{
    let mut protocol = ProtocolBuilder::new(is_initiator)
        .build_from_stream(stream)
        .into_stream();

    while let Some(event) = protocol.next().await {
        let event = event?;
        debug!("protocol event {:?}", event);
        match event {
            Event::Handshake(_) => {
                if is_initiator {
                    for feed in feedstore.feeds.values() {
                        protocol.open(feed.key().to_vec()).await?;
                    }
                }
            }
            Event::DiscoveryKey(dkey) => {
                if let Some(feed) = feedstore.get(&dkey) {
                    protocol.open(feed.key().to_vec()).await?;
                }
            }
            Event::Channel(channel) => {
                if let Some(feed) = feedstore.get(&channel.discovery_key()) {
                    feed.onpeer(channel);
                }
            }
        }
    }
    Ok(())
}

/// A container for hypercores.
struct FeedStore<T>
where
    T: RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>> + Debug + Send,
{
    feeds: HashMap<String, Arc<FeedWrapper<T>>>,
}
impl<T> FeedStore<T>
where
    T: RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>> + Debug + Send,
{
    pub fn new() -> Self {
        let feeds = HashMap::new();
        Self { feeds }
    }

    pub fn add(&mut self, feed: FeedWrapper<T>) {
        let hdkey = hex::encode(&feed.discovery_key);
        self.feeds.insert(hdkey, Arc::new(feed));
    }

    pub fn get(&self, discovery_key: &[u8]) -> Option<&Arc<FeedWrapper<T>>> {
        let hdkey = hex::encode(discovery_key);
        self.feeds.get(&hdkey)
    }
}

/// A Feed is a single unit of replication, an append-only log.
#[derive(Debug, Clone)]
struct FeedWrapper<T>
where
    T: RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>> + Debug + Send,
{
    discovery_key: Vec<u8>,
    key: Vec<u8>,
    feed: Arc<Mutex<Feed<T>>>,
}

impl FeedWrapper<RandomAccessMemory> {
    pub fn from_memory_feed(feed: Feed<RandomAccessMemory>) -> Self {
        let key = feed.public_key().to_bytes();
        FeedWrapper {
            key: key.to_vec(),
            discovery_key: discovery_key(&key),
            feed: Arc::new(Mutex::new(feed)),
        }
    }
}

impl<T> FeedWrapper<T>
where
    T: RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>> + Debug + Send + 'static,
{
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    pub fn onpeer(&self, mut channel: Channel) {
        let mut state = PeerState::default();
        let mut feed = self.feed.clone();
        task::spawn(async move {
            while let Some(message) = channel.next().await {
                let result = onmessage(&mut feed, &mut state, &mut channel, message).await;
                if let Err(e) = result {
                    error!("protocol error: {}", e);
                    break;
                }
            }
        });
    }
}

/// A PeerState stores the head seq of the remote.
/// This would have a bitfield to support sparse sync in the actual impl.
#[derive(Debug)]
struct PeerState {
    remote_head: Option<u64>,
}
impl Default for PeerState {
    fn default() -> Self {
        PeerState { remote_head: None }
    }
}

async fn onmessage<T>(
    feed: &mut Arc<Mutex<Feed<T>>>,
    state: &mut PeerState,
    channel: &mut Channel,
    message: Message,
) -> Result<()>
where
    T: RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>> + Debug + Send,
{
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
            let mut feed = feed.lock().await;
            let value: Option<&[u8]> = match msg.value.as_ref() {
                None => None,
                Some(value) => {
                    eprintln!(
                        "recv idx {}: {:?}",
                        msg.index,
                        String::from_utf8(value.clone()).unwrap()
                    );
                    Some(value)
                }
            };

            let signature = match msg.signature {
                Some(bytes) => Some(Signature::from_bytes(&bytes).unwrap()),
                None => None,
            };
            let nodes = msg
                .nodes
                .iter()
                .map(|n| Node::new(n.index, n.hash.clone(), n.size))
                .collect();
            let proof = Proof {
                index: msg.index,
                nodes,
                signature,
            };

            feed.put(msg.index, value, proof.clone()).await.unwrap();

            let i = msg.index;
            let node = feed.get(i).await.unwrap();
            if let Some(value) = node {
                println!("feed idx {}: {:?}", i, String::from_utf8(value).unwrap());
            } else {
                println!("feed idx {}: {:?}", i, "NONE");
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
                    channel.send(Message::Request(msg)).await?;
                }
            };
        }
        _ => {}
    };
    Ok(())
}
