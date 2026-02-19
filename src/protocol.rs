use async_channel::{Receiver, Sender};
use futures_lite::stream::Stream;
use futures_timer::Delay;
use hypercore_handshake::{CipherTrait, state_machine::PUBLIC_KEYLEN};
use std::{
    collections::VecDeque,
    convert::TryInto,
    fmt,
    io::{self, Result},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tracing::{error, instrument};

use crate::{
    channels::{Channel, ChannelMap},
    constants::{DEFAULT_KEEPALIVE, PROTOCOL_NAME},
    crypto::HandshakeResult,
    message::{ChannelMessage, Message},
    mqueue::MessageIo,
    schema::*,
    util::{map_channel_err, pretty_hash},
};

macro_rules! return_error {
    ($msg:expr) => {
        if let Err(e) = $msg {
            return Poll::Ready(Err(e));
        }
    };
}

const CHANNEL_CAP: usize = 1000;

/// Remote public key (32 bytes).
pub(crate) type RemotePublicKey = [u8; 32];
/// Discovery key (32 bytes).
pub type DiscoveryKey = [u8; 32];
/// Key (32 bytes).
pub type Key = [u8; 32];

/// A protocol event.
#[non_exhaustive]
#[derive(PartialEq)]
pub enum Event {
    /// Emitted after the handshake with the remote peer is complete.
    /// This is the first event.
    Handshake(RemotePublicKey),
    /// Emitted when the remote peer opens a channel that we did not yet open.
    DiscoveryKey(DiscoveryKey),
    /// Emitted when a channel is established.
    Channel(Channel),
    /// Emitted when a channel is closed.
    Close(DiscoveryKey),
    /// Convenience event to make it possible to signal the protocol from a channel.
    /// See channel.signal_local() and protocol.commands().signal_local().
    LocalSignal((String, Vec<u8>)),
}

/// A protocol command.
#[derive(Debug)]
pub enum Command {
    /// Open a channel
    Open(Key),
    /// Close a channel by discovery key
    Close(DiscoveryKey),
    /// Signal locally to protocol
    SignalLocal((String, Vec<u8>)),
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
            Event::LocalSignal((name, data)) => {
                write!(f, "LocalSignal(name={},len={})", name, data.len())
            }
        }
    }
}

/// A Protocol stream for replicating hypercores over an encrypted connection.
///
/// The protocol expects an already-encrypted, message-framed connection
/// (e.g., from hyperswarm). The `HandshakeResult` provides the handshake hash
/// and public keys needed for capability verification.
pub struct Protocol {
    io: MessageIo,
    is_initiator: bool,
    channels: ChannelMap,
    command_rx: Receiver<Command>,
    command_tx: CommandTx,
    outbound_rx: Receiver<Vec<ChannelMessage>>,
    outbound_tx: Sender<Vec<ChannelMessage>>,
    #[allow(dead_code)] // TODO: Implement keepalive
    keepalive: Delay,
    queued_events: VecDeque<Event>,
    handshake_emitted: bool,
}

impl std::fmt::Debug for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Protocol")
            .field("is_initiator", &self.is_initiator)
            .field("channels", &self.channels)
            .field("handshake_emitted", &self.handshake_emitted)
            .field("queued_events", &self.queued_events)
            .finish()
    }
}

impl Protocol {
    /// Create a new protocol instance.
    ///
    /// # Arguments
    /// * `stream` - An already-encrypted, message-framed connection (e.g., hyperswarm `Connection`)
    pub fn new(stream: Box<dyn CipherTrait>) -> Self {
        let (command_tx, command_rx) = async_channel::bounded(CHANNEL_CAP);
        let (outbound_tx, outbound_rx): (
            Sender<Vec<ChannelMessage>>,
            Receiver<Vec<ChannelMessage>>,
        ) = async_channel::bounded(CHANNEL_CAP);

        let is_initiator = stream.is_initiator();

        Protocol {
            io: MessageIo::new(stream),
            is_initiator,
            channels: ChannelMap::new(),
            command_rx,
            command_tx: CommandTx(command_tx),
            outbound_tx,
            outbound_rx,
            keepalive: Delay::new(Duration::from_secs(DEFAULT_KEEPALIVE as u64)),
            queued_events: VecDeque::new(),
            handshake_emitted: false,
        }
    }

    /// Whether this protocol stream initiated the underlying IO connection.
    pub fn is_initiator(&self) -> bool {
        self.is_initiator
    }

    /// Get your own Noise public key.
    pub fn public_key(&self) -> [u8; PUBLIC_KEYLEN] {
        self.io.local_public_key()
    }

    /// Get the remote's Noise public key.
    pub fn remote_public_key(&self) -> Option<[u8; PUBLIC_KEYLEN]> {
        self.io.remote_public_key()
    }

    /// Get a sender to send commands.
    pub fn commands(&self) -> CommandTx {
        self.command_tx.clone()
    }

    /// Give a command to the protocol.
    pub async fn command(&self, command: Command) -> Result<()> {
        self.command_tx.send(command).await
    }

    /// Open a new protocol channel.
    ///
    /// Once the other side proofed that it also knows the `key`, the channel is emitted as
    /// `Event::Channel` on the protocol event stream.
    pub async fn open(&self, key: Key) -> Result<()> {
        self.command_tx.open(key).await
    }

    /// Iterator of all currently opened channels.
    pub fn channels(&self) -> impl Iterator<Item = &DiscoveryKey> {
        self.channels.iter().map(|c| c.discovery_key())
    }

    #[instrument(skip_all, fields(initiator = ?self.is_initiator()))]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<Event>> {
        let this = self.get_mut();

        // Initiator needs to send and receive a message before proceeding
        if this.is_initiator && this.io.handshake_hash().is_none() {
            return_error!(this.poll_outbound_write(cx));
            return_error!(this.poll_inbound_read(cx));
            if this.io.handshake_hash().is_none() {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        }
        // Emit handshake event on first poll
        if !this.handshake_emitted {
            if let Some(remote_pubkey) = this.io.remote_public_key() {
                this.handshake_emitted = true;
                return Poll::Ready(Ok(Event::Handshake(remote_pubkey)));
            } else {
                cx.waker().wake_by_ref();
            }
        }

        // Drain queued events first.
        if let Some(event) = this.queued_events.pop_front() {
            return Poll::Ready(Ok(event));
        }

        // Read and process incoming messages.
        return_error!(this.poll_inbound_read(cx));

        // Check for commands.
        return_error!(this.poll_commands(cx));

        // Poll the keepalive timer.
        this.poll_keepalive(cx);

        // Write everything we can write.
        return_error!(this.poll_outbound_write(cx));

        // Check if any events are enqueued.
        if let Some(event) = this.queued_events.pop_front() {
            Poll::Ready(Ok(event))
        } else {
            Poll::Pending
        }
    }

    /// Poll commands.
    fn poll_commands(&mut self, cx: &mut Context<'_>) -> Result<()> {
        while let Poll::Ready(Some(command)) = Pin::new(&mut self.command_rx).poll_next(cx) {
            if let Err(e) = self.on_command(command) {
                error!(error = ?e, "Error handling command");
                return Err(e);
            }
        }
        Ok(())
    }

    /// TODO Poll the keepalive timer and queue a ping message if needed.
    fn poll_keepalive(&self, _cx: &mut Context<'_>) {
        /*
        const KEEPALIVE_DURATION: Duration = Duration::from_secs(DEFAULT_KEEPALIVE as u64);
        if Pin::new(&mut self.keepalive).poll(cx).is_ready() {
            if let State::Established = self.state {
                // 24 bit header for the empty message, hence the 3
                self.write_state
                    .queue_frame(Frame::RawBatch(vec![vec![0u8; 3]]));
            }
            self.keepalive.reset(KEEPALIVE_DURATION);
        }
        */
    }

    // just handles Close and LocalSignal
    fn on_outbound_message(&mut self, message: &ChannelMessage) -> bool {
        // If message is close, close the local channel.
        if let ChannelMessage {
            channel,
            message: Message::Close(_),
            ..
        } = message
        {
            self.close_local(*channel);
        // If message is a LocalSignal, emit an event and return false to indicate
        // this message should be filtered out.
        } else if let ChannelMessage {
            message: Message::LocalSignal((name, data)),
            ..
        } = message
        {
            self.queue_event(Event::LocalSignal((name.to_string(), data.to_vec())));
            return false;
        }
        true
    }

    /// Poll for inbound messages and process them.
    #[instrument(skip_all, err)]
    fn poll_inbound_read(&mut self, cx: &mut Context<'_>) -> Result<()> {
        loop {
            match self.io.poll_inbound(cx) {
                Poll::Ready(Some(result)) => {
                    let messages = result?;
                    self.on_inbound_channel_messages(messages)?;
                }
                Poll::Ready(None) => return Ok(()),
                Poll::Pending => return Ok(()),
            }
        }
    }

    /// Poll for outbound messages and write them.
    #[instrument(skip_all)]
    fn poll_outbound_write(&mut self, cx: &mut Context<'_>) -> Result<()> {
        loop {
            // Drive outbound IO
            if let Poll::Ready(Err(e)) = self.io.poll_outbound(cx) {
                error!(err = ?e, "error from poll_outbound");
                return Err(e);
            }
            // Send messages from outbound_rx
            match Pin::new(&mut self.outbound_rx).poll_next(cx) {
                Poll::Ready(Some(mut messages)) => {
                    if !messages.is_empty() {
                        messages.retain(|message| self.on_outbound_message(message));
                        for msg in messages {
                            self.io.enqueue(msg);
                        }
                    }
                }
                Poll::Ready(None) => unreachable!("Channel closed before end"),
                Poll::Pending => return Ok(()),
            }
        }
    }

    #[instrument(skip_all)]
    fn on_inbound_channel_messages(&mut self, channel_messages: Vec<ChannelMessage>) -> Result<()> {
        for channel_message in channel_messages {
            self.on_inbound_message(channel_message)?
        }
        Ok(())
    }

    #[instrument(skip_all)]
    fn on_inbound_message(&mut self, channel_message: ChannelMessage) -> Result<()> {
        let (remote_id, message) = channel_message.into_split();
        match message {
            Message::Open(msg) => self.on_open(remote_id, msg)?,
            Message::Close(msg) => self.on_close(remote_id, msg)?,
            _ => self
                .channels
                .forward_inbound_message(remote_id as usize, message)?,
        }
        Ok(())
    }

    #[instrument(skip(self))]
    fn on_command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Open(key) => self.command_open(key),
            Command::Close(discovery_key) => self.command_close(discovery_key),
            Command::SignalLocal((name, data)) => self.command_signal_local(name, data),
        }
    }

    /// Open a Channel with the given key. Adding it to our channel map
    #[instrument(skip_all)]
    fn command_open(&mut self, key: Key) -> Result<()> {
        // Create a new channel.
        let channel_handle = self.channels.attach_local(key);
        // Safe because attach_local always puts Some(local_id)
        let local_id = channel_handle.local_id().unwrap();
        let discovery_key = *channel_handle.discovery_key();

        // If the channel was already opened from the remote end, verify, and if
        // verification is ok, push a channel open event.
        if channel_handle.is_connected() {
            self.accept_channel(local_id)?;
        }

        // Tell the remote end about the new channel.
        let capability = self.capability(&key);
        let channel = local_id as u64;
        let message = Message::Open(Open {
            channel,
            protocol: PROTOCOL_NAME.to_string(),
            discovery_key: discovery_key.to_vec(),
            capability,
        });
        let channel_message = ChannelMessage::new(channel, message);
        self.io.enqueue(channel_message);
        Ok(())
    }

    fn command_close(&mut self, discovery_key: DiscoveryKey) -> Result<()> {
        if self.channels.has_channel(&discovery_key) {
            self.channels.remove(&discovery_key);
            self.queue_event(Event::Close(discovery_key));
        }
        Ok(())
    }

    fn command_signal_local(&mut self, name: String, data: Vec<u8>) -> Result<()> {
        self.queue_event(Event::LocalSignal((name, data)));
        Ok(())
    }

    #[instrument(skip(self))]
    fn on_open(&mut self, ch: u64, msg: Open) -> Result<()> {
        let discovery_key: DiscoveryKey = parse_key(&msg.discovery_key)?;
        let channel_handle =
            self.channels
                .attach_remote(discovery_key, ch as usize, msg.capability);

        if channel_handle.is_connected() {
            let local_id = channel_handle.local_id().unwrap();
            self.accept_channel(local_id)?;
        } else {
            self.queue_event(Event::DiscoveryKey(discovery_key));
        }

        Ok(())
    }

    #[instrument(skip(self))]
    fn queue_event(&mut self, event: Event) {
        self.queued_events.push_back(event);
    }

    #[instrument(skip(self))]
    fn accept_channel(&mut self, local_id: usize) -> Result<()> {
        let (key, remote_capability) = self.channels.prepare_to_verify(local_id)?;
        self.verify_remote_capability(remote_capability.cloned(), key)
            .expect("TODO channel can only be accepted after first message")?;
        let channel = self.channels.accept(local_id, self.outbound_tx.clone())?;
        self.queue_event(Event::Channel(channel));
        Ok(())
    }

    fn close_local(&mut self, local_id: u64) {
        if let Some(channel) = self.channels.get_local(local_id as usize) {
            let discovery_key = *channel.discovery_key();
            self.channels.remove(&discovery_key);
            self.queue_event(Event::Close(discovery_key));
        }
    }

    fn on_close(&mut self, remote_id: u64, msg: Close) -> Result<()> {
        if let Some(channel_handle) = self.channels.get_remote(remote_id as usize) {
            let discovery_key = *channel_handle.discovery_key();
            // There is a possibility both sides will close at the same time, so
            // the channel could be closed already, let's tolerate that.
            self.channels
                .forward_inbound_message_tolerate_closed(remote_id as usize, Message::Close(msg))?;
            self.channels.remove(&discovery_key);
            self.queue_event(Event::Close(discovery_key));
        }
        Ok(())
    }

    #[instrument(skip_all)]
    fn capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        let is_initiator = self.is_initiator;
        let remote_pubkey = self.remote_public_key()?;
        let local_pubkey = self.public_key();
        let handshake_hash = self.io.handshake_hash()?;
        HandshakeResult::from_pre_encrypted(
            is_initiator,
            local_pubkey,
            remote_pubkey,
            handshake_hash.to_vec(),
        )
        .capability(key)
    }

    #[instrument(skip_all)]
    fn verify_remote_capability(
        &self,
        capability: Option<Vec<u8>>,
        key: &[u8],
    ) -> Option<Result<()>> {
        let is_initiator = self.is_initiator;
        let remote_pubkey = self.remote_public_key()?;
        let local_pubkey = self.public_key();
        let handshake_hash = self.io.handshake_hash()?;
        Some(
            HandshakeResult::from_pre_encrypted(
                is_initiator,
                local_pubkey,
                remote_pubkey,
                handshake_hash.to_vec(),
            )
            .verify_remote_capability(capability, key),
        )
    }
}

impl Stream for Protocol {
    type Item = Result<Event>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Protocol::poll_next(self, cx) {
            Poll::Ready(Ok(e)) => Poll::Ready(Some(Ok(e))),
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Send [`Command`]s to the [`Protocol`].
#[derive(Clone, Debug)]
pub struct CommandTx(Sender<Command>);

impl CommandTx {
    /// Send a protocol command
    pub async fn send(&self, command: Command) -> Result<()> {
        self.0.send(command).await.map_err(map_channel_err)
    }
    /// Open a protocol channel.
    ///
    /// The channel will be emitted on the main protocol.
    pub async fn open(&self, key: Key) -> Result<()> {
        self.send(Command::Open(key)).await
    }

    /// Close a protocol channel.
    pub async fn close(&self, discovery_key: DiscoveryKey) -> Result<()> {
        self.send(Command::Close(discovery_key)).await
    }

    /// Send a local signal event to the protocol.
    pub async fn signal_local(&self, name: &str, data: Vec<u8>) -> Result<()> {
        self.send(Command::SignalLocal((name.to_string(), data)))
            .await
    }
}

fn parse_key(key: &[u8]) -> io::Result<[u8; 32]> {
    key.try_into()
        .map_err(|_e| io::Error::new(io::ErrorKind::InvalidInput, "Key must be 32 bytes long"))
}
