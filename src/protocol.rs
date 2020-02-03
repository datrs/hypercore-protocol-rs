use async_std::future;
use async_std::io::{BufReader, BufWriter};
use async_trait::async_trait;
use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use futures::stream::TryStreamExt;
use simple_message_channels::{send, Message as ChannelMessage, Reader};
use std::io::{Error, ErrorKind, Result};
use std::sync::Arc;
use std::time::Duration;

use crate::channels::{Channel, Channelizer};
use crate::handshake::{handshake, HandshakeResult};
use crate::message::Message;
use crate::schema::*;
use crate::util::discovery_key;

use crate::constants::{DEFAULT_KEEPALIVE, DEFAULT_TIMEOUT};

pub struct Protocol<R, W> {
    raw_writer: BufWriter<W>,
    reader: Reader<BufReader<R>>,
    handshake: Option<HandshakeResult>,
    channels: Channelizer,
    handlers: HandlerType,
}

impl<R, W> Protocol<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    pub fn new(
        reader: BufReader<R>,
        writer: BufWriter<W>,
        handshake: Option<HandshakeResult>,
    ) -> Self {
        let handlers = Arc::new(DefaultHandlers {});
        Protocol {
            raw_writer: writer,
            reader: Reader::new(reader),
            handshake,
            channels: Channelizer::new(),
            handlers,
        }
    }

    pub async fn from_stream_with_handshake<S>(
        stream: S,
        is_initiator: bool,
    ) -> Result<Protocol<S, S>>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + Clone + 'static,
    {
        let reader = stream.clone();
        let writer = stream.clone();
        Protocol::from_rw_with_handshake(reader, writer, is_initiator).await
    }

    pub async fn from_rw_with_handshake(
        reader: R,
        writer: W,
        is_initiator: bool,
    ) -> Result<Protocol<R, W>> {
        let reader = BufReader::new(reader);
        let writer = BufWriter::new(writer);
        Protocol::from_buf_rw_with_handshake(reader, writer, is_initiator).await
    }

    pub async fn from_buf_rw_with_handshake(
        reader: BufReader<R>,
        writer: BufWriter<W>,
        is_initiator: bool,
    ) -> Result<Protocol<R, W>> {
        let (reader, writer, handshake) = handshake(reader, writer, is_initiator).await?;
        Ok(Protocol::new(reader, writer, Some(handshake)))
    }

    pub fn set_handlers(&mut self, handlers: HandlerType) {
        self.handlers = handlers;
    }

    pub async fn listen(&mut self) -> Result<()> {
        loop {
            // TODO: Implement timeout.
            let _timeout = Duration::from_secs(DEFAULT_TIMEOUT as u64);
            let keepalive = Duration::from_secs(DEFAULT_KEEPALIVE as u64);

            let next = self.reader.try_next();

            match future::timeout(keepalive, next).await {
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
        let ChannelMessage {
            typ,
            channel,
            message,
        } = message;
        let message = Message::decode(typ, message)?;
        eprintln!("recv: {:?}", message);
        let _result = match message {
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
            capability,
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
            // No key, but the channel is already present - this only happens
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
        self.verify_remote_capability(capability, &pubkey)?;
        // Attach the channel for future use.
        self.channels
            .attach_remote(discovery_key.clone(), ch as usize);

        // Call the stream-level handler.
        self.handlers
            .clone()
            .onopen(&mut *self, &discovery_key)
            .await?;

        Ok(())
    }

    async fn send(&mut self, ch: u64, mut msg: Message) -> Result<()> {
        eprintln!("send {} {:?}", ch, msg);
        let encoded = msg.encode(ch)?;
        send(&mut self.raw_writer, encoded).await?;
        Ok(())
    }

    async fn send_channel(&mut self, discovery_key: &[u8], msg: Message) -> Result<()> {
        match self.channels.get_local_id(&discovery_key) {
            None => Err(Error::new(ErrorKind::BrokenPipe, "Channel is not open")),
            Some(local_id) => self.send(local_id as u64, msg).await,
        }
    }

    async fn ping(&mut self) -> Result<()> {
        let buf = vec![0u8];
        self.raw_writer.write_all(&buf).await?;
        self.raw_writer.flush().await?;
        Ok(())
    }

    fn capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        match self.handshake.as_ref() {
            Some(handshake) => handshake.capability(key),
            None => None,
        }
    }

    fn verify_remote_capability(&self, capability: Option<Vec<u8>>, key: &[u8]) -> Result<()> {
        match self.handshake.as_ref() {
            Some(handshake) => handshake.verify_remote_capability(capability, key),
            None => Err(Error::new(
                ErrorKind::PermissionDenied,
                "Missing handshake state for capability verification",
            )),
        }
    }
}

struct DefaultHandlers {}

#[async_trait]
impl Handlers for DefaultHandlers {}

pub type Proto = (dyn DynProtocol + Send);
#[async_trait]
pub trait DynProtocol {
    async fn send(&mut self, discovery_key: &[u8], message: Message) -> Result<()>;
}
#[async_trait]
impl<R, W> DynProtocol for Protocol<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    async fn send(&mut self, discovery_key: &[u8], message: Message) -> Result<()> {
        self.send_channel(discovery_key, message).await
    }
}

pub type HandlerType = Arc<dyn Handlers + Send + Sync>;

#[async_trait]
pub trait Handlers: Sync {
    fn ondiscoverykey(&self, _discovery_key: &[u8]) -> Option<Vec<u8>> {
        None
    }

    async fn onopen(&self, _protocol: &mut Proto, _discovery_key: &[u8]) -> Result<()> {
        Ok(())
    }

    fn onmessage(&self, _discovery_key: &[u8], _message: Message) {}
}
