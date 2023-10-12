use anyhow::Result;
use async_std::net::{TcpListener, TcpStream};
use async_std::prelude::*;
use async_std::sync::{Arc, Mutex};
use async_std::task;
use env_logger::Env;
use futures_lite::stream::StreamExt;
use hypercore::{
    Hypercore, HypercoreBuilder, PartialKeypair, RequestBlock, RequestUpgrade, Storage,
    VerifyingKey,
};
use log::*;
use random_access_memory::RandomAccessMemory;
use random_access_storage::RandomAccess;
use std::collections::HashMap;
use std::convert::TryInto;
use std::env;
use std::fmt::Debug;

use hypercore_protocol::schema::*;
use hypercore_protocol::{discovery_key, Channel, Event, Message, ProtocolBuilder};

fn main() {
    init_logger();
    if env::args().count() < 3 {
        usage();
    }
    let mode = env::args().nth(1).unwrap();
    let port = env::args().nth(2).unwrap();
    let address = format!("127.0.0.1:{port}");

    let key = env::args().nth(3);
    let key: Option<[u8; 32]> = key.map(|key| {
        hex::decode(key)
            .expect("Key has to be a hex string")
            .try_into()
            .expect("Key has to be a 32 byte hex string")
    });

    task::block_on(async move {
        let mut hypercore_store: HypercoreStore<RandomAccessMemory> = HypercoreStore::new();
        let storage = Storage::new_memory().await.unwrap();
        // Create a hypercore.
        let hypercore = if let Some(key) = key {
            let public_key = VerifyingKey::from_bytes(&key).unwrap();
            HypercoreBuilder::new(storage)
                .key_pair(PartialKeypair {
                    public: public_key,
                    secret: None,
                })
                .build()
                .await
                .unwrap()
        } else {
            let mut hypercore = HypercoreBuilder::new(storage).build().await.unwrap();
            let batch: &[&[u8]] = &[b"hi\n", b"ola\n", b"hello\n", b"mundo\n"];
            hypercore.append_batch(batch).await.unwrap();
            hypercore
        };
        println!(
            "KEY={}",
            hex::encode(hypercore.key_pair().public.as_bytes())
        );
        info!("{} opened hypercore", mode);
        // Wrap it and add to the hypercore store.
        let hypercore_wrapper = HypercoreWrapper::from_memory_hypercore(hypercore);
        hypercore_store.add(hypercore_wrapper);
        let hypercore_store = Arc::new(hypercore_store);

        let result = match mode.as_ref() {
            "server" => tcp_server(address, onconnection, hypercore_store).await,
            "client" => tcp_client(address, onconnection, hypercore_store).await,
            _ => panic!("{:?}", usage()),
        };
        log_if_error(&result);
    });
}

/// Print usage and exit.
fn usage() {
    println!("usage: cargo run --example hypercore -- [client|server] [port] [key]");
    std::process::exit(1);
}

// The onconnection handler is called for each incoming connection (if server)
// or once when connected (if client).
// Unfortunately, everything that touches the hypercore_store or a hypercore has to be generic
// at the moment.
async fn onconnection<T: 'static>(
    stream: TcpStream,
    is_initiator: bool,
    hypercore_store: Arc<HypercoreStore<T>>,
) -> Result<()>
where
    T: RandomAccess + Debug + Send,
{
    info!("onconnection, initiator: {}", is_initiator);
    let mut protocol = ProtocolBuilder::new(is_initiator).connect(stream);
    info!("protocol created, polling for next()");
    while let Some(event) = protocol.next().await {
        let event = event?;
        info!("protocol event {:?}", event);
        match event {
            Event::Handshake(_) => {
                if is_initiator {
                    for hypercore in hypercore_store.hypercores.values() {
                        protocol.open(*hypercore.key()).await?;
                    }
                }
            }
            Event::DiscoveryKey(dkey) => {
                if let Some(hypercore) = hypercore_store.get(&dkey) {
                    protocol.open(*hypercore.key()).await?;
                }
            }
            Event::Channel(channel) => {
                if let Some(hypercore) = hypercore_store.get(channel.discovery_key()) {
                    hypercore.onpeer(channel);
                }
            }
            Event::Close(_dkey) => {}
            _ => {}
        }
    }
    Ok(())
}

/// A container for hypercores.
#[derive(Debug)]
struct HypercoreStore<T>
where
    T: RandomAccess + Debug + Send,
{
    hypercores: HashMap<String, Arc<HypercoreWrapper<T>>>,
}
impl<T> HypercoreStore<T>
where
    T: RandomAccess + Debug + Send,
{
    pub fn new() -> Self {
        let hypercores = HashMap::new();
        Self { hypercores }
    }

    pub fn add(&mut self, hypercore: HypercoreWrapper<T>) {
        let hdkey = hex::encode(hypercore.discovery_key);
        self.hypercores.insert(hdkey, Arc::new(hypercore));
    }

    pub fn get(&self, discovery_key: &[u8; 32]) -> Option<&Arc<HypercoreWrapper<T>>> {
        let hdkey = hex::encode(discovery_key);
        self.hypercores.get(&hdkey)
    }
}

/// A Hypercore is a single unit of replication, an append-only log.
#[derive(Debug, Clone)]
struct HypercoreWrapper<T>
where
    T: RandomAccess + Debug + Send,
{
    discovery_key: [u8; 32],
    key: [u8; 32],
    hypercore: Arc<Mutex<Hypercore<T>>>,
}

impl HypercoreWrapper<RandomAccessMemory> {
    pub fn from_memory_hypercore(hypercore: Hypercore<RandomAccessMemory>) -> Self {
        let key = hypercore.key_pair().public.to_bytes();
        HypercoreWrapper {
            key,
            discovery_key: discovery_key(&key),
            hypercore: Arc::new(Mutex::new(hypercore)),
        }
    }
}

impl<T> HypercoreWrapper<T>
where
    T: RandomAccess + Debug + Send + 'static,
{
    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }

    pub fn onpeer(&self, mut channel: Channel) {
        let mut peer_state = PeerState::default();
        let mut hypercore = self.hypercore.clone();
        task::spawn(async move {
            let info = {
                let hypercore = hypercore.lock().await;
                hypercore.info()
            };

            if info.fork != peer_state.remote_fork {
                peer_state.can_upgrade = false;
            }
            let remote_length = if info.fork == peer_state.remote_fork {
                peer_state.remote_length
            } else {
                0
            };

            let sync_msg = Synchronize {
                fork: info.fork,
                length: info.length,
                remote_length,
                can_upgrade: peer_state.can_upgrade,
                uploading: true,
                downloading: true,
            };

            if info.contiguous_length > 0 {
                let range_msg = Range {
                    drop: false,
                    start: 0,
                    length: info.contiguous_length,
                };
                channel
                    .send_batch(&[Message::Synchronize(sync_msg), Message::Range(range_msg)])
                    .await
                    .unwrap();
            } else {
                channel.send(Message::Synchronize(sync_msg)).await.unwrap();
            }
            while let Some(message) = channel.next().await {
                let result =
                    onmessage(&mut hypercore, &mut peer_state, &mut channel, message).await;
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
    can_upgrade: bool,
    remote_fork: u64,
    remote_length: u64,
    remote_can_upgrade: bool,
    remote_uploading: bool,
    remote_downloading: bool,
    remote_synced: bool,
    length_acked: u64,
}
impl Default for PeerState {
    fn default() -> Self {
        PeerState {
            can_upgrade: true,
            remote_fork: 0,
            remote_length: 0,
            remote_can_upgrade: false,
            remote_uploading: true,
            remote_downloading: true,
            remote_synced: false,
            length_acked: 0,
        }
    }
}

async fn onmessage<T>(
    hypercore: &mut Arc<Mutex<Hypercore<T>>>,
    peer_state: &mut PeerState,
    channel: &mut Channel,
    message: Message,
) -> Result<()>
where
    T: RandomAccess + Debug + Send,
{
    match message {
        Message::Synchronize(message) => {
            println!("Got Synchronize message {message:?}");
            let length_changed = message.length != peer_state.remote_length;
            let first_sync = !peer_state.remote_synced;
            let info = {
                let hypercore = hypercore.lock().await;
                hypercore.info()
            };
            let same_fork = message.fork == info.fork;

            peer_state.remote_fork = message.fork;
            peer_state.remote_length = message.length;
            peer_state.remote_can_upgrade = message.can_upgrade;
            peer_state.remote_uploading = message.uploading;
            peer_state.remote_downloading = message.downloading;
            peer_state.remote_synced = true;

            peer_state.length_acked = if same_fork { message.remote_length } else { 0 };

            let mut messages = vec![];

            if first_sync {
                // Need to send another sync back that acknowledges the received sync
                let msg = Synchronize {
                    fork: info.fork,
                    length: info.length,
                    remote_length: peer_state.remote_length,
                    can_upgrade: peer_state.can_upgrade,
                    uploading: true,
                    downloading: true,
                };
                messages.push(Message::Synchronize(msg));
            }

            if peer_state.remote_length > info.length
                && peer_state.length_acked == info.length
                && length_changed
            {
                let msg = Request {
                    id: 1, // There should be proper handling for in-flight request ids
                    fork: info.fork,
                    hash: None,
                    block: None,
                    seek: None,
                    upgrade: Some(RequestUpgrade {
                        start: info.length,
                        length: peer_state.remote_length - info.length,
                    }),
                };
                messages.push(Message::Request(msg));
            }
            channel.send_batch(&messages).await?;
        }
        Message::Request(message) => {
            println!("Got Request message {message:?}");
            let (info, proof) = {
                let mut hypercore = hypercore.lock().await;
                let proof = hypercore
                    .create_proof(message.block, message.hash, message.seek, message.upgrade)
                    .await?;
                (hypercore.info(), proof)
            };
            if let Some(proof) = proof {
                let msg = Data {
                    request: message.id,
                    fork: info.fork,
                    hash: proof.hash,
                    block: proof.block,
                    seek: proof.seek,
                    upgrade: proof.upgrade,
                };
                channel.send(Message::Data(msg)).await?;
            }
        }
        Message::Data(message) => {
            println!("Got Data message {message:?}");
            let (_old_info, _applied, new_info, request_block) = {
                let mut hypercore = hypercore.lock().await;
                let old_info = hypercore.info();
                let proof = message.clone().into_proof();
                let applied = hypercore.verify_and_apply_proof(&proof).await?;
                let new_info = hypercore.info();
                let request_block: Option<RequestBlock> = if let Some(upgrade) = &message.upgrade {
                    // When getting the initial upgrade, send a request for the first missing block
                    if old_info.length < upgrade.length {
                        let request_index = old_info.length;
                        let nodes = hypercore.missing_nodes(request_index).await?;
                        Some(RequestBlock {
                            index: request_index,
                            nodes,
                        })
                    } else {
                        None
                    }
                } else if let Some(block) = &message.block {
                    // When receiving a block, ask for the next, if there are still some missing
                    if block.index < peer_state.remote_length - 1 {
                        let request_index = block.index + 1;
                        let nodes = hypercore.missing_nodes(request_index).await?;
                        Some(RequestBlock {
                            index: request_index,
                            nodes,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                // If all have been replicated, print the result
                if new_info.contiguous_length == new_info.length {
                    println!();
                    println!("### Results");
                    println!();
                    println!("Replication succeeded if this prints '0: hi', '1: ola', '2: hello' and '3: mundo':");
                    println!();
                    for i in 0..new_info.contiguous_length {
                        println!(
                            "{}: {}",
                            i,
                            String::from_utf8(hypercore.get(i).await?.unwrap()).unwrap()
                        );
                    }
                    println!("Press Ctrl-C to exit");
                }
                (old_info, applied, new_info, request_block)
            };

            let mut messages: Vec<Message> = vec![];
            if let Some(upgrade) = &message.upgrade {
                let new_length = upgrade.length;
                let remote_length = if new_info.fork == peer_state.remote_fork {
                    peer_state.remote_length
                } else {
                    0
                };
                messages.push(Message::Synchronize(Synchronize {
                    fork: new_info.fork,
                    length: new_length,
                    remote_length,
                    can_upgrade: false,
                    uploading: true,
                    downloading: true,
                }));
            }
            if let Some(request_block) = request_block {
                messages.push(Message::Request(Request {
                    id: request_block.index + 1,
                    fork: new_info.fork,
                    hash: None,
                    block: Some(request_block),
                    seek: None,
                    upgrade: None,
                }));
            }
            channel.send_batch(&messages).await.unwrap();
        }
        _ => {}
    };
    Ok(())
}

/// Init EnvLogger, logging info, warn and error messages to stdout.
pub fn init_logger() {
    env_logger::from_env(Env::default().default_filter_or("info")).init();
}

/// Log a result if it's an error.
pub fn log_if_error(result: &Result<()>) {
    if let Err(err) = result.as_ref() {
        log::error!("error: {}", err);
    }
}

/// A simple async TCP server that calls an async function for each incoming connection.
pub async fn tcp_server<F, C>(
    address: String,
    onconnection: impl Fn(TcpStream, bool, C) -> F + Send + Sync + Copy + 'static,
    context: C,
) -> Result<()>
where
    F: Future<Output = Result<()>> + Send,
    C: Clone + Send + 'static,
{
    let listener = TcpListener::bind(&address).await?;
    log::info!("listening on {}", listener.local_addr()?);
    let mut incoming = listener.incoming();
    while let Some(Ok(stream)) = incoming.next().await {
        let context = context.clone();
        let peer_addr = stream.peer_addr().unwrap();
        log::info!("new connection from {}", peer_addr);
        task::spawn(async move {
            let result = onconnection(stream, false, context).await;
            log_if_error(&result);
            log::info!("connection closed from {}", peer_addr);
        });
    }
    Ok(())
}

/// A simple async TCP client that calls an async function when connected.
pub async fn tcp_client<F, C>(
    address: String,
    onconnection: impl Fn(TcpStream, bool, C) -> F + Send + Sync + Copy + 'static,
    context: C,
) -> Result<()>
where
    F: Future<Output = Result<()>> + Send,
    C: Clone + Send + 'static,
{
    log::info!("attempting connection to {address}");
    let stream = TcpStream::connect(&address).await?;
    log::info!("connected to {address}");
    onconnection(stream, true, context).await
}
