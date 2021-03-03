use async_channel::{Receiver, Sender};
use futures::io::{AsyncRead, AsyncWrite};
use futures::io::{BufReader, BufWriter};
use futures::stream::Stream;
use futures::task::{Context, Poll};
use futures_timer::Delay;
use log::*;
use std::collections::VecDeque;
use std::convert::TryInto;
use std::fmt;
use std::future::Future;
use std::io::{self, Error, ErrorKind, Result};
use std::pin::Pin;
use std::time::Duration;

use crate::builder::{Builder, Options};
use crate::channels::{Channel, ChannelMap};
use crate::constants::DEFAULT_KEEPALIVE;
use crate::message::{ChannelMessage, Message};
use crate::noise::{Handshake, HandshakeResult};
use crate::reader::ProtocolReader;
use crate::schema::*;
use crate::util::map_channel_err;
use crate::util::pretty_hash;
use crate::writer::ProtocolWriter;

macro_rules! return_error {
    ($msg:expr) => {
        if let Err(e) = $msg {
            return Poll::Ready(Err(e));
        }
    };
}

const CHANNEL_CAP: usize = 1000;
const KEEPALIVE_DURATION: Duration = Duration::from_secs(DEFAULT_KEEPALIVE as u64);

pub type RemotePublicKey = [u8; 32];
pub type DiscoveryKey = [u8; 32];
pub type Key = [u8; 32];

/// A protocol event.
pub enum Event {
    Handshake(RemotePublicKey),
    DiscoveryKey(DiscoveryKey),
    Channel(Channel),
    Close(DiscoveryKey),
    Error(std::io::Error),
}

/// A protocol command.
pub enum Command {
    Open(Key),
    Close(DiscoveryKey),
}

impl fmt::Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Handshake(remote_key) => {
                write!(f, "Handshake(remote_key={})", &pretty_hash(remote_key))
            }
            Event::DiscoveryKey(discovery_key) => {
                write!(f, "DiscoveryKey({})", &pretty_hash(discovery_key))
            }
            Event::Channel(channel) => {
                write!(f, "Channel({})", &pretty_hash(channel.discovery_key()))
            }
            Event::Close(discovery_key) => write!(f, "Close({})", &pretty_hash(discovery_key)),
            Event::Error(error) => write!(f, "{:?}", error),
        }
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
            State::NotInitialized => write!(f, "NotInitialized"),
            State::Handshake(_) => write!(f, "Handshaking"),
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
    writer: ProtocolWriter<BufWriter<W>>,
    reader: ProtocolReader<BufReader<R>>,
    state: State,
    options: Options,
    handshake: Option<HandshakeResult>,
    channels: ChannelMap,
    error: Option<Error>,
    command_rx: Receiver<Command>,
    command_tx: CommandTx,
    outbound_rx: Receiver<ChannelMessage>,
    outbound_tx: Sender<ChannelMessage>,
    keepalive: Delay,
    queued_events: VecDeque<Event>,
}

impl<R, W> Protocol<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    /// Create a new Protocol instance.
    pub fn new(reader: R, writer: W, options: Options) -> Self {
        let reader = ProtocolReader::new(BufReader::new(reader));
        let writer = ProtocolWriter::new(BufWriter::new(writer));
        let (command_tx, command_rx) = async_channel::bounded(CHANNEL_CAP);
        let (outbound_tx, outbound_rx) = async_channel::unbounded();
        Protocol {
            writer,
            reader,
            options,
            state: State::NotInitialized,
            channels: ChannelMap::new(),
            handshake: None,
            error: None,
            command_rx,
            command_tx: CommandTx(command_tx),
            outbound_tx,
            outbound_rx,
            keepalive: Delay::new(Duration::from_secs(DEFAULT_KEEPALIVE as u64)),
            queued_events: VecDeque::new(),
        }
    }

    /// Create a protocol builder.
    pub fn builder(is_initiator: bool) -> Builder {
        Builder::new(is_initiator)
    }

    pub fn is_initiator(&self) -> bool {
        self.options.is_initiator
    }

    /// Get your own Noise public key.
    ///
    /// Empty before the handshake completed.
    pub fn public_key(&self) -> Option<&[u8]> {
        match &self.handshake {
            None => None,
            Some(handshake) => Some(handshake.local_pubkey.as_slice()),
        }
    }

    /// Get the remote's Noise public key.
    ///
    /// Empty before the handshake completed.
    pub fn remote_public_key(&self) -> Option<&[u8]> {
        match &self.handshake {
            None => None,
            Some(handshake) => Some(handshake.remote_pubkey.as_slice()),
        }
    }

    /// Destroy the protocol instance with an error.
    pub fn destroy(&mut self, error: Error) {
        self.error = Some(error)
    }

    /// Get a sender to send commands.
    pub fn commands(&self) -> CommandTx {
        self.command_tx.clone()
    }

    /// Give a command to the protocol.
    pub async fn command(&mut self, command: Command) -> Result<()> {
        self.command_tx.send(command).await
    }

    /// Open a new protocol channel.
    ///
    /// Once the other side proofed that it also knows the `key`, the channel is emitted as
    /// `Event::Channel` on the protocol event stream.
    pub async fn open(&mut self, key: Key) -> Result<()> {
        self.command_tx.open(key).await
    }

    /// Stop the protocol and return the inner reader and writer.
    pub fn release(self) -> (R, W) {
        (
            self.reader.into_inner().into_inner(),
            self.writer.into_inner().into_inner(),
        )
    }

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<Event>> {
        let this = self.get_mut();

        if let State::NotInitialized = this.state {
            return_error!(this.init());
        }

        // Drain queued events first.
        if let Some(event) = this.queued_events.pop_front() {
            return Poll::Ready(Ok(event));
        }

        // Read and process incoming messages.
        return_error!(this.poll_inbound_read(cx));

        if let State::Established = this.state {
            // Check for commands, but only once the connection is established.
            return_error!(this.poll_commands(cx));
            // Process and forward outgoing messages from the channels.
            return_error!(this.poll_outbound_forward(cx));
        }

        // Poll the keepalive timer.
        this.poll_keepalive(cx);

        // Write queued outgoing messages to network.
        return_error!(this.poll_outbound_write(cx));

        // Check if any events are enqueued.
        if let Some(event) = this.queued_events.pop_front() {
            Poll::Ready(Ok(event))
        } else {
            Poll::Pending
        }
    }

    fn init(&mut self) -> Result<()> {
        debug!(
            "protocol init, state {:?}, options {:?}",
            self.state, self.options
        );
        match self.state {
            State::NotInitialized => {}
            _ => return Ok(()),
        };

        self.state = if self.options.noise {
            let mut handshake = Handshake::new(self.options.is_initiator)?;
            // If the handshake start returns a buffer, send it now.
            if let Some(buf) = handshake.start()? {
                self.queue_outbound_message(buf.to_vec());
            }
            State::Handshake(Some(handshake))
        } else {
            State::Established
        };

        Ok(())
    }

    /// Poll commands.
    fn poll_commands(self: &mut Self, cx: &mut Context) -> Result<()> {
        while let Poll::Ready(Some(command)) = Pin::new(&mut self.command_rx).poll_next(cx) {
            self.on_command(command)?;
        }
        Ok(())
    }

    /// Poll the keepalive timer and queue a ping message if needed.
    fn poll_keepalive(self: &mut Self, cx: &mut Context) {
        if let Poll::Ready(_) = Future::poll(Pin::new(&mut self.keepalive), cx) {
            self.queue_ping();
            self.keepalive.reset(KEEPALIVE_DURATION);
        }
    }

    /// Poll the channels for outbound messages.
    fn poll_outbound_forward(self: &mut Self, cx: &mut Context) -> Result<()> {
        while let Poll::Ready(Some(channel_message)) = Pin::new(&mut self.outbound_rx).poll_next(cx)
        {
            // If message is close, close the local channel.
            if let ChannelMessage {
                channel,
                message: Message::Close(_),
            } = channel_message
            {
                self.close_local(channel);
            }
            self.queue_outbound_channel_message(channel_message)?;
        }
        Ok(())
    }

    /// Poll for inbound messages and processs them.
    fn poll_inbound_read(self: &mut Self, cx: &mut Context) -> Result<()> {
        loop {
            let msg = Stream::poll_next(Pin::new(&mut self.reader), cx);
            match msg {
                Poll::Ready(Some(Ok(message))) => {
                    self.on_message(message)?;
                }
                Poll::Ready(Some(Err(e))) => return Err(e),
                Poll::Pending | Poll::Ready(None) => return Ok(()),
            }
        }
    }

    /// Write outgoing messages to the network.
    fn poll_outbound_write(self: &mut Self, cx: &mut Context) -> Result<()> {
        match Pin::new(&mut self.writer).poll_write_all(cx) {
            Poll::Pending | Poll::Ready(Ok(_)) => Ok(()),
            Poll::Ready(Err(e)) => Err(e),
        }
    }

    fn on_message(&mut self, buf: Vec<u8>) -> Result<()> {
        // log::trace!(
        //     "[{}] onmessage IN len {} state {:?}",
        //     self.is_initiator(),
        //     buf.len(),
        //     self.state
        // );
        match self.state {
            State::Handshake(_) => self.on_handshake_message(buf),
            State::Established => self.on_proto_message(buf),
            State::NotInitialized => panic!("cannot receive messages before starting the protocol"),
        }
    }

    fn on_handshake_message(&mut self, buf: Vec<u8>) -> Result<()> {
        let mut handshake = match &mut self.state {
            State::Handshake(handshake) => handshake.take().unwrap(),
            _ => panic!("may not call on_handshake_message when not in Handshake state"),
        };

        if let Some(response_buf) = handshake.read(&buf)? {
            self.queue_outbound_message(response_buf.to_vec());
        }

        if !handshake.complete() {
            self.state = State::Handshake(Some(handshake));
        } else {
            let result = handshake.into_result()?;
            if self.options.encrypted {
                self.reader.upgrade_with_handshake(&result)?;
                self.writer.upgrade_with_handshake(&result)?;
            }
            let remote_public_key = parse_key(&result.remote_pubkey)?;
            log::debug!(
                "handshake complete, remote_key {}",
                pretty_hash(&remote_public_key)
            );
            self.handshake = Some(result);
            self.state = State::Established;
            self.queue_event(Event::Handshake(remote_public_key));
        }
        Ok(())
    }

    fn on_proto_message(&mut self, buf: Vec<u8>) -> Result<()> {
        let channel_message = ChannelMessage::decode(buf)?;
        log::debug!("[{}] recv {:?}", self.is_initiator(), channel_message);
        let (remote_id, message) = channel_message.into_split();
        match message {
            Message::Open(msg) => self.on_open(remote_id, msg),
            Message::Close(msg) => self.on_close(remote_id, msg),
            Message::Extension(_msg) => unimplemented!(),
            _ => self
                .channels
                .forward_inbound_message(remote_id as usize, message),
        }
    }

    fn on_command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Open(key) => self.command_open(key),
            _ => Ok(()),
        }
    }

    fn command_open(&mut self, key: Key) -> Result<()> {
        // Create a new channel.
        let channel_handle = self.channels.attach_local(key.clone());
        // Safe because attach_local always puts Some(local_id)
        let local_id = channel_handle.local_id().unwrap();
        let discovery_key = channel_handle.discovery_key().clone();

        // If the channel was already opened from the remote end, verify, and if
        // verification is ok, push a channel open event.
        if channel_handle.is_connected() {
            self.accept_channel(local_id)?;
        }

        // Tell the remote end about the new channel.
        let capability = self.capability(&key);
        let message = Message::Open(Open {
            discovery_key: discovery_key.to_vec(),
            capability,
        });
        let channel_message = ChannelMessage::new(local_id as u64, message);
        self.queue_outbound_channel_message(channel_message)
    }

    fn on_open(&mut self, ch: u64, msg: Open) -> Result<()> {
        let discovery_key: DiscoveryKey = parse_key(&msg.discovery_key)?;
        let channel_handle =
            self.channels
                .attach_remote(discovery_key.clone(), ch as usize, msg.capability.clone());

        if channel_handle.is_connected() {
            let local_id = channel_handle.local_id().unwrap();
            self.accept_channel(local_id)?;
        } else {
            self.queue_event(Event::DiscoveryKey(discovery_key));
        }

        Ok(())
    }

    fn queue_event(&mut self, event: Event) {
        self.queued_events.push_back(event);
    }

    fn queue_outbound_message(&mut self, message: Vec<u8>) {
        self.writer.queue_message(message);
    }

    fn queue_outbound_channel_message(&mut self, channel_message: ChannelMessage) -> Result<()> {
        let buf = channel_message.encode()?;
        self.queue_outbound_message(buf);
        Ok(())
    }

    fn queue_ping(&mut self) {
        self.writer.queue_raw(vec![0]);
    }

    fn accept_channel(&mut self, local_id: usize) -> Result<()> {
        let (key, remote_capability) = self.channels.prepare_to_verify(local_id)?;
        self.verify_remote_capability(remote_capability.cloned(), key)?;
        let channel = self.channels.accept(local_id, self.outbound_tx.clone())?;
        self.queue_event(Event::Channel(channel));
        Ok(())
    }

    fn close_local(&mut self, local_id: u64) {
        if let Some(channel) = self.channels.get_local(local_id as usize) {
            let discovery_key = channel.discovery_key().clone();
            self.channels.remove(&discovery_key);
            self.queue_event(Event::Close(discovery_key));
        }
    }

    fn on_close(&mut self, remote_id: u64, msg: Close) -> Result<()> {
        if let Some(channel_handle) = self.channels.get_remote(remote_id as usize) {
            let discovery_key = channel_handle.discovery_key().clone();
            self.channels
                .forward_inbound_message(remote_id as usize, Message::Close(msg))?;
            self.channels.remove(&discovery_key);
            self.queue_event(Event::Close(discovery_key));
        }
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

impl<R, W> Stream for Protocol<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Item = Result<Event>;
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        Protocol::poll_next(self, cx).map(Some)
    }
}

/// Send [Command](Command)s to the [Protocol](Protocol).
#[derive(Clone)]
pub struct CommandTx(Sender<Command>);

impl CommandTx {
    pub async fn send(&mut self, command: Command) -> Result<()> {
        self.0.send(command).await.map_err(map_channel_err)
    }
    /// Open a protocol channel.
    ///
    /// The channel will be emitted on the main protocol.
    pub async fn open(&mut self, key: Key) -> Result<()> {
        self.send(Command::Open(key)).await
    }

    /// Close a protocol channel.
    pub async fn close(&mut self, discovery_key: DiscoveryKey) -> Result<()> {
        self.send(Command::Close(discovery_key)).await
    }
}

fn parse_key(key: &[u8]) -> io::Result<[u8; 32]> {
    key.try_into()
        .map_err(|_e| io::Error::new(io::ErrorKind::InvalidInput, "Key must be 32 bytes long"))
}
