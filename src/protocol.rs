use async_std::future;
use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use futures::stream::TryStreamExt;
use simple_message_channels::{send, Message as ChannelMessage, Reader};
use std::io::{Error, ErrorKind, Result};
use std::sync::Arc;
use std::time::Duration;

use crate::channels::{Channel, Channelizer};
use crate::constants::{DEFAULT_KEEPALIVE, DEFAULT_TIMEOUT};
use crate::encrypt::{EncryptedReader, EncryptedWriter};
use crate::handlers::{ChannelContext, ChannelHandlerType, DefaultHandlers, StreamHandlerType};
use crate::handshake::{handshake, HandshakeResult};
use crate::message::Message;
use crate::schema::*;
use crate::util::discovery_key;

/// Options for a Protocol instance.
pub struct ProtocolOptions {
    pub is_initiator: bool,
    pub noise: bool,
    pub encrypted: bool,
    pub handlers: Option<ChannelHandlerType>,
}

impl ProtocolOptions {
    /// Default options with handshake and encryption enabled.
    pub fn default(is_initiator: bool) -> Self {
        Self {
            is_initiator,
            noise: true,
            encrypted: true,
            handlers: None,
        }
    }

    /// Default options for an initiating endpoint.
    pub fn initiator() -> Self {
        Self::default(true)
    }

    /// Default options for a responding endpoint.
    pub fn responder() -> Self {
        Self::default(false)
    }

    pub fn set_handlers(mut self, handlers: ChannelHandlerType) -> Self {
        self.handlers = Some(handlers);
        self
    }

    pub fn set_encrypted(mut self, encrypted: bool) -> Self {
        self.encrypted = encrypted;
        self
    }

    pub fn set_noise(mut self, noise: bool) -> Self {
        self.noise = noise;
        self
    }
}

/// A Protocol stream.
pub struct Protocol<R, W> {
    raw_writer: W,
    reader: Reader<R>,
    // options: ProtocolOptions,
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
    /// Create a new Protocol instance. Note that this needs a handshake result and encryption
    /// has to be handled by the underlying reader and writer. This is for special case use only,
    /// normally you'd use the async initializers, e.g. [from_rw](Protocol::from_rw).
    pub fn new(reader: R, writer: W, handshake: Option<HandshakeResult>) -> Self {
        let handlers = Arc::new(DefaultHandlers {});
        Protocol {
            raw_writer: writer,
            reader: Reader::new(reader),
            handshake,
            channels: Channelizer::new(),
            handlers,
            error: None,
        }
    }

    /// Create a new Protocol from an [AsyncRead](AsyncRead) + [AsyncWrite](AsyncWrite) stream with default options.
    /// This needs complex type annotations, better use [from_rw](Protocol::from_rw).
    // pub async fn from_stream<S>(
    //     stream: S,
    //     is_initiator: bool,
    // ) -> Result<Protocol<EncryptedReader<S>, EncryptedWriter<S>>>
    // where
    //     S: AsyncRead + AsyncWrite + Send + Unpin + Clone + 'static,
    // {
    //     // TODO: This is in a seperate function because I ran into type parameters
    //     // not being derivable if inlined here.
    //     from_stream_with_options(stream, ProtocolOptions::default(is_initiator)).await
    // }

    /// Create a new Protocol from an [AsyncRead](AsyncRead) and an [AsyncWrite](AsyncWrite) with default options. The returned future resolves to a [Protocol](Protocol) once the
    /// handshake is complete.
    pub async fn from_rw(
        reader: R,
        writer: W,
        is_initiator: bool,
    ) -> Result<Protocol<EncryptedReader<R>, EncryptedWriter<W>>> {
        from_rw_with_options(reader, writer, ProtocolOptions::default(is_initiator)).await
    }

    /// Create a new Protocol from an [AsyncRead](AsyncRead) and an [AsyncWrite](AsyncWrite) with [options](ProtocolOptions). The returned future resolves to a [Protocol](Protocol) once the
    /// handshake is complete.
    pub async fn from_rw_with_options(
        reader: R,
        writer: W,
        options: ProtocolOptions,
    ) -> Result<Protocol<EncryptedReader<R>, EncryptedWriter<W>>> {
        from_rw_with_options(reader, writer, options).await
    }

    /// Create a new Protocol from an [AsyncRead](AsyncRead) + [AsyncWrite](AsyncWrite) stream with [options](ProtocolOptions). The returned future resolves to a [Protocol](Protocol) once the
    /// handshake is complete.
    pub async fn from_stream_with_options<S>(
        stream: S,
        options: ProtocolOptions,
    ) -> Result<Protocol<EncryptedReader<S>, EncryptedWriter<S>>>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + Clone + 'static,
    {
        // TODO: This is in a seperate function because I ran into type parameters
        // not being derivable if inlined here.
        from_stream_with_options(stream, options).await
    }

    /// Set the Protocol handlers.
    pub fn set_handlers(&mut self, handlers: StreamHandlerType) {
        self.handlers = handlers;
    }

    /// Start the protocol. The returned future resolves either if an error occurs
    /// or if the other side drops the connection.
    /// TODO: Enable graceful shutdown.
    pub async fn listen(&mut self) -> Result<()> {
        loop {
            // TODO: Check this later again.
            if let Some(error) = self.error.take() {
                return Err(error);
            }

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

    /// Destroy the protocol instance with an error.
    pub async fn destroy(&mut self, error: Error) {
        self.error = Some(error)
    }

    async fn onmessage(&mut self, message: ChannelMessage) -> Result<()> {
        let ChannelMessage {
            typ,
            channel,
            message,
        } = message;
        let message = Message::decode(typ, message)?;
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
        handlers.clone().on_open(&mut context, &discovery_key).await
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
                // wants to open a channel.
                self.handlers
                    .clone()
                    .on_discoverykey(&mut *self, &discovery_key)
                    .await?;

                // And check again if the channel is there.
                match self.channels.get(&discovery_key) {
                    Some(Channel { key: Some(key), .. }) => Ok(key.to_vec()),
                    _ => Err(Error::new(ErrorKind::BrokenPipe, "Key not found")),
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
        let encoded = msg.encode(ch)?;
        send(&mut self.raw_writer, encoded).await?;
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

async fn from_stream_with_options<S>(
    stream: S,
    options: ProtocolOptions,
) -> Result<Protocol<EncryptedReader<S>, EncryptedWriter<S>>>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + Clone + 'static,
{
    let reader = stream.clone();
    let writer = stream.clone();
    from_rw_with_options(reader, writer, options).await
}

async fn from_rw_with_options<RR, WW>(
    reader: RR,
    writer: WW,
    options: ProtocolOptions,
) -> Result<Protocol<EncryptedReader<RR>, EncryptedWriter<WW>>>
where
    RR: AsyncRead + Send + Unpin + 'static,
    WW: AsyncWrite + Send + Unpin + 'static,
{
    let mut reader = EncryptedReader::new(reader);
    let mut writer = EncryptedWriter::new(writer);
    let mut maybe_handshake = None;
    if options.noise {
        let handshake = handshake(&mut reader, &mut writer, options.is_initiator).await?;
        if options.encrypted {
            reader.upgrade_with_handshake(&handshake)?;
            writer.upgrade_with_handshake(&handshake)?;
        }
        maybe_handshake = Some(handshake);
    }
    Ok(Protocol::new(reader, writer, maybe_handshake))
}

// pub type ReadType = EncryptedReader<BufReader<AsyncRead + Send + Unpin + Clone + 'static>>;
// pub type WriteType = EncryptedReader<BufWriter<AsyncWrite + Send + Unpin + Clone + 'static>>;
// pub async fn from_stream_with_handshake<S>(
//     stream: S,
//     is_initiator: bool,
// ) -> Result<Protocol<EncryptedReader<S>, EncryptedWriter<S>>>
// where
//     S: AsyncRead + AsyncWrite + Send + Unpin + Clone + 'static,
// {
//     let reader = stream.clone();
//     let writer = stream.clone();
//     from_rw_with_handshake(reader, writer, is_initiator).await
// }

// async fn from_rw_with_handshake<R, W>(
//     reader: R,
//     writer: W,
//     is_initiator: bool,
// ) -> Result<Protocol<EncryptedReader<R>, EncryptedWriter<W>>>
// where
//     R: AsyncRead + Send + Unpin + 'static,
//     W: AsyncWrite + Send + Unpin + 'static,
// {
//     let mut reader = EncryptedReader::new(reader);
//     let mut writer = EncryptedWriter::new(writer);
//     let handshake = handshake(&mut reader, &mut writer, is_initiator).await?;
//     reader.upgrade_with_handshake(&handshake)?;
//     writer.upgrade_with_handshake(&handshake)?;
//     Ok(Protocol::new(reader, writer, Some(handshake)))
// }
