use futures::channel::mpsc::{Receiver, Sender};
use futures::future::{Fuse, FutureExt};
use futures::io::{AsyncRead, AsyncWrite};
use futures::io::{BufReader, BufWriter};
use futures::stream::{SelectAll, Stream, StreamExt};
use futures_timer::Delay;
use log::*;
use std::collections::VecDeque;
use std::fmt;
use std::io::{Error, ErrorKind, Result};
use std::time::Duration;

use crate::channels::{Channel, Channelizer};
use crate::constants::DEFAULT_KEEPALIVE;
use crate::message::{ChannelMessage, Message};
use crate::noise::{Handshake, HandshakeResult};
use crate::reader::ProtocolReader;
use crate::schema::*;
use crate::util::pretty_hash;
use crate::writer::ProtocolWriter;

const CHANNEL_CAP: usize = 1000;
const KEEPALIVE_DURATION: Duration = Duration::from_secs(DEFAULT_KEEPALIVE as u64);

/// A protocol event.
pub enum Event {
    Handshake(Vec<u8>),
    DiscoveryKey(Vec<u8>),
    Channel(Channel),
    Close(Vec<u8>),
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
            Event::Close(discovery_key) => write!(f, "Close({})", &pretty_hash(discovery_key)),
            Event::Channel(channel) => write!(f, "{:?}", channel),
        }
    }
}

/// Options for a Protocol instance.
#[derive(Debug)]
pub struct ProtocolOptions {
    pub is_initiator: bool,
    pub noise: bool,
    pub encrypted: bool,
}

/// Build a Protocol instance with options.
pub struct ProtocolBuilder(ProtocolOptions);

impl ProtocolBuilder {
    pub fn new(is_initiator: bool) -> Self {
        Self(ProtocolOptions {
            is_initiator,
            noise: true,
            encrypted: true,
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
            State::NotInitialized => write!(f, "NotInitialized"),
            State::Handshake(_) => write!(f, "Handshaking"),
            State::Established => write!(f, "Established"),
        }
    }
}

/// The output of the set of channel senders.
type CombinedOutputStream = SelectAll<Box<dyn Stream<Item = ChannelMessage> + Send + Unpin>>;

/// A Protocol stream.
pub struct Protocol<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    writer: ProtocolWriter<BufWriter<W>>,
    reader: ProtocolReader<BufReader<R>>,
    state: State,
    options: ProtocolOptions,
    handshake: Option<HandshakeResult>,
    channels: Channelizer,
    error: Option<Error>,
    outbound_rx: CombinedOutputStream,
    control_rx: Receiver<stream::ControlEvent>,
    control_tx: Sender<stream::ControlEvent>,
    events: VecDeque<Event>,
    keepalive: Option<Fuse<Delay>>,
}

impl<R, W> Protocol<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    /// Create a new Protocol instance.
    pub fn new(reader: R, writer: W, options: ProtocolOptions) -> Self {
        let reader = ProtocolReader::new(BufReader::new(reader));
        let writer = ProtocolWriter::new(BufWriter::new(writer));
        let (control_tx, control_rx) = futures::channel::mpsc::channel(CHANNEL_CAP);
        Protocol {
            writer,
            reader,
            options,
            state: State::NotInitialized,
            channels: Channelizer::new(),
            handshake: None,
            error: None,
            outbound_rx: SelectAll::new(),
            control_rx,
            control_tx,
            events: VecDeque::new(),
            keepalive: None,
        }
    }

    pub async fn init(&mut self) -> Result<()> {
        trace!(
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
                self.writer.send_prefixed(buf).await?;
            }
            State::Handshake(Some(handshake))
        } else {
            State::Established
        };

        self.reset_keepalive();

        Ok(())
    }

    fn reset_keepalive(&mut self) {
        let keepalive_duration = Duration::from_secs(DEFAULT_KEEPALIVE as u64);
        self.keepalive = Some(Delay::new(keepalive_duration).fuse());
    }

    pub fn is_initiator(&self) -> bool {
        self.options.is_initiator
    }

    /// Wait for the next protocol event.
    ///
    /// This function should be called in a loop until this returns an error.
    pub async fn loop_next(&mut self) -> Result<Event> {
        // trace!(
        //     "[{}] loop_next start, state {:?} events.len {}",
        //     self.is_initiator(),
        //     self.state,
        //     self.events.len()
        // );
        if let State::NotInitialized = self.state {
            self.init().await?;
        }

        let mut keepalive = if let Some(keepalive) = self.keepalive.take() {
            keepalive
        } else {
            Delay::new(KEEPALIVE_DURATION).fuse()
        };

        // Wait for new bytes to arrive, or for the keepalive to occur to send a ping.
        // If data was received, reset the keepalive timer.
        loop {
            // trace!("[{}] loop_next loop in", self.is_initiator());

            if let Some(event) = self.events.pop_front() {
                return Ok(event);
            }

            let event = futures::select! {
                _ = keepalive => {
                    // trace!("[{}] loop_next ! keepalive", self.is_initiator());
                    self.ping().await?;
                    // TODO: It would be better to `reset` the keepalive and not recreate it.
                    // I couldn't get this to work with `fuse()` though which is needed for
                    // the `select!` macro.
                    keepalive = Delay::new(KEEPALIVE_DURATION).fuse();
                    None
                },
                buf = self.reader.select_next_some() => {
                    // trace!("[{}] loop_next ! incoming message", self.is_initiator());
                    self.on_message(buf?).await?
                },
                channel_message = self.outbound_rx.select_next_some() => {
                    // trace!("[{}] loop_next ! outbound_rx", self.is_initiator());
                    let event = match channel_message {
                        ChannelMessage { channel, message: Message::Close(_) } => {
                            self.close_local(channel).await?
                        },
                        _ => None
                    };
                    self.send(channel_message).await?;
                    // trace!("[{}] loop_next ! outbound_rx SENT", self.is_initiator());
                    event
                },
                ev = self.control_rx.select_next_some() => {
                    // trace!("[{}] loop_next ! control_rx", self.is_initiator());
                    match ev {
                        stream::ControlEvent::Open(key) => {
                            self.open(key).await?;
                            None
                        }
                    }
                },
            };
            // trace!(
            //     "[{}] loop_next loop out, event {:?}",
            //     self.is_initiator(),
            //     event
            // );
            if let Some(event) = event {
                self.keepalive = Some(keepalive);
                return Ok(event);
            }
        }
    }

    /// Get the peer's Noise public key.
    ///
    /// Empty before the handshake completed.
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

    async fn on_message(&mut self, buf: Vec<u8>) -> Result<Option<Event>> {
        // trace!("onmessage, state {:?} msg len {}", self.state, buf.len());
        match self.state {
            State::Handshake(_) => self.on_handshake_message(buf).await,
            State::Established => self.on_proto_message(buf).await,
            State::NotInitialized => panic!("cannot receive messages before starting the protocol"),
        }
    }

    async fn on_handshake_message(&mut self, buf: Vec<u8>) -> Result<Option<Event>> {
        let mut handshake = match &mut self.state {
            State::Handshake(handshake) => handshake.take().unwrap(),
            _ => panic!("cannot call on_handshake_message when not in Handshake state"),
        };
        if let Some(response_buf) = handshake.read(&buf)? {
            self.writer.send_prefixed(response_buf).await?;
        }
        if !handshake.complete() {
            self.state = State::Handshake(Some(handshake));
            Ok(None)
        } else {
            let result = handshake.into_result()?;
            if self.options.encrypted {
                self.reader.upgrade_with_handshake(&result)?;
                self.writer.upgrade_with_handshake(&result)?;
            }
            let remote_key = result.remote_pubkey.to_vec();
            log::trace!(
                "handshake complete, remote_key {}",
                pretty_hash(&remote_key)
            );
            self.handshake = Some(result);
            self.state = State::Established;
            Ok(Some(Event::Handshake(remote_key)))
        }
    }

    async fn on_proto_message(&mut self, buf: Vec<u8>) -> Result<Option<Event>> {
        let channel_message = ChannelMessage::decode(buf)?;
        log::trace!("recv {:?}", channel_message);
        let (channel, message) = channel_message.into_split();
        match message {
            Message::Open(msg) => self.on_open(channel, msg).await,
            Message::Close(msg) => self.on_close(channel, msg).await,
            Message::Extension(_msg) => unimplemented!(),
            _ => {
                self.channels.forward(channel as usize, message).await?;
                Ok(None)
            }
        }
    }

    /// Open a new protocol channel.
    ///
    /// Once the other side proofed that it also knows the `key`, the channel is emitted as
    /// `Event::Channel` on the protocol event stream.
    pub async fn open(&mut self, key: Vec<u8>) -> Result<()> {
        // Create a new channel.
        let channel_info = self.channels.attach_local(key.clone());
        // Safe because attach_local always puts Some(local_id)
        let local_id = channel_info.local_id.unwrap();
        let discovery_key = channel_info.discovery_key.clone();

        // If the channel was already opened from the remote end, verify, and if
        // verification is ok, push a channel open event.
        if let Some(_remote_id) = channel_info.remote_id {
            let remote_capability = channel_info.remote_capability.clone();
            self.verify_remote_capability(remote_capability, &key)?;
            let channel = self.create_channel(local_id).await?;
            self.events.push_back(Event::Channel(channel));
        }

        // Tell the remote end about the new channel.
        let capability = self.capability(&key);
        let message = Message::Open(Open {
            discovery_key,
            capability,
        });
        let channel_message = ChannelMessage::new(local_id as u64, message);
        self.outbound_rx.push(Box::new(
            futures::future::ready(channel_message).into_stream(),
        ));

        Ok(())
    }

    async fn on_open(&mut self, ch: u64, msg: Open) -> Result<Option<Event>> {
        let inner_channel = self.channels.attach_remote(
            msg.discovery_key.clone(),
            ch as usize,
            msg.capability.clone(),
        );

        // This means there is not yet a locally-opened channel for this discovery_key.
        if let Some(local_id) = inner_channel.local_id {
            let key = inner_channel.key.as_ref().unwrap().clone();
            self.verify_remote_capability(msg.capability, &key)?;
            let channel = self.create_channel(local_id).await?;
            Ok(Some(Event::Channel(channel)))
        } else {
            Ok(Some(Event::DiscoveryKey(msg.discovery_key.clone())))
        }
    }

    async fn create_channel(&mut self, local_id: usize) -> Result<Channel> {
        let inner_channel = self.channels.get_local_mut(local_id).unwrap();
        let (channel, send_rx) = inner_channel.open().await?;
        self.outbound_rx.push(Box::new(send_rx));
        Ok(channel)
    }

    async fn on_close(&mut self, ch: u64, msg: Close) -> Result<Option<Event>> {
        self.close_remote(ch, msg).await
    }

    async fn close_local(&mut self, local_id: u64) -> Result<Option<Event>> {
        if let Some(channel) = self.channels.get_local_mut(local_id as usize) {
            let discovery_key = channel.discovery_key.clone();
            channel.recv_close(None).await?;
            self.channels.remove(&discovery_key);
            Ok(Some(Event::Close(discovery_key)))
        } else {
            Ok(None)
        }
    }

    async fn close_remote(&mut self, remote_id: u64, msg: Close) -> Result<Option<Event>> {
        if let Some(channel) = self.channels.get_remote_mut(remote_id as usize) {
            let discovery_key = channel.discovery_key.clone();
            channel.recv_close(Some(msg)).await?;
            self.channels.remove(&discovery_key);
            Ok(Some(Event::Close(discovery_key)))
        } else {
            Ok(None)
        }
    }

    pub(crate) async fn send(&mut self, channel_message: ChannelMessage) -> Result<()> {
        log::trace!("send {:?}", channel_message);
        let buf = channel_message.encode()?;
        self.writer.send_prefixed(&buf).await
    }

    async fn ping(&mut self) -> Result<()> {
        self.writer.ping().await
    }

    /// Stop the protocol and return the inner reader and writer.
    pub fn release(self) -> (R, W) {
        (
            self.reader.into_inner().into_inner(),
            self.writer.into_inner().into_inner(),
        )
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

    pub fn into_stream(self) -> stream::ProtocolStream<R, W> {
        let tx = self.control_tx.clone();
        stream::ProtocolStream::new(self, tx)
    }
}

pub use stream::ProtocolStream;
mod stream {
    use crate::util::map_channel_err;
    use crate::{Event, Protocol};
    use futures::channel::mpsc::Sender;
    use futures::future::FutureExt;
    use futures::io::{AsyncRead, AsyncWrite};
    use futures::sink::SinkExt;
    use futures::stream::Stream;
    use std::future::Future;
    use std::io::Result;
    use std::pin::Pin;
    use std::task::Poll;

    #[derive(Debug)]
    pub enum ControlEvent {
        Open(Vec<u8>),
    }

    type LoopFuture<R, W> = Pin<Box<dyn Future<Output = (Result<Event>, Protocol<R, W>)> + Send>>;

    async fn loop_next<R, W>(mut protocol: Protocol<R, W>) -> (Result<Event>, Protocol<R, W>)
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let event = protocol.loop_next().await;
        (event, protocol)
    }

    /// Event stream interface for a Protocol instance.
    pub struct ProtocolStream<R, W>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        fut: LoopFuture<R, W>,
        tx: Sender<ControlEvent>,
    }

    impl<R, W> ProtocolStream<R, W>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        pub fn new(protocol: Protocol<R, W>, tx: Sender<ControlEvent>) -> Self {
            let fut = loop_next(protocol).boxed();
            Self { fut, tx }
        }

        pub async fn open(&mut self, key: Vec<u8>) -> Result<()> {
            self.tx
                .send(ControlEvent::Open(key))
                .await
                .map_err(map_channel_err)
        }
    }

    impl<R, W> Stream for ProtocolStream<R, W>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        type Item = Result<Event>;
        fn poll_next(
            mut self: Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> Poll<Option<Self::Item>> {
            let fut = Pin::as_mut(&mut self.fut);
            match fut.poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(result) => {
                    let (result, protocol) = result;
                    self.fut = loop_next(protocol).boxed();
                    Poll::Ready(Some(result))
                }
            }
        }
    }
}
