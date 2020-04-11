use futures::future::{select, Either};
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use futures::io::{BufReader, BufWriter};
use log::*;
use std::fmt;
use std::io::{Error, ErrorKind, Result};
use std::time::Duration;
// We use the instant crate for WASM compatiblity.
use instant::Instant;

use crate::channels::Channelizer;
use crate::constants::{DEFAULT_KEEPALIVE, DEFAULT_TIMEOUT, MAX_MESSAGE_SIZE};
use crate::encrypt::{EncryptedReader, EncryptedWriter};
use crate::handlers::{Channel, ChannelHandlerType, DefaultHandlers, StreamHandlerType};
use crate::handshake::{Handshake, HandshakeResult};
use crate::message::Message;
use crate::schema::*;
use crate::util::discovery_key;
use crate::wire_message::Message as WireMessage;

/// Options for a Protocol instance.
pub struct ProtocolOptions {
    pub is_initiator: bool,
    pub noise: bool,
    pub encrypted: bool,
    pub handlers: Option<StreamHandlerType>,
}

impl fmt::Debug for ProtocolOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProtocolOptions")
            .field("is_initiator", &self.is_initiator)
            .field("noise", &self.noise)
            .field("encrypted", &self.encrypted)
            .finish()
    }
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

    pub fn build_from_stream<S>(self, stream: S) -> Protocol<S, S>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + Clone + 'static,
    {
        Protocol::new(stream.clone(), stream, self.0)
    }

    pub fn build_from_io<R, W>(self, reader: R, writer: W) -> Protocol<R, W>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Protocol::new(reader, writer, self.0)
    }
}

/// Protocol state
#[allow(clippy::large_enum_variant)]
pub enum State {
    // The Handshake struct sits behind an option only so that we can .take()
    // it out, it's never actually empty when in State::Handshake.
    Handshake(Option<Handshake>),
    Established,
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            State::Handshake(_) => write!(f, "Handshake"),
            State::Established => write!(f, "Established"),
        }
    }
}

/// A Protocol stream.
pub struct Protocol<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    writer: EncryptedWriter<BufWriter<W>>,
    reader: EncryptedReader<BufReader<R>>,
    state: State,
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
        let reader = EncryptedReader::new(BufReader::new(reader));
        let writer = EncryptedWriter::new(BufWriter::new(writer));
        let handlers = options
            .handlers
            .take()
            .unwrap_or_else(|| DefaultHandlers::new());

        let state = if options.noise {
            State::Handshake(Some(Handshake::new(options.is_initiator).unwrap()))
        } else {
            State::Established
        };

        Protocol {
            writer,
            reader,
            handlers,
            options,
            state,
            channels: Channelizer::new(),
            handshake: None,
            error: None,
            // stream_state,
        }
    }

    // Start the main protocol loop.
    //
    // The returned future resolves either if an error occurrs, if the connection
    // is dropped, or if all channels are closed (TODO: implement the latter).
    pub async fn listen(&mut self) -> Result<()> {
        trace!("protocol init, options {:?}", self.options);
        // If we are the initiator, first send the initial handshake payload.
        if let State::Handshake(ref mut handshake) = self.state {
            let mut handshake = handshake.take().unwrap();
            if let Some(buf) = handshake.start()? {
                self.send_prefixed(buf).await?;
            }
            self.state = State::Handshake(Some(handshake))
        }
        // Enter the main receive loop.
        self.main_loop().await
    }

    /// The main protocol loop.
    async fn main_loop(&mut self) -> Result<()> {
        /// The reading state.
        #[derive(Debug)]
        struct State {
            /// The read buffer.
            buf: Vec<u8>,
            /// The number of relevant bytes in the read buffer.
            cap: usize,
            /// The logical state of the reading (either header or body).
            step: Step,
        }
        // The reading step.
        #[derive(Debug)]
        enum Step {
            Header { factor: u64, varint: u64 },
            Body { header_len: usize, body_len: usize },
        }

        let keepalive_secs = Duration::from_secs(DEFAULT_KEEPALIVE as u64);
        let timeout_secs = Duration::from_secs(DEFAULT_TIMEOUT as u64);

        let mut state = State {
            buf: vec![0u8; MAX_MESSAGE_SIZE as usize + 8],
            cap: 0,
            step: Step::Header {
                factor: 1,
                varint: 0,
            },
        };

        let mut keepalive = Some(futures_timer::Delay::new(keepalive_secs));
        let mut timeout = Instant::now();

        loop {
            // Wait for new bytes to arrive, or for the keepalive to occur to send a ping.
            // If data was received, keep the previous keepalive timer.
            let read_fut = self.reader.read(&mut state.buf[state.cap..]);
            let keepalive_fut = keepalive.take().unwrap();
            let (bytes_read, next_keepalive) = match select(keepalive_fut, read_fut).await {
                Either::Left(_) => {
                    self.ping().await?;
                    // Create a new keepalive future for the next ping.
                    let keepalive = futures_timer::Delay::new(keepalive_secs);
                    (None, keepalive)
                }
                Either::Right((Err(e), _)) => return Err(e),
                Either::Right((Ok(0), _)) => return Ok(()),
                Either::Right((Ok(n), last_keepalive)) => (Some(n), last_keepalive),
            };
            // Store our keepalive for the next iteration.
            keepalive = Some(next_keepalive);

            // If we read some bytes, increase cap and reset the timeout timer.
            if let Some(n) = bytes_read {
                state.cap += n;
                timeout = Instant::now()
            } else if timeout.elapsed() > timeout_secs {
                return Err(Error::new(ErrorKind::TimedOut, "Remote timeout"));
            }

            // If there's no data to process, re-enter the select loop.
            if state.cap == 0 {
                continue;
            }

            // Keep processing our current buffer until we need more bytes.
            let mut needs_more_bytes = false;
            while !needs_more_bytes {
                match state.step {
                    // Read a varint.
                    Step::Header {
                        ref mut factor,
                        ref mut varint,
                    } => {
                        needs_more_bytes = true;
                        for (i, byte) in state.buf[..state.cap].iter().enumerate() {
                            // Ignore empty keepalive bytes.
                            if byte == &0 {
                                continue;
                            }
                            *varint += (*byte as u64 & 127) * *factor;
                            if *varint > MAX_MESSAGE_SIZE {
                                return Err(Error::new(
                                    ErrorKind::InvalidInput,
                                    "Message too long",
                                ));
                            }
                            if byte < &128 {
                                state.step = Step::Body {
                                    header_len: i + 1,
                                    body_len: *varint as usize,
                                };
                                needs_more_bytes = false;
                                break;
                            }
                            *factor *= 128;
                        }
                    }
                    // Read the actual message.
                    Step::Body {
                        header_len,
                        body_len,
                    } => {
                        let message_len = header_len + body_len;
                        if message_len > state.cap {
                            // Not enough bytes for a full message, return to reading.
                            needs_more_bytes = true
                        } else {
                            // We have enough bytes for a full message!
                            let message_buf = &state.buf[header_len..message_len];
                            self.on_message(message_buf).await?;

                            // If we have even more bytes, copy them to the beginning of our read
                            // buffer and adjust the cap accordingly.
                            if state.cap > message_len {
                                // TODO: If we were using a ring buffer we wouldn't have to copy and
                                // allocate here.
                                let overflow_buf = &state.buf[message_len..state.cap].to_vec();
                                state.cap -= message_len;
                                state.buf[..state.cap].copy_from_slice(&overflow_buf[..]);
                            // Otherwise, read again!
                            } else {
                                state.cap = 0;
                                needs_more_bytes = true;
                            }
                            // In any case, after reading a message the next step is to read a varint.
                            state.step = Step::Header {
                                factor: 1,
                                varint: 0,
                            };
                        }
                    }
                }
            }
        }
    }

    /// Destroy the protocol instance with an error.
    pub fn destroy(&mut self, error: Error) {
        self.error = Some(error)
    }

    async fn on_message(&mut self, buf: &[u8]) -> Result<()> {
        match self.state {
            State::Handshake(ref mut handshake) => {
                let handshake = handshake.take().unwrap();
                self.on_handshake_message(buf, handshake).await
            }
            State::Established => self.on_proto_message(buf).await,
        }
    }

    async fn on_handshake_message(&mut self, buf: &[u8], mut handshake: Handshake) -> Result<()> {
        if let Some(send) = handshake.read(buf)? {
            self.send_prefixed(send).await?;
        }
        if handshake.complete() {
            let result = handshake.into_result()?;
            if self.options.encrypted {
                self.reader.upgrade_with_handshake(&result)?;
                self.writer.upgrade_with_handshake(&result)?;
            }
            log::trace!("handshake completed");
            self.handshake = Some(result);
            self.state = State::Established;
        } else {
            self.state = State::Handshake(Some(handshake))
        }
        Ok(())
    }

    async fn on_proto_message(&mut self, message_buf: &[u8]) -> Result<()> {
        let message = WireMessage::from_buf(&message_buf)?;
        let channel = message.channel;
        let message = Message::decode(message.typ, message.message)?;
        log::trace!("recv: {}", message);
        let _result = match message {
            Message::Open(msg) => self.on_open(channel, msg).await,
            Message::Close(msg) => self.on_close(channel, msg).await,
            Message::Extension(_msg) => unimplemented!(),
            _ => {
                let discovery_key = self.channels.resolve_remote(channel as usize)?;
                self.on_channel_message(discovery_key, message).await
            }
        };
        Ok(())
    }

    async fn on_channel_message<'a>(
        &'a mut self,
        discovery_key: Vec<u8>,
        message: Message,
    ) -> Result<()> {
        let (handlers, mut context) = self.channel_context(&discovery_key)?;
        handlers.on_message(&mut context, message).await
    }

    async fn on_channel_open<'a>(&'a mut self, discovery_key: Vec<u8>) -> Result<()> {
        let (handlers, mut context) = self.channel_context(&discovery_key)?;
        handlers.on_open(&mut context, &discovery_key).await
    }

    fn channel_context<'a>(
        &'a mut self,
        discovery_key: &'a [u8],
    ) -> Result<(ChannelHandlerType, Channel<'a>)> {
        let channel = self
            .channels
            .get(&discovery_key)
            .ok_or_else(|| Error::new(ErrorKind::BrokenPipe, "Channel is not open"))?;
        let handlers = channel.handlers.clone();
        let context = Channel::new(&mut *self, &discovery_key);
        Ok((handlers, context))
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

    async fn on_close(&mut self, ch: u64, msg: Close) -> Result<()> {
        let ch = ch as usize;
        if let Some(discovery_key) = msg.discovery_key {
            self.channels.remove(&discovery_key);
        } else if let Some(channel) = self.channels.get_remote(ch) {
            let discovery_key = channel.discovery_key.clone();
            self.channels.remove(&discovery_key);
        }
        Ok(())
    }

    async fn on_open(&mut self, ch: u64, msg: Open) -> Result<()> {
        let Open {
            discovery_key,
            capability,
        } = msg;

        let mut key = self.channels.get_key(&discovery_key);

        // This means there is not yet a locally-opened channel for this discovery_key.
        if key.is_none() {
            // Let the application open a channel if wanted.
            let handlers = self.handlers.clone();
            handlers.on_discoverykey(&mut *self, &discovery_key).await?;
            // And see if it happened.
            key = self.channels.get_key(&discovery_key);
        }

        // If we still have no key for this channel after invoking the on_discoverykey handler,
        // we return.
        if key.is_none() {
            // TODO: Do something if a channel is opened without us having the key,
            // maybe close the channel to singal this back to the remote?
            return Ok(());
        }
        // This is save because of the check above.
        let key = key.unwrap();

        // Verify the remote capability.
        self.verify_remote_capability(capability, &key)?;
        // Attach the channel for future use.
        self.channels
            .attach_remote(discovery_key.clone(), ch as usize)?;
        // Let the channel handler know the channel is open.
        self.on_channel_open(discovery_key.to_vec()).await?;
        Ok(())
    }

    pub async fn send_raw(&mut self, buf: &[u8]) -> Result<()> {
        self.writer.write_all(&buf).await?;
        self.writer.flush().await
    }

    pub async fn send_prefixed(&mut self, buf: &[u8]) -> Result<()> {
        let len = buf.len();
        let prefix_len = varinteger::length(len as u64);
        let mut prefix_buf = vec![0u8; prefix_len];
        varinteger::encode(len as u64, &mut prefix_buf[..prefix_len]);
        self.writer.write_all(&prefix_buf).await?;
        self.writer.write_all(&buf).await?;
        self.writer.flush().await
    }

    pub async fn send(&mut self, ch: u64, mut msg: Message) -> Result<()> {
        log::trace!("send: {}", msg);
        let message = msg.encode(ch)?;
        let buf = message.encode()?;
        self.send_prefixed(&buf).await
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
