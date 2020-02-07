use async_std::future;
use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use simple_message_channels::Message as WireMessage;
use std::io::{Error, ErrorKind, Result};
use std::time::Duration;

use crate::channels::{Channel, Channelizer};
use crate::constants::{DEFAULT_KEEPALIVE, DEFAULT_TIMEOUT};
use crate::encrypt::{EncryptedReader, EncryptedWriter};
use crate::handlers::{ChannelContext, ChannelHandlerType, DefaultHandlers, StreamHandlerType};
use crate::handshake::{handshake, HandshakeResult};
use crate::message::Message;
use crate::prefixed;
use crate::schema::*;
use crate::util::discovery_key;

/// Options for a Protocol instance.
pub struct ProtocolOptions {
    pub is_initiator: bool,
    pub noise: bool,
    pub encrypted: bool,
    pub handlers: Option<StreamHandlerType>,
}

/// Build a Protocol instance with options.
pub struct ProtocolBuilder(ProtocolOptions);

impl ProtocolBuilder {
    pub fn new(is_initiator: bool) -> Self {
        Self(ProtocolOptions {
            is_initiator,
            noise: true,
            encrypted: true,
            handlers: None,
        })
    }

    /// Default options for an initiating endpoint.
    pub fn initiator() -> Self {
        Self::new(true)
    }

    /// Default options for a responding endpoint.
    pub fn responder() -> Self {
        Self::new(false)
    }

    pub fn set_handlers(mut self, handlers: StreamHandlerType) -> Self {
        self.0.handlers = Some(handlers);
        self
    }

    pub fn set_encrypted(mut self, encrypted: bool) -> Self {
        self.0.encrypted = encrypted;
        self
    }

    pub fn set_noise(mut self, noise: bool) -> Self {
        self.0.noise = noise;
        self
    }

    pub fn from_stream<S>(self, stream: S) -> Protocol<S, S>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + Clone + 'static,
    {
        Protocol::new(stream.clone(), stream.clone(), self.0)
    }

    pub fn from_rw<R, W>(self, reader: R, writer: W) -> Protocol<R, W>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Protocol::new(reader, writer, self.0)
    }
}

/// A Protocol stream.
pub struct Protocol<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    writer: EncryptedWriter<W>,
    reader: EncryptedReader<R>,
    options: ProtocolOptions,
    handshake: Option<HandshakeResult>,
    channels: Channelizer,
    handlers: StreamHandlerType,
    error: Option<Error>,
}

impl<R, W> Protocol<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    /// Create a new Protocol instance.
    pub fn new(reader: R, writer: W, mut options: ProtocolOptions) -> Self {
        let reader = EncryptedReader::new(reader);
        let writer = EncryptedWriter::new(writer);
        let handlers = options
            .handlers
            .take()
            .unwrap_or_else(|| DefaultHandlers::new());
        Protocol {
            writer,
            reader,
            handlers,
            options,
            channels: Channelizer::new(),
            handshake: None,
            error: None,
        }
    }

    // Start the main protocol loop.
    //
    // The returned future resolves either if an error occurrs, if the connection
    // is dropped, or if all channels are closed (TODO: implement the latter).
    pub async fn listen(&mut self) -> Result<()> {
        if self.options.noise {
            self.perform_handshake().await?;
        }
        self.main_loop().await?;
        Ok(())
    }

    async fn perform_handshake(&mut self) -> Result<()> {
        let handshake = handshake(
            &mut self.reader,
            &mut self.writer,
            self.options.is_initiator,
        )
        .await?;
        if self.options.encrypted {
            self.reader.upgrade_with_handshake(&handshake)?;
            self.writer.upgrade_with_handshake(&handshake)?;
        }
        self.handshake = Some(handshake);
        Ok(())
    }

    /// Start the protocol. The returned future resolves either if an error occurs
    /// or if the other side drops the connection.
    /// TODO: Enable graceful shutdown.
    async fn main_loop(&mut self) -> Result<()> {
        // TODO: Implement timeout.
        let _timeout = Duration::from_secs(DEFAULT_TIMEOUT as u64);
        let keepalive = Duration::from_secs(DEFAULT_KEEPALIVE as u64);
        loop {
            // Check if an error was set (through Protocol.destroy()).
            if let Some(error) = self.error.take() {
                return Err(error);
            }

            let next = prefixed::read_prefixed(&mut self.reader);

            match future::timeout(keepalive, next).await {
                Err(_timeout_err) => {
                    self.ping().await?;
                }
                Ok(Ok(message)) => {
                    let message = WireMessage::from_buf(&message)?;
                    self.onmessage(message).await?;
                }
                Ok(Err(e)) => {
                    return Err(e);
                }
            }
        }
    }

    /// Destroy the protocol instance with an error.
    pub async fn destroy(&mut self, error: Error) {
        self.error = Some(error)
    }

    async fn onmessage(&mut self, message: WireMessage) -> Result<()> {
        let channel = message.channel;
        let message = Message::decode(message.typ, message.message)?;
        log::trace!("recv: {}", message);
        let _result = match message {
            Message::Open(msg) => self.onopen(channel, msg).await,
            Message::Close(msg) => self.onclose(channel, msg).await,
            Message::Extension(_msg) => unimplemented!(),
            _ => {
                let channel = self
                    .channels
                    .get_remote(channel as usize)
                    .ok_or(Error::new(
                        ErrorKind::BrokenPipe,
                        "Received message on closed channel",
                    ))?;

                let discovery_key = channel.discovery_key.clone();
                let handlers = channel.handlers.clone();
                self.on_channel_message(handlers, discovery_key, message)
                    .await
            }
        };
        Ok(())
    }

    async fn on_channel_message<'a>(
        &'a mut self,
        handlers: ChannelHandlerType,
        discovery_key: Vec<u8>,
        message: Message,
    ) -> Result<()> {
        let mut context = ChannelContext::new(&mut *self, &discovery_key);
        let handlers = handlers.clone();
        handlers.onmessage(&mut context, message).await
    }

    async fn on_channel_open<'a>(
        &'a mut self,
        handlers: ChannelHandlerType,
        discovery_key: Vec<u8>,
    ) -> Result<()> {
        let mut context = ChannelContext::new(&mut *self, &discovery_key);
        let handlers = handlers.clone();
        handlers.on_open(&mut context, &discovery_key).await
    }

    /// Open a new channel by passing a key and a Arc-wrapped [handlers](crate::ChannelHandlers)
    /// object.
    pub async fn open(&mut self, key: Vec<u8>, handlers: ChannelHandlerType) -> Result<()> {
        self.channels.attach_local(key.clone(), handlers);
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

        // The remote is opening a channel for a discovery key. Let's see
        // if we know the pubkey for this channel.
        let key = match self.channels.get(&discovery_key) {
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
                // wants to open a channel.
                self.handlers
                    .clone()
                    .on_discoverykey(&mut *self, &discovery_key)
                    .await?;

                // And check again if a channel has been opened.
                match self.channels.get(&discovery_key) {
                    Some(Channel { key: Some(key), .. }) => Ok(key.to_vec()),
                    _ => Err(Error::new(ErrorKind::BrokenPipe, "Key not found")),
                }
            }
        }?;

        // Verify the remote capability.
        self.verify_remote_capability(capability, &key)?;
        // Attach the channel for future use.
        self.channels
            .attach_remote(discovery_key.clone(), ch as usize);

        // Here, a channel should always be ready.
        let channel = self.channels.get(&discovery_key).ok_or(Error::new(
            ErrorKind::BrokenPipe,
            "Failed to open a channel.",
        ))?;
        let handlers = channel.handlers.clone();
        self.on_channel_open(handlers, discovery_key.to_vec())
            .await?;
        Ok(())
    }

    pub async fn send(&mut self, ch: u64, mut msg: Message) -> Result<()> {
        log::trace!("send {} {}", ch, msg);
        let message = msg.encode(ch)?;
        let buf = message.encode()?;
        self.writer.write_all(&buf).await?;
        self.writer.flush().await?;
        Ok(())
    }

    pub async fn send_channel(&mut self, discovery_key: &[u8], msg: Message) -> Result<()> {
        match self.channels.get_local_id(&discovery_key) {
            None => Err(Error::new(ErrorKind::BrokenPipe, "Channel is not open")),
            Some(local_id) => self.send(local_id as u64, msg).await,
        }
    }

    async fn ping(&mut self) -> Result<()> {
        let buf = vec![0u8];
        self.writer.write_all(&buf).await?;
        self.writer.flush().await?;
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
