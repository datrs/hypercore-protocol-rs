use async_std::future;
use blake2_rfc::blake2b::Blake2b;
use bytes::Bytes;
use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use futures::stream::TryStreamExt;
use async_trait::async_trait;
use hex;
use prost::Message as ProstMessage;
use simple_message_channels::{send, Message as ChannelMessage, Reader};
use std::collections::HashMap;
// use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::sync::Arc;
use std::time::Duration;
use varinteger;

use crate::handshake::HandshakeResult;
use crate::schema::*;

const CAP_NS_BUF: &[u8] = b"hypercore capability";
const TIMEOUT_SECS: u64 = 10;
const HYPERCORE: &[u8] = b"hypercore";

const DEBUG_KEY: &[u8] = b"01234567890123456789012345678901";

#[derive(Debug)]
pub enum Message {
    Open(Open),
    Options(Options),
    Status(Status),
    Have(Have),
    Unhave(Unhave),
    Want(Want),
    Unwant(Unwant),
    Request(Request),
    Cancel(Cancel),
    Data(Data),
    Close(Close),
    Extension(ExtensionMessage),
}

#[derive(Debug)]
pub struct ExtensionMessage {
    id: u64,
    message: Vec<u8>,
}
impl ExtensionMessage {
    fn _new(id: u64, message: Vec<u8>) -> Self {
        Self { id, message }
    }

    fn decode(buf: &[u8]) -> Result<Self> {
        let mut id: u64 = 0;
        let id_len = varinteger::decode(buf, &mut id);
        Ok(Self {
            id,
            message: buf[id_len..].to_vec(),
        })
    }

    fn encoded_len(&self) -> usize {
        let id_len = varinteger::length(self.id);
        id_len + self.message.len()
    }

    fn encode(&self, buf: &mut [u8]) {
        let id_len = varinteger::length(self.id);
        varinteger::encode(self.id, &mut buf[..id_len]);
        buf[id_len..].copy_from_slice(&self.message)
    }

    fn to_vec(&self) -> Vec<u8> {
        let mut buf = vec![0u8; self.encoded_len()];
        self.encode(&mut buf);
        buf.to_vec()
    }
}

impl Message {
    fn decode(typ: u8, buf: Vec<u8>) -> Result<Self> {
        let bytes = Bytes::from(buf);
        eprintln!("decode! typ {}", typ);
        match typ {
            0 => Ok(Self::Open(Open::decode(bytes)?)),
            1 => Ok(Self::Options(Options::decode(bytes)?)),
            2 => Ok(Self::Status(Status::decode(bytes)?)),
            3 => Ok(Self::Have(Have::decode(bytes)?)),
            4 => Ok(Self::Unhave(Unhave::decode(bytes)?)),
            5 => Ok(Self::Want(Want::decode(bytes)?)),
            6 => Ok(Self::Unwant(Unwant::decode(bytes)?)),
            7 => Ok(Self::Request(Request::decode(bytes)?)),
            8 => Ok(Self::Cancel(Cancel::decode(bytes)?)),
            9 => Ok(Self::Data(Data::decode(bytes)?)),
            10 => Ok(Self::Close(Close::decode(bytes)?)),
            15 => Ok(Self::Extension(ExtensionMessage::decode(&bytes)?)),
            _ => Err(Error::new(ErrorKind::InvalidData, "Invalid message type")),
        }
    }

    fn encode(&mut self, ch: u64) -> Result<ChannelMessage> {
        match self {
            Self::Open(msg) => Ok(ChannelMessage::new(ch, 0, encode_msg(msg)?)),
            Self::Options(msg) => Ok(ChannelMessage::new(ch, 1, encode_msg(msg)?)),
            Self::Status(msg) => Ok(ChannelMessage::new(ch, 2, encode_msg(msg)?)),
            Self::Have(msg) => Ok(ChannelMessage::new(ch, 3, encode_msg(msg)?)),
            Self::Unhave(msg) => Ok(ChannelMessage::new(ch, 4, encode_msg(msg)?)),
            Self::Want(msg) => Ok(ChannelMessage::new(ch, 5, encode_msg(msg)?)),
            Self::Unwant(msg) => Ok(ChannelMessage::new(ch, 6, encode_msg(msg)?)),
            Self::Request(msg) => Ok(ChannelMessage::new(ch, 7, encode_msg(msg)?)),
            Self::Cancel(msg) => Ok(ChannelMessage::new(ch, 8, encode_msg(msg)?)),
            Self::Data(msg) => Ok(ChannelMessage::new(ch, 9, encode_msg(msg)?)),
            Self::Close(msg) => Ok(ChannelMessage::new(ch, 10, encode_msg(msg)?)),
            Self::Extension(msg) => Ok(ChannelMessage::new(ch, 15, msg.to_vec())),
            // _ => Err(Error::new(ErrorKind::InvalidData, "Invalid message type")),
        }
    }
}

pub async fn handle_connection<R, W>(
    reader: R,
    writer: W,
    handshake: Option<HandshakeResult>,
) -> Result<()>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    // let handlers = None;
    let mut protocol = Protocol::new(reader, writer, handshake);
    protocol.listen().await?;
    Ok(())
}


#[derive(Clone)]
struct Channel {
    local_id: Option<usize>,
    remote_id: Option<usize>,
    discovery_key: Vec<u8>,
    key: Option<Vec<u8>>,
    handlers: Option<HandlerType>,
}

struct Channelizer {
    channels: HashMap<String, Channel>,
    local_id: Vec<Option<String>>,
    remote_id: Vec<Option<String>>,
}

impl Channelizer {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
            local_id: Vec::new(),
            remote_id: Vec::new(),
        }
    }

    fn alloc_local(&mut self) -> usize {
        let empty_id = self.local_id.iter().position(|x| x.is_none());
        match empty_id {
            Some(empty_id) => empty_id,
            None => {
                self.local_id.push(None);
                self.local_id.len() - 1
            }
        }
    }

    fn alloc_remote(&mut self, id: usize) {
        if self.remote_id.len() > id {
            self.remote_id[id] = None;
        } else {
            while self.remote_id.len() < id + 1 {
                self.remote_id.push(None);
            }
        }
    }

    pub fn get(&self, discovery_key: &[u8]) -> Option<&Channel> {
        let hdkey = hex::encode(discovery_key);
        self.channels.get(&hdkey)
    }

    pub fn get_remote(&self, id: usize) -> Option<&Channel> {
        match self.remote_id.get(id) {
            None => None,
            Some(None) => None,
            Some(Some(hdkey)) => self.channels.get(hdkey),
        }
    }

    pub fn _get_local(&self, id: usize) -> Option<&Channel> {
        match self.local_id.get(id) {
            None => None,
            Some(None) => None,
            Some(Some(hdkey)) => self.channels.get(hdkey),
        }
    }

    pub fn remove(&mut self, discovery_key: &[u8]) {
        let hdkey = hex::encode(discovery_key);
        let channel = self.channels.get(&hdkey);
        if let Some(channel) = channel {
            if let Some(local_id) = channel.local_id {
                self.local_id[local_id] = None;
            }
            if let Some(remote_id) = channel.remote_id {
                self.remote_id[remote_id] = None;
            }
        }
    }

    // pub fn by_key(&self, key: &[u8]) -> Option<&Channel> {
    //     let discovery_key = discovery_key(key);
    //     let hdkey = hex::encode(&discovery_key);
    //     self.channels.get(&hdkey)
    // }

    pub fn attach_local(&mut self, key: Vec<u8>, handlers: Option<HandlerType>) {
        let discovery_key = discovery_key(&key);
        let hdkey = hex::encode(&discovery_key);

        let local_id = self.alloc_local();
        self.local_id[local_id] = Some(hdkey.clone());

        if self.channels.contains_key(&hdkey) {
            let channel = self.channels.get_mut(&hdkey).unwrap();
            channel.local_id = Some(local_id);
            channel.key = Some(key);
        } else {
            let channel = Channel {
                key: Some(key),
                discovery_key: discovery_key,
                local_id: Some(local_id),
                remote_id: None,
                handlers
            };
            self.channels.insert(hdkey.clone(), channel);
        }
    }

    pub fn attach_remote(&mut self, discovery_key: Vec<u8>, remote_id: usize) {
        let hdkey = hex::encode(&discovery_key);

        self.alloc_remote(remote_id);
        self.remote_id[remote_id] = Some(hdkey.clone());

        if self.channels.contains_key(&hdkey) {
            let channel = self.channels.get_mut(&hdkey).unwrap();
            channel.remote_id = Some(remote_id);
        } else {
            let channel = Channel {
                key: None,
                discovery_key: discovery_key,
                local_id: None,
                remote_id: Some(remote_id),
                handlers: None,
            };
            self.channels.insert(hdkey.clone(), channel);
        }
    }

    // pub fn encode (message: Message) -> (ch,
}

#[async_trait]
pub trait Handlers: Sync {
    fn ondiscoverykey(&self, _discovery_key: &[u8]) -> Option<Vec<u8>> {
        Some(DEBUG_KEY.to_vec())
        // None
    }

    async fn onopen(&self, _protocol: &mut Proto, _discovery_key: &[u8]) -> Result<()> {
        Ok(())
    }

    fn onmessage(&self, _discovery_key: &[u8], _message: Message) {
    }

    // fn oninit(&self, protocol: &mut (dyn DynProtocol + Send)) {
    //     eprintln!("oninit handlers!");
    // }
}

pub type Proto = (dyn DynProtocol + Send);
pub type HandlerType = Arc<dyn Handlers + Send + Sync>;

struct DefaultHandlers {}
#[async_trait]
impl Handlers for DefaultHandlers {}

// type HandlerFn = FnMut() -> Future<Output = ()>;

#[async_trait]
pub trait DynProtocol {
    async fn send (&mut self, discovery_key: &[u8], message: Message) -> Result<()>;
}
#[async_trait]
impl <R,W>DynProtocol for Protocol<R,W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static
    {
    async fn send (&mut self, discovery_key: &[u8], message: Message) -> Result<()>{
        Protocol::send_channel(self, discovery_key, message).await
    }
}

#[async_trait]
impl <R,W>DynProtocol for &mut Protocol<R,W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static
    {
    async fn send (&mut self, discovery_key: &[u8], message: Message) -> Result<()> {
        self.send_channel(discovery_key, message).await
    }
}

pub struct Protocol<R, W> {
    // raw_reader: R,
    raw_writer: W,
    reader: Reader<R>,
    handshake: Option<HandshakeResult>,
    channels: Channelizer,
    handlers: HandlerType,
    // writer: Writer<W>,
    // is_initiator: bool
}

impl<R, W> Protocol<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static
{
    pub fn new(reader: R, writer: W, handshake: Option<HandshakeResult>) -> Self {
        let handlers = Arc::new(DefaultHandlers {});
        Protocol {
            raw_writer: writer,
            reader: Reader::new(reader),
            handshake,
            channels: Channelizer::new(),
            handlers
        }
    }
    pub fn set_handlers (&mut self, handlers: HandlerType) {
        self.handlers = handlers;
    }

    pub async fn listen(&mut self) -> Result<()> {
        loop {
            let timeout = Duration::from_secs(TIMEOUT_SECS);
            let next = self.reader.try_next();

            match future::timeout(timeout, next).await {
                Err(_timeout_err) => {
                    self.ping().await?;
                }
                Ok(Ok(Some(message))) => {
                    self.onmessage(message).await?;
                }
                Ok(Ok(None)) => {
                    return Err(Error::new(ErrorKind::UnexpectedEof, "connection closed"));
                }
                Ok(Err(e)) => {
                    return Err(e);
                }
            }
        }
    }

    async fn onmessage(&mut self, message: ChannelMessage) -> Result<()> {
        let ChannelMessage { typ, channel, message } = message;
        let message = Message::decode(typ, message)?;
        eprintln!("recv: {:?}", message);
        let _result = match message{
            Message::Open(msg) => self.onopen(channel, msg).await,
            Message::Close(msg) => self.onclose(channel, msg).await,
            Message::Extension(_msg) => unimplemented!(),
            _ => {
                let channel = self.channels.get_remote(channel as usize);
                match channel {
                    None => eprintln!("Message on closed channel"),
                    Some(channel) => {
                        if let Some(handlers) = channel.handlers.as_ref() {
                            handlers.onmessage(&channel.discovery_key, message);
                        } else {
                            self.handlers.onmessage(&channel.discovery_key, message);
                        }
                    }
                };
                Ok(())
            }
        };
        Ok(())
    }

    async fn open(&mut self, key: Vec<u8>) -> Result<()> {
        self.channels.attach_local(key.clone(), None);
        let discovery_key = discovery_key(&key);
        let capability = self.capability(&key);
        let message = Message::Open(Open {
            discovery_key: discovery_key.clone(),
            capability
        });
        self.send_channel(&discovery_key, message).await?;
        Ok(())
    }

    async fn onclose(&mut self, ch: u64, msg: Close) -> Result<()> {
        let ch = ch as usize;
        if let Some(discovery_key) = msg.discovery_key {
            self.channels.remove(&discovery_key.clone());
        } else if let Some(channel) = self.channels.get_remote(ch) {
            let discovery_key = channel.discovery_key.clone();
            self.channels.remove(&discovery_key);
        }
        Ok(())
    }

    async fn onopen(&mut self, ch: u64, msg: Open) -> Result<()> {
        let Open {
            discovery_key,
            capability,
        } = msg;

        eprintln!("onopen");

        // The remote is opening a channel for a discovery key. Let's see
        // if we know the pubkey for this channel.
        let pubkey = match self.channels.get(&discovery_key) {
            // Yep, we opened this channel locally.
            Some(Channel { key: Some(key), .. }) => Ok(key.to_vec()),
            // No key, but the channel is already opened - this only happens
            // if the remote already opened this channel. Exit out.
            Some(Channel { key: None, .. }) => {
                Err(Error::new(ErrorKind::AlreadyExists, "Channel already open"))
            }
            // No channel yet for this discovery key.
            None => {
                // Call out to our ondiscoverykey handler to see if the application
                // can provide the matching key.
                let key = self.handlers.ondiscoverykey(&discovery_key);
                match key {
                    // If not, well, we don't know the key, so exit and out.
                    // TODO: This shouldn't kill the stream.
                    None => Err(Error::new(ErrorKind::AddrNotAvailable, "Key not found")),
                    // Yay, we found a key! So let's open a channel from our side too.
                    Some(key) => {
                        self.open(key.clone()).await?;
                        Ok(key)
                    }
                }
            }
        };

        // Unwrap the result from above.
        let pubkey = pubkey?;

        // Verify the remote capability.
        self.verify_remote_capability(&capability, &pubkey)?;
        // Attach the channel for future use.
        self.channels
            .attach_remote(discovery_key.clone(), ch as usize);

        // self.channel
        // let handlers = self.handlers.clone();
        self.handlers.clone().onopen(&mut *self, &discovery_key).await?;
        // let handlers = Arc::get_mut(&mut handlers).unwrap();
        // Handlers::onopen(handlers, &mut *self, &discovery_key).await?;


        // if let Some(channel) = self.channels.get(&msg.discovery_key) {
        //     channel.attach_remote(ch)
        // }

        // let remote_capability = self.remote_capability(DEBUG_KEY.to_vec());
        // if remote_capability == msg.capability {
        //     eprintln!("remote verified!");
        // } else {
        //     eprintln!("bad bad remote!");
        // }
        // let capability = self.capability(DEBUG_KEY.to_vec());
        // self.send(
        //     ch,
        //     Message::Open(Open {
        //         discovery_key: msg.discovery_key,
        //         capability,
        //     }),
        // )
        // .await?;
        // self.send(
        //     ch,
        //     Message::Want(Want {
        //         start: 0,
        //         length: Some(100),
        //     }),
        // )
        // .await?;
        // self.send(
        //     ch,
        //     Message::Request(Request {
        //         index: 0,
        //         bytes: None,
        //         hash: None,
        //         nodes: None,
        //     }),
        // )
        // .await?;
        Ok(())
    }

    async fn send(&mut self, ch: u64, mut msg: Message) -> Result<()> {
        eprintln!("send {} {:?}", ch, msg);
        let encoded = msg.encode(ch)?;
        send(&mut self.raw_writer, encoded).await?;
        // self.writer.send(encoded).await?;
        Ok(())
    }

    async fn send_channel(&mut self, discovery_key: &[u8], msg: Message) -> Result<()> {
        let local_id = match self.channels.get(&discovery_key) {
            None | Some(Channel { local_id: None, .. }) => {
                return Err(Error::new(ErrorKind::BrokenPipe, "Channel is not open"))
            }
            Some(Channel {
                local_id: Some(local_id),
                ..
            }) => local_id.clone() as u64,
        };
        self.send(local_id, msg).await?;
        Ok(())
    }

    async fn ping(&mut self) -> Result<()> {
        let buf = vec![0u8];
        self.raw_writer.write_all(&buf).await?;
        self.raw_writer.flush().await?;
        Ok(())
    }

    fn capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        let handshake = match self.handshake.as_ref() {
            Some(h) => h,
            None => return None,
        };
        let mut context = Blake2b::with_key(32, &handshake.split_rx[..32]);
        context.update(CAP_NS_BUF);
        context.update(&handshake.split_tx[..32]);
        context.update(key);
        let hash = context.finalize();
        Some(hash.as_bytes().to_vec())
    }

    fn remote_capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        let handshake = match self.handshake.as_ref() {
            Some(h) => h,
            None => return None,
        };
        let mut context = Blake2b::with_key(32, &handshake.split_tx[..32]);
        context.update(CAP_NS_BUF);
        context.update(&handshake.split_rx[..32]);
        context.update(key);
        let hash = context.finalize();
        Some(hash.as_bytes().to_vec())
    }

    fn verify_remote_capability(&self, capability: &Option<Vec<u8>>, key: &[u8]) -> Result<()> {
        let capability = match capability {
            None => {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    "Did not receive remote capability",
                ));
            }
            Some(capability) => capability,
        };
        let expected_capability = self.remote_capability(key);
        let expected_capability = match expected_capability {
            None => {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    "Cannot verify capability",
                ));
            }
            Some(expected_capability) => expected_capability,
        };
        if expected_capability != *capability {
            Err(Error::new(
                ErrorKind::PermissionDenied,
                "Remote capability is invalid",
            ))
        } else {
            Ok(())
        }
    }
}

fn encode_msg(msg: &mut impl ProstMessage) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; msg.encoded_len()];
    msg.encode(&mut buf)?;
    Ok(buf)
}

pub fn discovery_key(key: &[u8]) -> Vec<u8> {
    let mut hasher = Blake2b::with_key(32, key);
    hasher.update(&HYPERCORE);
    hasher.finalize().as_bytes().to_vec()
}
