use futures::channel::mpsc::{Receiver, Sender};
use futures::future::FutureExt;
use futures::io::{AsyncRead, AsyncWrite};
use futures::io::{BufReader, BufWriter};
use futures::ready;
use futures::sink::SinkExt;
use futures::stream::{SelectAll, Stream};
use futures::task::{Context, Poll};
use futures_timer::Delay;
use log::*;
use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::time::Duration;

use crate::builder::{Builder, Options};
use crate::channels::{Channel, Channelizer};
use crate::constants::DEFAULT_KEEPALIVE;
use crate::message::{ChannelMessage, Message};
use crate::noise::{Handshake, HandshakeResult};
use crate::reader::ProtocolReader;
use crate::schema::*;
use crate::util::map_channel_err;
use crate::util::pretty_hash;
use crate::writer::ProtocolWriter;

const CHANNEL_CAP: usize = 1000;
const KEEPALIVE_DURATION: Duration = Duration::from_secs(DEFAULT_KEEPALIVE as u64);

pub type RemotePublicKey = Vec<u8>;
pub type DiscoveryKey = Vec<u8>;
pub type Key = Vec<u8>;
pub type ChannelId = usize;

/// A protocol event.
pub enum Event {
    Handshake(RemotePublicKey),
    DiscoveryKey(DiscoveryKey),
    Open(ChannelId, DiscoveryKey),
    Message(ChannelId, Message),
    Close(ChannelId),
    Error(std::io::Error),
}

pub enum Action {
    Open(Key),
    Close(DiscoveryKey),
    Send(ChannelId, Message),
}

// struct Channelizzer<R, W>
// where
//     R: AsyncRead + Send + Unpin + 'static,
//     W: AsyncWrite + Send + Unpin + 'static,
// {
//     protocol: Protocol<R, W>,
// }
// impl Stream for Channelizer {
//     type Item = C
//     fn poll_next(
//         mut self: Pin<&mut Self>,
//         cx: &mut std::task::Context<'_>,
//     ) -> Poll<Option<Self::Item>> {
//     }
// }

impl fmt::Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Handshake(remote_key) => {
                write!(f, "Handshake(remote_key={})", &pretty_hash(remote_key))
            }
            Event::DiscoveryKey(discovery_key) => {
                write!(f, "DiscoveryKey({})", &pretty_hash(discovery_key))
            }
            Event::Close(channel) => write!(f, "Close({})", channel),
            Event::Open(channel, discovery_key) => {
                write!(f, "Open({}, {})", channel, &pretty_hash(discovery_key))
            }
            Event::Message(channel, message) => write!(f, "{:?} {:?}", channel, message),
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

/// The output of the set of channel senders.
type OutboundMessagesReceiver = SelectAll<Box<dyn Stream<Item = ChannelMessage> + Send + Unpin>>;

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
    channels: Channelizer,
    error: Option<Error>,
    outbound_rx: OutboundMessagesReceiver,
    control_rx: Receiver<ControlEvent>,
    control_tx: ControlTx,
    // outgoing: VecDeque<Event>,
    keepalive: Delay,

    // new fields
    queued_events: VecDeque<Event>,
    queued_messages: VecDeque<Vec<u8>>,
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
            control_tx: ControlTx(control_tx),
            // outgoing: VecDeque::new(),
            keepalive: Delay::new(Duration::from_secs(DEFAULT_KEEPALIVE as u64)),
            // new fields
            //
            queued_events: VecDeque::new(),
            queued_messages: VecDeque::new(),
        }
    }

    /// Create a protocol builder.
    pub fn builder(is_initiator: bool) -> Builder {
        Builder::new(is_initiator)
    }

    fn init(&mut self) -> Result<()> {
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
                self.queue_message(buf.to_vec());
            }
            State::Handshake(Some(handshake))
        } else {
            State::Established
        };

        // self.reset_keepalive();

        Ok(())
    }

    // fn reset_keepalive(&mut self) {
    //     let keepalive_duration = Duration::from_secs(DEFAULT_KEEPALIVE as u64);
    //     self.keepalive = Some(Delay::new(keepalive_duration).fuse());
    // }

    pub fn is_initiator(&self) -> bool {
        self.options.is_initiator
    }

    pub fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<Event>> {
        let this = self.get_mut();

        if let State::NotInitialized = this.state {
            match this.init() {
                Ok(_) => {}
                Err(e) => return Poll::Ready(Err(e)),
            }
        }

        loop {
            // Drain queued events first.
            if let Some(event) = this.queued_events.pop_front() {
                return Poll::Ready(Ok(event));
            }

            // Send queued messages out.
            while !this.queued_messages.is_empty() {
                let res = ready!(Pin::new(&mut this.writer)
                    .poll_write_message(this.queued_messages.front().unwrap(), cx));
                this.writer.reset();
                this.queued_messages.pop_front();
                if let Err(e) = res {
                    this.queued_events.push_back(Event::Error(e));
                    continue;
                }
            }

            // Check for outgoing messages.
            match Pin::new(&mut this.outbound_rx).poll_next(cx) {
                Poll::Pending => {}
                // TODO: Can this actually happen?
                Poll::Ready(None) => {}
                Poll::Ready(Some(channel_message)) => {
                    // If message is close, close the local channel.
                    match channel_message {
                        ChannelMessage {
                            channel,
                            message: Message::Close(_),
                        } => {
                            this.close_local(channel);
                        }
                        _ => {}
                    };
                    let res = this.queue_channel_message(channel_message);
                    match res {
                        Err(e) => return Poll::Ready(Err(e)),
                        _ => {}
                    }
                }
            }

            // Poll the keepalive timer.
            match Future::poll(Pin::new(&mut this.keepalive), cx) {
                Poll::Pending => {}
                Poll::Ready(_) => {
                    this.queue_ping();
                    this.keepalive.reset(KEEPALIVE_DURATION);
                }
            }

            // Wait for incoming messages.
            while let Poll::Ready(Some(message)) = Stream::poll_next(Pin::new(&mut this.reader), cx)
            {
                let res = match message {
                    Ok(message) => this.on_message(message),
                    Err(e) => Err(e),
                };
                if let Err(e) = res {
                    this.queued_events.push_back(Event::Error(e));
                    continue;
                }
            }

            // Check if any events are enqueued.
            if this.queued_events.is_empty() {
                return Poll::Pending;
            }
        }
    }

    pub async fn loop_next(mut self: &mut Self) -> Result<Event> {
        let fut = futures::future::poll_fn(move |cx| Protocol::poll_next(Pin::new(&mut self), cx));
        fut.await
    }

    /// Wait for the next protocol event.
    ///
    /// This function should be called in a loop until this returns an error.
    // pub async fn loop_next(&mut self) -> Result<Event> {
    //     // trace!(
    //     //     "[{}] loop_next start, state {:?} events.len {}",
    //     //     self.is_initiator(),
    //     //     self.state,
    //     //     self.events.len()
    //     // );
    //     if let State::NotInitialized = self.state {
    //         self.init().await?;
    //     }

    //     let mut keepalive = if let Some(keepalive) = self.keepalive.take() {
    //         keepalive
    //     } else {
    //         Delay::new(KEEPALIVE_DURATION).fuse()
    //     };

    //     // Wait for new bytes to arrive, or for the keepalive to occur to send a ping.
    //     // If data was received, reset the keepalive timer.
    //     loop {
    //         // trace!("[{}] loop_next loop in", self.is_initiator());

    //         if let Some(event) = self.events.pop_front() {
    //             return Ok(event);
    //         }

    //         let event = futures::select! {
    //             // Keepalive timer for pings
    //             _ = keepalive => {
    //                 // trace!("[{}] loop_next ! keepalive", self.is_initiator());
    //                 self.ping().await?;
    //                 // TODO: It would be better to `reset` the keepalive and not recreate it.
    //                 // I couldn't get this to work with `fuse()` though which is needed for
    //                 // the `select!` macro.
    //                 keepalive = Delay::new(KEEPALIVE_DURATION).fuse();
    //                 None
    //             },
    //             // New wire message incoming
    //             buf = self.reader.select_next_some() => {
    //                 // trace!("[{}] loop_next ! incoming message", self.is_initiator());
    //                 self.on_message(buf?).await?
    //             },
    //             // New outbound message
    //             channel_message = self.outbound_rx.select_next_some() => {
    //                 // trace!("[{}] loop_next ! outbound_rx", self.is_initiator());
    //                 let event = match channel_message {
    //                     ChannelMessage { channel, message: Message::Close(_) } => {
    //                         self.close_local(channel).await?
    //                     },
    //                     _ => None
    //                 };
    //                 self.send(channel_message).await?;
    //                 // trace!("[{}] loop_next ! outbound_rx SENT", self.is_initiator());
    //                 event
    //             },
    //             // New control message
    //             ev = self.control_rx.select_next_some() => {
    //                 // trace!("[{}] loop_next ! control_rx", self.is_initiator());
    //                 match ev {
    //                     ControlEvent::Open(key) => {
    //                         self.open(key).await?;
    //                         None
    //                     }
    //                 }
    //             },
    //         };
    //         // trace!(
    //         //     "[{}] loop_next loop out, event {:?}",
    //         //     self.is_initiator(),
    //         //     event
    //         // );
    //         if let Some(event) = event {
    //             self.keepalive = Some(keepalive);
    //             return Ok(event);
    //         }
    //     }
    // }

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

    fn on_message(&mut self, buf: Vec<u8>) -> Result<()> {
        // trace!("onmessage, state {:?} msg len {}", self.state, buf.len());
        match self.state {
            State::Handshake(_) => self.on_handshake_message(buf)?,
            State::Established => self.on_proto_message(buf)?,
            State::NotInitialized => panic!("cannot receive messages before starting the protocol"),
        };
        Ok(())
    }

    fn on_handshake_message(&mut self, buf: Vec<u8>) -> Result<()> {
        let mut handshake = match &mut self.state {
            State::Handshake(handshake) => handshake.take().unwrap(),
            _ => panic!("cannot call on_handshake_message when not in Handshake state"),
        };
        if let Some(response_buf) = handshake.read(&buf)? {
            self.queue_message(response_buf.to_vec());
        }
        if !handshake.complete() {
            self.state = State::Handshake(Some(handshake));
            Ok(())
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
            self.queue_event(Event::Handshake(remote_key));
            Ok(())
            // Ok(Some(Event::Handshake(remote_key)))
        }
    }

    fn on_proto_message(&mut self, buf: Vec<u8>) -> Result<()> {
        let channel_message = ChannelMessage::decode(buf)?;
        log::trace!("recv {:?}", channel_message);
        let (remote_id, message) = channel_message.into_split();
        match message {
            Message::Open(msg) => self.on_open(remote_id, msg),
            Message::Close(msg) => self.on_close(remote_id, msg),
            Message::Extension(_msg) => unimplemented!(),
            _ => self.on_channel_message(remote_id as usize, message),
        }
    }

    /// Open a new protocol channel.
    ///
    /// Once the other side proofed that it also knows the `key`, the channel is emitted as
    /// `Event::Channel` on the protocol event stream.
    pub fn open(&mut self, key: Vec<u8>) -> Result<()> {
        // Create a new channel.
        let inner_channel = self.channels.attach_local(key.clone());
        // Safe because attach_local always puts Some(local_id)
        let local_id = inner_channel.local_id.unwrap();
        let discovery_key = inner_channel.discovery_key.clone();

        // If the channel was already opened from the remote end, verify, and if
        // verification is ok, push a channel open event.
        if let Some(_remote_id) = inner_channel.remote_id {
            let remote_capability = inner_channel.remote_capability.clone();
            self.verify_remote_capability(remote_capability, &key)?;
            let _channel = self.create_channel(local_id);
            self.queued_events
                .push_back(Event::Open(local_id, discovery_key.clone()));
        }

        // Tell the remote end about the new channel.
        let capability = self.capability(&key);
        let message = Message::Open(Open {
            discovery_key,
            capability,
        });
        let channel_message = ChannelMessage::new(local_id as u64, message);
        self.queue_channel_message(channel_message)
    }

    fn on_open(&mut self, ch: u64, msg: Open) -> Result<()> {
        let inner_channel = self.channels.attach_remote(
            msg.discovery_key.clone(),
            ch as usize,
            msg.capability.clone(),
        );

        // This means there is not yet a locally-opened channel for this discovery_key.
        if let Some(local_id) = inner_channel.local_id {
            let key = inner_channel.key.as_ref().unwrap().clone();
            self.verify_remote_capability(msg.capability.clone(), &key)?;

            let channel = self.create_channel(local_id);
            let id = channel.id();
            self.queue_event(Event::Open(id, channel.discovery_key));
            self.queue_event(Event::Message(id, Message::Open(msg)));
        } else {
            self.queue_event(Event::DiscoveryKey(msg.discovery_key.clone()));
        }
        Ok(())
    }

    fn queue_event(&mut self, event: Event) {
        self.queued_events.push_back(event);
    }

    fn create_channel(&mut self, local_id: usize) -> Channel {
        let inner_channel = self.channels.get_local_mut(local_id).unwrap();
        let (channel, send_rx) = inner_channel.open();
        self.outbound_rx.push(Box::new(send_rx));
        channel
    }

    fn close_local(&mut self, local_id: u64) {
        if let Some(channel) = self.channels.get_local_mut(local_id as usize) {
            let discovery_key = channel.discovery_key.clone();
            self.channels.remove(&discovery_key);
            self.queue_event(Event::Close(local_id as usize));
        }
    }

    fn on_close(&mut self, remote_id: u64, msg: Close) -> Result<()> {
        if let Some(channel) = self.channels.get_remote_mut(remote_id as usize) {
            let discovery_key = channel.discovery_key.clone();
            if let Some(id) = channel.id() {
                self.queue_event(Event::Message(id, Message::Close(msg)));
                self.queue_event(Event::Close(id));
            }
            self.channels.remove(&discovery_key);
        }
        Ok(())
    }

    fn on_channel_message(&mut self, remote_id: usize, message: Message) -> Result<()> {
        if let Some(channel) = self.channels.get_remote_mut(remote_id) {
            if let Some(id) = channel.id() {
                self.queue_event(Event::Message(id, message));
            }
        }
        Ok(())
    }

    fn queue_message(&mut self, message: Vec<u8>) {
        let prefix = length_prefix(&message);
        self.queued_messages.push_back(prefix);
        self.queued_messages.push_back(message);
    }

    fn queue_channel_message(&mut self, channel_message: ChannelMessage) -> Result<()> {
        log::trace!("send {:?}", channel_message);
        let buf = channel_message.encode()?;
        self.queue_message(buf);
        Ok(())
    }

    fn queue_ping(&mut self) {
        self.queued_messages.push_back(vec![0]);
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

    /// Convert the protocol into a [Stream](futures::io::Stream) of [Event](Event)s.
    pub fn into_stream(self) -> stream::ProtocolStream<R, W> {
        let control = self.control();
        stream::ProtocolStream::new(self, control)
    }

    /// Get a sender to send control events.
    pub fn control(&self) -> ControlTx {
        self.control_tx.clone()
    }
}

/// A control event for the protocol stream.
#[derive(Debug)]
pub enum ControlEvent {
    Open(Vec<u8>),
}

/// Send [ControlEvent](ControlEvent)s to the [Protocol](Protocol).
#[derive(Clone)]
pub struct ControlTx(Sender<ControlEvent>);

impl ControlTx {
    /// Open a protocol channel.
    ///
    /// The channel will be emitted on the main protocol.
    pub async fn open(&mut self, key: Vec<u8>) -> Result<()> {
        self.0
            .send(ControlEvent::Open(key))
            .await
            .map_err(map_channel_err)
    }
}

pub use stream::ProtocolStream;
mod stream {
    use super::ControlTx;
    use crate::{Event, Protocol};
    use futures::future::FutureExt;
    use futures::io::{AsyncRead, AsyncWrite};
    use futures::stream::Stream;
    use std::future::Future;
    use std::io::Result;
    use std::pin::Pin;
    use std::task::Poll;

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
        tx: ControlTx,
    }

    impl<R, W> ProtocolStream<R, W>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        pub fn new(protocol: Protocol<R, W>, tx: ControlTx) -> Self {
            let fut = loop_next(protocol).boxed();
            Self { fut, tx }
        }

        pub async fn open(&mut self, key: Vec<u8>) -> Result<()> {
            self.tx.open(key).await
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

fn length_prefix(buf: &[u8]) -> Vec<u8> {
    let len = buf.len();
    let prefix_len = varinteger::length(len as u64);
    let mut prefix_buf = vec![0u8; prefix_len];
    varinteger::encode(len as u64, &mut prefix_buf[..prefix_len]);
    prefix_buf
}
