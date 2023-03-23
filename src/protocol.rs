use async_channel::{Receiver, Sender};
use futures_lite::io::{AsyncRead, AsyncWrite};
use futures_lite::stream::Stream;
use futures_timer::Delay;
use std::collections::VecDeque;
use std::convert::TryInto;
use std::fmt;
use std::future::Future;
use std::io::{self, Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use crate::builder::{Builder, Options};
use crate::channels::{Channel, ChannelMap};
use crate::constants::{DEFAULT_KEEPALIVE, PROTOCOL_NAME};
use crate::crypto::{DecryptCipher, EncryptCipher, Handshake, HandshakeResult};
use crate::message::{ChannelMessage, EncodeError, Frame, FrameType, Message};
use crate::reader::ReadState;
use crate::schema::*;
use crate::util::{map_channel_err, pretty_hash};
use crate::writer::WriteState;

macro_rules! return_error {
    ($msg:expr) => {
        if let Err(e) = $msg {
            return Poll::Ready(Err(e));
        }
    };
}

const CHANNEL_CAP: usize = 1000;
const KEEPALIVE_DURATION: Duration = Duration::from_secs(DEFAULT_KEEPALIVE as u64);

/// Remote public key (32 bytes).
pub type RemotePublicKey = [u8; 32];
/// Discovery key (32 bytes).
pub type DiscoveryKey = [u8; 32];
/// Key (32 bytes).
pub type Key = [u8; 32];

/// A protocol event.
#[non_exhaustive]
#[derive(PartialEq)]
pub enum Event {
    /// Emitted after the handshake with the remove peer is complete.
    /// This is the first event (if the handshake is not disabled).
    Handshake(RemotePublicKey),
    /// Emitted when the remote peer opens a channel that we did not yet open.
    DiscoveryKey(DiscoveryKey),
    /// Emitted when a channel is established.
    Channel(Channel),
    /// Emitted when a channel is closed.
    Close(DiscoveryKey),
    /// Convenience event to make it possible to signal the protocol from a channel.
    /// See channel.signal_local().
    LocalSignal((String, Vec<u8>)),
}

/// A protocol command.
#[derive(Debug)]
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
            Event::LocalSignal((name, data)) => {
                write!(f, "LocalSignal(name={},len={})", name, data.len())
            }
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
    SecretStream(Option<EncryptCipher>),
    Established,
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            State::NotInitialized => write!(f, "NotInitialized"),
            State::Handshake(_) => write!(f, "Handshaking"),
            State::SecretStream(_) => write!(f, "SecretStream"),
            State::Established => write!(f, "Established"),
        }
    }
}

/// A Protocol stream.
///
#[derive(Debug)]
pub struct Protocol<IO> {
    write_state: WriteState,
    read_state: ReadState,
    io: IO,
    state: State,
    options: Options,
    handshake: Option<HandshakeResult>,
    channels: ChannelMap,
    command_rx: Receiver<Command>,
    command_tx: CommandTx,
    outbound_rx: Receiver<Vec<ChannelMessage>>,
    outbound_tx: Sender<Vec<ChannelMessage>>,
    keepalive: Delay,
    queued_events: VecDeque<Event>,
}

impl<IO> Protocol<IO>
where
    IO: AsyncWrite + AsyncRead + Send + Unpin + 'static,
{
    /// Create a new protocol instance.
    pub fn new(io: IO, options: Options) -> Self {
        let (command_tx, command_rx) = async_channel::bounded(CHANNEL_CAP);
        let (outbound_tx, outbound_rx): (
            Sender<Vec<ChannelMessage>>,
            Receiver<Vec<ChannelMessage>>,
        ) = async_channel::bounded(1);
        Protocol {
            io,
            read_state: ReadState::new(),
            write_state: WriteState::new(),
            options,
            state: State::NotInitialized,
            channels: ChannelMap::new(),
            handshake: None,
            command_rx,
            command_tx: CommandTx(command_tx),
            outbound_tx,
            outbound_rx,
            keepalive: Delay::new(Duration::from_secs(DEFAULT_KEEPALIVE as u64)),
            queued_events: VecDeque::new(),
        }
    }

    /// Create a protocol instance with the default options.
    pub fn with_defaults(io: IO, is_initiator: bool) -> Self {
        let options = Options::new(is_initiator);
        Protocol::new(io, options)
    }

    /// Create a protocol builder that allows to set additional options.
    pub fn builder(is_initiator: bool) -> Builder {
        Builder::new(is_initiator)
    }

    /// Whether this protocol stream initiated the underlying IO connection.
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

    /// Iterator of all currently opened channels.
    pub fn channels(&self) -> impl Iterator<Item = &DiscoveryKey> {
        self.channels.iter().map(|c| c.discovery_key())
    }

    /// Stop the protocol and return the inner reader and writer.
    pub fn release(self) -> IO {
        self.io
    }

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<Event>> {
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
        }

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

    fn init(&mut self) -> Result<()> {
        tracing::debug!(
            "protocol init, state {:?}, options {:?}",
            self.state,
            self.options
        );
        match self.state {
            State::NotInitialized => {}
            _ => return Ok(()),
        };

        self.state = if self.options.noise {
            let mut handshake = Handshake::new(self.options.is_initiator)?;
            // If the handshake start returns a buffer, send it now.
            if let Some(buf) = handshake.start()? {
                self.queue_frame_direct(buf.to_vec()).unwrap();
            }
            self.read_state.set_frame_type(FrameType::Raw);
            State::Handshake(Some(handshake))
        } else {
            self.read_state.set_frame_type(FrameType::Message);
            State::Established
        };

        Ok(())
    }

    /// Poll commands.
    fn poll_commands(&mut self, cx: &mut Context<'_>) -> Result<()> {
        while let Poll::Ready(Some(command)) = Pin::new(&mut self.command_rx).poll_next(cx) {
            self.on_command(command)?;
        }
        Ok(())
    }

    /// Poll the keepalive timer and queue a ping message if needed.
    fn poll_keepalive(&mut self, cx: &mut Context<'_>) {
        if Pin::new(&mut self.keepalive).poll(cx).is_ready() {
            if let State::Established = self.state {
                // 24 bit header for the empty message, hence the 3
                self.write_state
                    .queue_frame(Frame::RawBatch(vec![vec![0u8; 3]]));
            }
            self.keepalive.reset(KEEPALIVE_DURATION);
        }
    }

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

    /// Poll for inbound messages and processs them.
    fn poll_inbound_read(&mut self, cx: &mut Context<'_>) -> Result<()> {
        loop {
            let msg = self.read_state.poll_reader(cx, &mut self.io);
            match msg {
                Poll::Ready(Ok(message)) => {
                    self.on_inbound_frame(message)?;
                }
                Poll::Ready(Err(e)) => return Err(e),
                Poll::Pending => return Ok(()),
            }
        }
    }

    /// Poll for outbound messages and write them.
    fn poll_outbound_write(&mut self, cx: &mut Context<'_>) -> Result<()> {
        loop {
            if let Poll::Ready(Err(e)) = self.write_state.poll_send(cx, &mut self.io) {
                return Err(e);
            }
            if !self.write_state.can_park_frame() || !matches!(self.state, State::Established) {
                return Ok(());
            }

            match Pin::new(&mut self.outbound_rx).poll_next(cx) {
                Poll::Ready(Some(mut messages)) => {
                    if !messages.is_empty() {
                        messages.retain(|message| self.on_outbound_message(&message));
                        if !messages.is_empty() {
                            let frame = Frame::MessageBatch(messages);
                            self.write_state.park_frame(frame);
                        }
                    }
                }
                Poll::Ready(None) => unreachable!("Channel closed before end"),
                Poll::Pending => return Ok(()),
            }
        }
    }

    fn on_inbound_frame(&mut self, frame: Frame) -> Result<()> {
        match frame {
            Frame::RawBatch(raw_batch) => {
                let mut processed_state: Option<String> = None;
                for buf in raw_batch {
                    let state_name: String = format!("{:?}", self.state);
                    match self.state {
                        State::Handshake(_) => self.on_handshake_message(buf)?,
                        State::SecretStream(_) => self.on_secret_stream_message(buf)?,
                        State::Established => {
                            if let Some(processed_state) = processed_state.as_ref() {
                                let previous_state = if self.options.encrypted {
                                    State::SecretStream(None)
                                } else {
                                    State::Handshake(None)
                                };
                                if processed_state == &format!("{:?}", previous_state) {
                                    // This is the unlucky case where the batch had two or more messages where
                                    // the first one was correctly identified as Raw but everything
                                    // after that should have been (decrypted and) a MessageBatch. Correct the mistake
                                    // here post-hoc.
                                    let buf = self.read_state.decrypt_buf(&buf)?;
                                    let frame = Frame::decode(&buf, &FrameType::Message)?;
                                    self.on_inbound_frame(frame)?;
                                    continue;
                                }
                            }
                            unreachable!(
                                "May not receive raw frames in Established state"
                            )
                        }
                        _ => unreachable!(
                            "May not receive raw frames outside of handshake or secretstream state, was {:?}",
                            self.state
                        ),
                    };
                    if processed_state.is_none() {
                        processed_state = Some(state_name)
                    }
                }
                Ok(())
            }
            Frame::MessageBatch(channel_messages) => match self.state {
                State::Established => {
                    for channel_message in channel_messages {
                        self.on_inbound_message(channel_message)?
                    }
                    Ok(())
                }
                _ => unreachable!("May not receive message batch frames when not established"),
            },
        }
    }

    fn on_handshake_message(&mut self, buf: Vec<u8>) -> Result<()> {
        let mut handshake = match &mut self.state {
            State::Handshake(handshake) => handshake.take().unwrap(),
            _ => unreachable!("May not call on_handshake_message when not in Handshake state"),
        };

        if let Some(response_buf) = handshake.read(&buf)? {
            self.queue_frame_direct(response_buf.to_vec()).unwrap();
        }

        if !handshake.complete() {
            self.state = State::Handshake(Some(handshake));
        } else {
            let handshake_result = handshake.into_result()?;

            if self.options.encrypted {
                // The cipher will be put to use to the writer only after the peer's answer has come
                let (cipher, init_msg) = EncryptCipher::from_handshake_tx(&handshake_result)?;
                self.state = State::SecretStream(Some(cipher));

                // Send the secret stream init message header to the other side
                self.queue_frame_direct(init_msg).unwrap();
            } else {
                // Skip secret stream and go straight to Established, then notify about
                // handshake
                self.read_state.set_frame_type(FrameType::Message);
                let remote_public_key = parse_key(&handshake_result.remote_pubkey)?;
                self.queue_event(Event::Handshake(remote_public_key));
                self.state = State::Established;
            }
            // Store handshake result
            self.handshake = Some(handshake_result);
        }
        Ok(())
    }

    fn on_secret_stream_message(&mut self, buf: Vec<u8>) -> Result<()> {
        let encrypt_cipher = match &mut self.state {
            State::SecretStream(encrypt_cipher) => encrypt_cipher.take().unwrap(),
            _ => {
                unreachable!("May not call on_secret_stream_message when not in SecretStream state")
            }
        };
        let handshake_result = &self
            .handshake
            .as_ref()
            .expect("Handshake result must be set before secret stream");
        let decrypt_cipher = DecryptCipher::from_handshake_rx_and_init_msg(handshake_result, &buf)?;
        self.read_state.upgrade_with_decrypt_cipher(decrypt_cipher);
        self.write_state.upgrade_with_encrypt_cipher(encrypt_cipher);
        self.read_state.set_frame_type(FrameType::Message);

        // Lastly notify that handshake is ready and set state to established
        let remote_public_key = parse_key(&handshake_result.remote_pubkey)?;
        self.queue_event(Event::Handshake(remote_public_key));
        self.state = State::Established;
        Ok(())
    }

    fn on_inbound_message(&mut self, channel_message: ChannelMessage) -> Result<()> {
        // let channel_message = ChannelMessage::decode(buf)?;
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

    fn on_command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Open(key) => self.command_open(key),
            _ => Ok(()),
        }
    }

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
        self.write_state
            .queue_frame(Frame::MessageBatch(vec![channel_message]));
        Ok(())
    }

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

    fn queue_event(&mut self, event: Event) {
        self.queued_events.push_back(event);
    }

    fn queue_frame_direct(&mut self, body: Vec<u8>) -> std::result::Result<bool, EncodeError> {
        let mut frame = Frame::RawBatch(vec![body]);
        self.write_state.try_queue_direct(&mut frame)
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

impl<IO> Stream for Protocol<IO>
where
    IO: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Item = Result<Event>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Protocol::poll_next(self, cx).map(Some)
    }
}

/// Send [Command](Command)s to the [Protocol](Protocol).
#[derive(Clone, Debug)]
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
