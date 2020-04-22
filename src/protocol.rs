use futures::future::{select, Either, Future, FutureExt};
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use futures::io::{BufReader, BufWriter};
use futures::sink::SinkExt;
use futures::stream::{Fuse, Map, SelectAll, Stream, StreamExt};
use futures_timer::Delay;
use log::*;
use std::collections::VecDeque;
use std::fmt;
use std::io::{Error, ErrorKind, Result};
use std::task::Poll;
use std::time::Duration;
// We use the instant crate for WASM compatiblity.
use futures::channel::mpsc::{Receiver, Sender};
use instant::Instant;
use std::pin::Pin;

use crate::channels::Channelizer;
use crate::constants::DEFAULT_KEEPALIVE;
use crate::encrypt::{EncryptedReader, EncryptedWriter};
use crate::handshake::{Handshake, HandshakeResult};
use crate::message::Message;
use crate::schema::*;
use crate::util::discovery_key;
use crate::wire_message::Message as WireMessage;

#[derive(Debug)]
pub enum Event {
    Handshake(Vec<u8>),
    DiscoveryKey(Vec<u8>),
    Channel(Channel),
}

#[derive(Debug)]
pub struct Channel {
    receiver: Receiver<Message>,
    sender: Sender<Message>,
    discovery_key: Vec<u8>, // id: usize, // discovery_key: Vec<u8>,
}

impl Channel {
    pub fn sender(&self) -> Sender<Message> {
        self.sender.clone()
    }
    pub fn discovery_key(&self) -> &[u8] {
        &self.discovery_key
    }
    pub async fn send(&mut self, message: Message) -> Result<()> {
        self.sender.send(message).await.map_err(map_channel_err)
    }
}

impl Stream for Channel {
    type Item = Message;
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_next(cx)
    }
}

/// Options for a Protocol instance.
pub struct ProtocolOptions {
    pub is_initiator: bool,
    pub noise: bool,
    pub encrypted: bool,
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
            // handlers: None,
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

    // pub fn set_handlers(mut self, handlers: StreamHandlerType) -> Self {
    //     self.0.handlers = Some(handlers);
    //     self
    // }

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
    NotInitialized,
    // The Handshake struct sits behind an option only so that we can .take()
    // it out, it's never actually empty when in State::Handshake.
    Handshake(Option<Handshake>),
    Established,
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            State::NotInitialized => write!(f, "Not initialized"),
            State::Handshake(_) => write!(f, "Handshake"),
            State::Established => write!(f, "Established"),
        }
    }
}

type CombinedOutputStream = SelectAll<Box<dyn Stream<Item = (usize, Message)> + Send + Unpin>>;

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
    error: Option<Error>,
    outbound_recv: CombinedOutputStream,
    messages: VecDeque<(u64, Message)>,
    events: VecDeque<Event>,
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
        Protocol {
            writer,
            reader,
            options,
            state: State::NotInitialized,
            channels: Channelizer::new(),
            handshake: None,
            error: None,
            outbound_recv: SelectAll::new(), // stream_state,
            events: VecDeque::new(),
            messages: VecDeque::new(),
        }
    }

    pub async fn init(&mut self) -> Result<()> {
        trace!("protocol init, options {:?}", self.options);
        match self.state {
            State::NotInitialized => {}
            _ => return Ok(()),
        };

        self.state = if self.options.noise {
            State::Handshake(Some(Handshake::new(self.options.is_initiator).unwrap()))
        } else {
            State::Established
        };

        // If we are the initiator, first send the initial handshake payload.
        if let State::Handshake(ref mut handshake) = self.state {
            eprintln!("yes here");
            let mut handshake = handshake.take().unwrap();
            if let Some(buf) = handshake.start()? {
                eprintln!("ok good");
                self.send_prefixed(buf).await?;
            }
            self.state = State::Handshake(Some(handshake))
        }

        Ok(())
    }

    pub async fn loop_next(&mut self) -> Result<Event> {
        if let State::NotInitialized = self.state {
            self.init().await?;
        }

        while let Some((ch, message)) = self.messages.pop_front() {
            self.send(ch, message).await?;
        }

        if let Some(event) = self.events.pop_front() {
            return Ok(event);
        }

        let keepalive_duration = Duration::from_secs(DEFAULT_KEEPALIVE as u64);
        let mut keepalive = Delay::new(keepalive_duration).fuse();

        // Wait for new bytes to arrive, or for the keepalive to occur to send a ping.
        // If data was received, reset the keepalive timer.
        loop {
            let event = futures::select! {
                _ = keepalive => {
                    self.ping().await?;
                    keepalive = Delay::new(keepalive_duration).fuse();
                    None
                    // keepalive.reset(keepalive_duration);
                },
                message = self.reader.select_next_some() => {
                    match message {
                        Ok(message) => self.on_message(&message).await?,
                        Err(err) => return Err(err),
                        // None => return Ok(()),
                    }
                },
                (ch, message) = self.outbound_recv.select_next_some() => {
                    self.send(ch as u64, message).await?;
                    None
                }
            };
            trace!("main loop out, event {}", event.is_some());
            if let Some(event) = event {
                return Ok(event);
            }
        }
    }

    pub fn remote_key(&self) -> Option<&[u8]> {
        match &self.handshake {
            None => None,
            Some(handshake) => Some(handshake.remote_pubkey.as_slice()),
        }
    }

    /// Destroy the protocol instance with an error.
    pub fn destroy(&mut self, error: Error) {
        self.error = Some(error)
    }

    async fn on_message(&mut self, buf: &[u8]) -> Result<Option<Event>> {
        eprintln!("onmessage, state {:?} msg len {}", self.state, buf.len());
        match self.state {
            State::Handshake(ref mut handshake) => {
                let handshake = handshake.take().unwrap();
                self.on_handshake_message(buf, handshake).await
            }
            State::Established => self.on_proto_message(buf).await,
            _ => panic!("invalid state"),
        }
    }

    async fn on_handshake_message(
        &mut self,
        buf: &[u8],
        mut handshake: Handshake,
    ) -> Result<Option<Event>> {
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
            Ok(Some(Event::Handshake(self.remote_key().unwrap().to_vec())))
        } else {
            self.state = State::Handshake(Some(handshake));
            Ok(None)
        }
    }

    async fn on_proto_message(&mut self, message_buf: &[u8]) -> Result<Option<Event>> {
        let message = WireMessage::from_buf(&message_buf)?;
        let channel = message.channel;
        let message = Message::decode(message.typ, message.message)?;
        log::trace!("recv: {}", message);
        match message {
            Message::Open(msg) => self.on_open(channel, msg).await,
            Message::Close(msg) => {
                self.on_close(channel, msg).await?;
                Ok(None)
            }
            Message::Extension(_msg) => unimplemented!(),
            _ => {
                self.channels.forward(channel as usize, message).await?;
                Ok(None)
            }
        }
    }

    pub async fn open(&mut self, key: Vec<u8>) -> Result<()> {
        let discovery_key = discovery_key(&key);
        let id = self.channels.attach_local(key.clone());
        if let Some(channel) = self.channels.get(&discovery_key) {
            if let Some(_remote_id) = channel.remote_id {
                self.verify_remote_capability(channel.remote_capability.clone(), &key)?;
                let channel = self.create_channel(id, &discovery_key).await;
                self.events.push_back(Event::Channel(channel));
            }
        }

        let capability = self.capability(&key);
        let message = Message::Open(Open {
            discovery_key: discovery_key.clone(),
            capability,
        });
        self.messages.push_back((id as u64, message));

        Ok(())
    }

    async fn create_channel(&mut self, id: usize, discovery_key: &[u8]) -> Channel {
        let (send_tx, send_rx) = futures::channel::mpsc::channel(100);
        let (recv_tx, recv_rx) = futures::channel::mpsc::channel(100);
        let channel = Channel {
            receiver: recv_rx,
            sender: send_tx,
            discovery_key: discovery_key.to_vec(), // id: id.clone(),
        };
        let send_rx_mapped = send_rx.map(move |message| (id, message));
        self.outbound_recv.push(Box::new(send_rx_mapped));
        self.channels.open(&discovery_key, recv_tx).await.unwrap();
        channel
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

    async fn on_open(&mut self, ch: u64, msg: Open) -> Result<Option<Event>> {
        let Open {
            discovery_key,
            capability,
        } = msg.clone();

        let key = self.channels.get_key(&discovery_key);
        self.channels
            .attach_remote(discovery_key.clone(), ch as usize, capability.clone())?;

        // This means there is not yet a locally-opened channel for this discovery_key.
        if key.is_none() {
            return Ok(Some(Event::DiscoveryKey(discovery_key.clone())));
        } else {
            if let Some(channel) = self.channels.get(&discovery_key) {
                let key = key.unwrap();
                self.verify_remote_capability(capability, &key)?;
                let channel = self
                    .create_channel(channel.local_id.clone().unwrap(), &discovery_key)
                    .await;
                self.channels
                    .forward(ch as usize, Message::Open(msg))
                    .await?;
                return Ok(Some(Event::Channel(channel)));
            }
            return Ok(None);
        }
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
            None => Err(Error::new(
                ErrorKind::BrokenPipe,
                "Cannot send: Channel is not open",
            )),
            Some(local_id) => self.send(local_id as u64, msg).await,
        }
    }

    async fn ping(&mut self) -> Result<()> {
        let buf = vec![0u8];
        self.send_raw(&buf).await
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

fn map_channel_err(err: futures::channel::mpsc::SendError) -> Error {
    Error::new(
        ErrorKind::BrokenPipe,
        format!("Cannot forward on channel: {}", err),
    )
}
