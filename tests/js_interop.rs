#![cfg(feature = "v10")]

use _util::wait_for_localhost_port;
use anyhow::Result;
use async_std::net::{TcpListener, TcpStream};
use async_std::prelude::*;
use async_std::sync::{Arc, Mutex};
use async_std::task;
use futures::Future;
use futures_lite::stream::StreamExt;
use hypercore::{
    Hypercore, PartialKeypair, PublicKey, RequestBlock, RequestUpgrade, SecretKey, Storage,
    PUBLIC_KEY_LENGTH, SECRET_KEY_LENGTH,
};
use instant::Duration;
use random_access_disk::RandomAccessDisk;
use random_access_storage::RandomAccess;
use std::fmt::Debug;
use std::path::Path;
use std::sync::Once;

use hypercore_protocol::schema::*;
use hypercore_protocol::{discovery_key, Channel, Event, Message, ProtocolBuilder};

pub mod _util;
mod js;
use js::{cleanup, install, js_run_client, js_start_server, prepare_test_set};

static INIT: Once = Once::new();
fn init() {
    INIT.call_once(|| {
        // run initialization here
        cleanup();
        install();
    });
}

const TEST_SET_NODE_CLIENT_NODE_SERVER: &str = "ncns";
const TEST_SET_RUST_CLIENT_NODE_SERVER: &str = "rcns";
const TEST_SET_NODE_CLIENT_RUST_SERVER: &str = "ncrs";
const TEST_SET_RUST_CLIENT_RUST_SERVER: &str = "rcrs";
const TEST_SET_SERVER_WRITER: &str = "sw";
const TEST_SET_CLIENT_WRITER: &str = "cw";
const TEST_SET_SIMPLE: &str = "simple";

#[async_std::test]
#[cfg_attr(not(feature = "js_interop_tests"), ignore)]
async fn js_interop_ncns_simple_server_writer() -> Result<()> {
    js_interop_ncns_simple(true, 8101).await?;
    Ok(())
}

#[async_std::test]
#[cfg_attr(not(feature = "js_interop_tests"), ignore)]
async fn js_interop_ncns_simple_client_writer() -> Result<()> {
    js_interop_ncns_simple(false, 8102).await?;
    Ok(())
}

#[async_std::test]
#[cfg_attr(not(feature = "js_interop_tests"), ignore)]
async fn js_interop_rcns_simple_server_writer() -> Result<()> {
    js_interop_rcns_simple(true, 8103).await?;
    Ok(())
}

#[async_std::test]
#[cfg_attr(not(feature = "js_interop_tests"), ignore)]
async fn js_interop_rcns_simple_client_writer() -> Result<()> {
    js_interop_rcns_simple(false, 8104).await?;
    Ok(())
}

#[async_std::test]
#[cfg_attr(not(feature = "js_interop_tests"), ignore)]
async fn js_interop_ncrs_simple_server_writer() -> Result<()> {
    js_interop_ncrs_simple(true, 8105).await?;
    Ok(())
}

#[async_std::test]
#[cfg_attr(not(feature = "js_interop_tests"), ignore)]
async fn js_interop_ncrs_simple_client_writer() -> Result<()> {
    js_interop_ncrs_simple(false, 8106).await?;
    Ok(())
}

#[async_std::test]
#[cfg_attr(not(feature = "js_interop_tests"), ignore)]
async fn js_interop_rcrs_simple_server_writer() -> Result<()> {
    js_interop_rcrs_simple(true, 8107).await?;
    Ok(())
}

#[async_std::test]
#[cfg_attr(not(feature = "js_interop_tests"), ignore)]
async fn js_interop_rcrs_simple_client_writer() -> Result<()> {
    js_interop_rcrs_simple(false, 8108).await?;
    Ok(())
}

async fn js_interop_ncns_simple(server_writer: bool, port: u32) -> Result<()> {
    init();
    let test_set = format!(
        "{}_{}_{}",
        TEST_SET_NODE_CLIENT_NODE_SERVER,
        if server_writer {
            TEST_SET_SERVER_WRITER
        } else {
            TEST_SET_CLIENT_WRITER
        },
        TEST_SET_SIMPLE
    );
    let (result_path, _writer_path, _reader_path) = prepare_test_set(&test_set);
    let item_count = 4;
    let item_size = 4;
    let data_char = '1';
    let _server = js_start_server(
        server_writer,
        port,
        item_count,
        item_size,
        data_char,
        test_set.clone(),
    )
    .await?;
    js_run_client(
        !server_writer,
        port,
        item_count,
        item_size,
        data_char,
        &test_set,
    )
    .await;
    assert_result(result_path, item_count, item_size, data_char).await?;

    Ok(())
}

async fn js_interop_rcns_simple(server_writer: bool, port: u32) -> Result<()> {
    init();
    let test_set = format!(
        "{}_{}_{}",
        TEST_SET_RUST_CLIENT_NODE_SERVER,
        if server_writer {
            TEST_SET_SERVER_WRITER
        } else {
            TEST_SET_CLIENT_WRITER
        },
        TEST_SET_SIMPLE
    );
    let (result_path, writer_path, reader_path) = prepare_test_set(&test_set);
    let item_count = 4;
    let item_size = 4;
    let data_char = '1';
    let _server = js_start_server(
        server_writer,
        port,
        item_count,
        item_size,
        data_char,
        test_set.clone(),
    )
    .await?;
    run_client(
        !server_writer,
        port,
        item_count,
        item_size,
        data_char,
        if server_writer {
            &reader_path
        } else {
            &writer_path
        },
        &result_path,
    )
    .await?;
    assert_result(result_path, item_count, item_size, data_char).await?;

    Ok(())
}

async fn js_interop_ncrs_simple(server_writer: bool, port: u32) -> Result<()> {
    init();
    let test_set = format!(
        "{}_{}_{}",
        TEST_SET_NODE_CLIENT_RUST_SERVER,
        if server_writer {
            TEST_SET_SERVER_WRITER
        } else {
            TEST_SET_CLIENT_WRITER
        },
        TEST_SET_SIMPLE
    );
    let (result_path, writer_path, reader_path) = prepare_test_set(&test_set);
    let item_count = 4;
    let item_size = 4;
    let data_char = '1';

    let _server = start_server(
        server_writer,
        port,
        item_count,
        item_size,
        data_char,
        if server_writer {
            &writer_path
        } else {
            &reader_path
        },
        &result_path,
    )
    .await?;
    js_run_client(
        !server_writer,
        port,
        item_count,
        item_size,
        data_char,
        &test_set.clone(),
    )
    .await;

    assert_result(result_path, item_count, item_size, data_char).await?;

    Ok(())
}

async fn js_interop_rcrs_simple(server_writer: bool, port: u32) -> Result<()> {
    init();
    let test_set = format!(
        "{}_{}_{}",
        TEST_SET_RUST_CLIENT_RUST_SERVER,
        if server_writer {
            TEST_SET_SERVER_WRITER
        } else {
            TEST_SET_CLIENT_WRITER
        },
        TEST_SET_SIMPLE
    );
    let (result_path, writer_path, reader_path) = prepare_test_set(&test_set);
    let item_count = 4;
    let item_size = 4;
    let data_char = '1';

    let _server = start_server(
        server_writer,
        port,
        item_count,
        item_size,
        data_char,
        if server_writer {
            &writer_path
        } else {
            &reader_path
        },
        &result_path,
    )
    .await?;
    run_client(
        !server_writer,
        port,
        item_count,
        item_size,
        data_char,
        if server_writer {
            &reader_path
        } else {
            &writer_path
        },
        &result_path,
    )
    .await?;

    assert_result(result_path, item_count, item_size, data_char).await?;

    Ok(())
}

async fn assert_result(
    result_path: String,
    item_count: usize,
    item_size: usize,
    data_char: char,
) -> Result<()> {
    let mut reader = async_std::io::BufReader::new(async_std::fs::File::open(result_path).await?);

    let mut i: usize = 0;
    let expected_value = data_char.to_string().repeat(item_size);
    let mut line = String::new();
    while reader.read_line(&mut line).await? != 0 {
        assert_eq!(line, format!("{} {}\n", i, expected_value));
        i += 1;
        line = String::new();
    }
    assert_eq!(i, item_count);
    Ok(())
}

async fn run_client(
    is_writer: bool,
    port: u32,
    data_count: usize,
    data_size: usize,
    data_char: char,
    data_path: &str,
    result_path: &str,
) -> Result<()> {
    let hypercore = if is_writer {
        create_writer_hypercore(data_count, data_size, data_char, data_path).await?
    } else {
        create_reader_hypercore(data_path).await?
    };
    let hypercore_wrapper = HypercoreWrapper::from_disk_hypercore(
        hypercore,
        if is_writer {
            None
        } else {
            Some(result_path.to_string())
        },
    );
    tcp_client(port, on_replication_connection, Arc::new(hypercore_wrapper)).await?;
    Ok(())
}

async fn start_server(
    is_writer: bool,
    port: u32,
    item_count: usize,
    item_size: usize,
    data_char: char,
    data_path: &str,
    result_path: &str,
) -> Result<RustServer> {
    let hypercore = if is_writer {
        create_writer_hypercore(item_count, item_size, data_char, &data_path).await?
    } else {
        create_reader_hypercore(&data_path).await?
    };
    let hypercore_wrapper = HypercoreWrapper::from_disk_hypercore(
        hypercore,
        if is_writer {
            None
        } else {
            Some(result_path.to_string())
        },
    );
    let mut server = RustServer::new();
    server.run(Arc::new(hypercore_wrapper), port).await;
    Ok(server)
}

async fn create_writer_hypercore(
    data_count: usize,
    data_size: usize,
    data_char: char,
    path: &str,
) -> Result<Hypercore<RandomAccessDisk>> {
    let path = Path::new(path).to_owned();
    let key_pair = get_test_key_pair(true);
    let storage = Storage::new_disk(&path, false).await?;
    let mut hypercore = Hypercore::new_with_key_pair(storage, key_pair).await?;
    for _ in 0..data_count {
        let value = vec![data_char as u8; data_size];
        hypercore.append(&value).await?;
    }
    Ok(hypercore)
}

async fn create_reader_hypercore(path: &str) -> Result<Hypercore<RandomAccessDisk>> {
    let path = Path::new(path).to_owned();
    let key_pair = get_test_key_pair(false);
    let storage = Storage::new_disk(&path, false).await?;
    Ok(Hypercore::new_with_key_pair(storage, key_pair).await?)
}

const TEST_PUBLIC_KEY_BYTES: [u8; PUBLIC_KEY_LENGTH] = [
    0x97, 0x60, 0x6c, 0xaa, 0xd2, 0xb0, 0x8c, 0x1d, 0x5f, 0xe1, 0x64, 0x2e, 0xee, 0xa5, 0x62, 0xcb,
    0x91, 0xd6, 0x55, 0xe2, 0x00, 0xc8, 0xd4, 0x3a, 0x32, 0x09, 0x1d, 0x06, 0x4a, 0x33, 0x1e, 0xe3,
];
// NB: In the javascript version this is 64 bytes, but that's because sodium appends the the public
// key after the secret key for some reason. Only the first 32 bytes are actually used in
// javascript side too for signing.
const TEST_SECRET_KEY_BYTES: [u8; SECRET_KEY_LENGTH] = [
    0x27, 0xe6, 0x74, 0x25, 0xc1, 0xff, 0xd1, 0xd9, 0xee, 0x62, 0x5c, 0x96, 0x2b, 0x57, 0x13, 0xc3,
    0x51, 0x0b, 0x71, 0x14, 0x15, 0xf3, 0x31, 0xf6, 0xfa, 0x9e, 0xf2, 0xbf, 0x23, 0x5f, 0x2f, 0xfe,
];

pub fn get_test_key_pair(include_secret: bool) -> PartialKeypair {
    let public = PublicKey::from_bytes(&TEST_PUBLIC_KEY_BYTES).unwrap();
    let secret = if include_secret {
        Some(SecretKey::from_bytes(&TEST_SECRET_KEY_BYTES).unwrap())
    } else {
        None
    };
    PartialKeypair { public, secret }
}

async fn on_replication_connection<T: 'static>(
    stream: TcpStream,
    is_initiator: bool,
    hypercore: Arc<HypercoreWrapper<T>>,
) -> Result<()>
where
    T: RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>> + Debug + Send,
{
    let mut protocol = ProtocolBuilder::new(is_initiator).connect(stream);
    while let Some(event) = protocol.next().await {
        let event = event?;
        match event {
            Event::Handshake(_) => {
                if is_initiator {
                    protocol.open(hypercore.key().clone()).await?;
                }
            }
            Event::DiscoveryKey(dkey) => {
                if hypercore.discovery_key == dkey {
                    protocol.open(hypercore.key().clone()).await?;
                } else {
                    panic!("Invalid discovery key");
                }
            }
            Event::Channel(channel) => {
                hypercore.on_replication_peer(channel);
            }
            Event::Close(_dkey) => {
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct HypercoreWrapper<T>
where
    T: RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>> + Debug + Send,
{
    discovery_key: [u8; 32],
    key: [u8; 32],
    hypercore: Arc<Mutex<Hypercore<T>>>,
    result_path: Option<String>,
}

impl HypercoreWrapper<RandomAccessDisk> {
    pub fn from_disk_hypercore(
        hypercore: Hypercore<RandomAccessDisk>,
        result_path: Option<String>,
    ) -> Self {
        let key = hypercore.key_pair().public.to_bytes();
        HypercoreWrapper {
            key,
            discovery_key: discovery_key(&key),
            hypercore: Arc::new(Mutex::new(hypercore)),
            result_path,
        }
    }
}

impl<T> HypercoreWrapper<T>
where
    T: RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>> + Debug + Send + 'static,
{
    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }

    pub fn on_replication_peer(&self, mut channel: Channel) {
        let mut peer_state = PeerState::default();
        let mut hypercore = self.hypercore.clone();
        let result_path = self.result_path.clone();
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
                let ready = on_replication_message(
                    &mut hypercore,
                    &mut peer_state,
                    result_path.clone(),
                    &mut channel,
                    message,
                )
                .await
                .expect("on_replication_message should return Ok");
                if ready {
                    channel.close().await.expect("Should be able to close");
                    break;
                }
            }
        });
    }
}

async fn on_replication_message<T>(
    hypercore: &mut Arc<Mutex<Hypercore<T>>>,
    peer_state: &mut PeerState,
    result_path: Option<String>,
    channel: &mut Channel,
    message: Message,
) -> Result<bool>
where
    T: RandomAccess<Error = Box<dyn std::error::Error + Send + Sync>> + Debug + Send,
{
    match message {
        Message::Synchronize(message) => {
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
                    id: 1,
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
            } else {
                panic!("Could not create proof from {:?}", message.id);
            }
        }
        Message::Data(message) => {
            let (old_info, applied, new_info, synced) = {
                let mut hypercore = hypercore.lock().await;
                let old_info = hypercore.info();
                let proof = message.clone().into_proof();
                let applied = hypercore.verify_and_apply_proof(&proof).await?;
                let new_info = hypercore.info();
                let mut lines: Vec<String> = vec![];
                let synced = new_info.contiguous_length == new_info.length;
                (old_info, applied, new_info, synced)
            };
            assert!(applied, "Could not apply proof");
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

                for i in old_info.length..new_length {
                    messages.push(Message::Request(Request {
                        id: i + 1,
                        fork: new_info.fork,
                        hash: None,
                        block: Some(RequestBlock { index: i, nodes: 2 }),
                        seek: None,
                        upgrade: None,
                    }));
                }
            }
            if let Some(block) = &message.block {
                // Send Range if the number of items changed, both for the single and
                // for the contiguous length
                if old_info.length < new_info.length {
                    messages.push(Message::Range(Range {
                        drop: false,
                        start: block.index,
                        length: 1,
                    }));
                }
                if old_info.contiguous_length < new_info.contiguous_length {
                    messages.push(Message::Range(Range {
                        drop: false,
                        start: 0,
                        length: new_info.contiguous_length,
                    }));
                }
            }
            let exit = if synced {
                if let Some(result_path) = result_path.as_ref() {
                    let mut hypercore = hypercore.lock().await;
                    let mut writer = async_std::io::BufWriter::new(
                        async_std::fs::File::create(result_path).await?,
                    );
                    for i in 0..new_info.contiguous_length {
                        let value = String::from_utf8(hypercore.get(i).await?.unwrap()).unwrap();
                        let line = format!("{} {}\n", i, value);
                        writer.write(line.as_bytes()).await?;
                    }
                    writer.flush().await?;
                    true
                } else {
                    false
                }
            } else {
                false
            };
            channel.send_batch(&messages).await.unwrap();
            if exit {
                return Ok(true);
            }
        }
        Message::Range(message) => {
            if result_path.is_none() {
                let info = {
                    let hypercore = hypercore.lock().await;
                    hypercore.info()
                };
                if message.start == 0 && message.length == info.contiguous_length {
                    // We need to wait here for the other side to finish writing the file,
                    // which in node's case could be a while.
                    async_std::task::sleep(Duration::from_millis(100)).await;
                    return Ok(true);
                }
            }
        }
        _ => {
            panic!("Received unexpected message {:?}", message);
        }
    };
    Ok(false)
}

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

pub struct RustServer {
    handle: Option<async_std::task::JoinHandle<()>>,
}

impl RustServer {
    pub fn new() -> RustServer {
        RustServer { handle: None }
    }

    pub async fn run(&mut self, hypercore: Arc<HypercoreWrapper<RandomAccessDisk>>, port: u32) {
        self.handle = Some(async_std::task::spawn(async move {
            tcp_server(port, on_replication_connection, hypercore)
                .await
                .expect("Server return ok");
        }));
        wait_for_localhost_port(port).await;
    }
}

impl Drop for RustServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            async_std::task::block_on(handle.cancel());
        }
    }
}

pub async fn tcp_server<F, C>(
    port: u32,
    onconnection: impl Fn(TcpStream, bool, C) -> F + Send + Sync + Copy + 'static,
    context: C,
) -> Result<()>
where
    F: Future<Output = Result<()>> + Send,
    C: Clone + Send + 'static,
{
    let listener = TcpListener::bind(&format!("localhost:{}", port)).await?;
    let mut incoming = listener.incoming();
    while let Some(Ok(stream)) = incoming.next().await {
        let context = context.clone();
        let _peer_addr = stream.peer_addr().unwrap();
        task::spawn(async move {
            onconnection(stream, false, context)
                .await
                .expect("Should return ok");
        });
    }
    Ok(())
}

pub async fn tcp_client<F, C>(
    port: u32,
    onconnection: impl Fn(TcpStream, bool, C) -> F + Send + Sync + Copy + 'static,
    context: C,
) -> Result<()>
where
    F: Future<Output = Result<()>> + Send,
    C: Clone + Send + 'static,
{
    let stream = TcpStream::connect(&format!("localhost:{}", port)).await?;
    onconnection(stream, true, context).await
}
