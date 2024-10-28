use crate::message::ChannelMessage;
use crate::schema::*;
use crate::util::{map_channel_err, pretty_hash};
use crate::Message;
use crate::{discovery_key, DiscoveryKey, Key};
use async_channel::{Receiver, Sender, TrySendError};
use futures_lite::ready;
use futures_lite::stream::Stream;
use std::collections::HashMap;
use std::fmt;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::Poll;
use tracing::debug;

/// A protocol channel.
///
/// This is the handle that can be sent to other threads.
#[derive(Clone)]
pub struct Channel {
    inbound_rx: Option<Receiver<Message>>,
    direct_inbound_tx: Sender<Message>,
    outbound_tx: Sender<Vec<ChannelMessage>>,
    key: Key,
    discovery_key: DiscoveryKey,
    local_id: usize,
    closed: Arc<AtomicBool>,
}

impl PartialEq for Channel {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.discovery_key == other.discovery_key
            && self.local_id == other.local_id
    }
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Channel")
            .field("discovery_key", &pretty_hash(&self.discovery_key))
            .finish()
    }
}

impl Channel {
    fn new(
        inbound_rx: Option<Receiver<Message>>,
        direct_inbound_tx: Sender<Message>,
        outbound_tx: Sender<Vec<ChannelMessage>>,
        discovery_key: DiscoveryKey,
        key: Key,
        local_id: usize,
        closed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            inbound_rx,
            direct_inbound_tx,
            outbound_tx,
            key,
            discovery_key,
            local_id,
            closed,
        }
    }
    /// Get the discovery key of this channel.
    pub fn discovery_key(&self) -> &[u8; 32] {
        &self.discovery_key
    }

    /// Get the key of this channel.
    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }

    /// Get the local wire ID of this channel.
    pub fn id(&self) -> usize {
        self.local_id
    }

    /// Check if the channel is closed.
    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Send a message over the channel.
    pub async fn send(&mut self, message: Message) -> Result<()> {
        if self.closed() {
            return Err(Error::new(
                ErrorKind::ConnectionAborted,
                "Channel is closed",
            ));
        }
        debug!("TX:\n{message:?}\n");
        let message = ChannelMessage::new(self.local_id as u64, message);
        self.outbound_tx
            .send(vec![message])
            .await
            .map_err(map_channel_err)
    }

    /// Send a batch of messages over the channel.
    pub async fn send_batch(&mut self, messages: &[Message]) -> Result<()> {
        // In javascript this is cork()/uncork(), e.g.:
        //
        // https://github.com/holepunchto/hypercore/blob/c338b9aaa4442d35bc9d283d2c242b86a46de6d4/lib/replicator.js#L402-L418
        //
        // at the protomux level, where there can be messages from multiple channels in a single
        // stream write:
        //
        // https://github.com/holepunchto/protomux/blob/d3d6f8f55e52c2fbe5cd56f5d067ac43ca13c27d/index.js#L368-L389
        //
        // Batching messages across channels like protomux is capable of doing is not (yet) implemented.
        if self.closed() {
            return Err(Error::new(
                ErrorKind::ConnectionAborted,
                "Channel is closed",
            ));
        }

        let messages = messages
            .iter()
            .map(|message| {
                debug!("TX:\n{message:?}\n");
                ChannelMessage::new(self.local_id as u64, message.clone())
            })
            .collect();
        self.outbound_tx
            .send(messages)
            .await
            .map_err(map_channel_err)
    }

    /// Take the receiving part out of the channel.
    ///
    /// After taking the receiver, this Channel will not emit messages when
    /// polled as a stream. The returned receiver will.
    pub fn take_receiver(&mut self) -> Option<Receiver<Message>> {
        self.inbound_rx.take()
    }

    /// Clone the local sending part of the channel receiver. Useful
    /// for direct local communication to the channel listener. Typically
    /// you will only want to send a LocalSignal message with this sender to make
    /// it clear what event came from the remote peer and what was local
    /// signaling.
    pub fn local_sender(&self) -> Sender<Message> {
        self.direct_inbound_tx.clone()
    }

    /// Send a close message and close this channel.
    pub async fn close(&mut self) -> Result<()> {
        if self.closed() {
            return Ok(());
        }
        let close = Close {
            channel: self.local_id as u64,
        };
        self.send(Message::Close(close)).await?;
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Signal the protocol to produce Event::LocalSignal. If you want to send a message
    /// to the channel level, see take_receiver() and local_sender().
    pub async fn signal_local_protocol(&mut self, name: &str, data: Vec<u8>) -> Result<()> {
        self.send(Message::LocalSignal((name.to_string(), data)))
            .await?;
        Ok(())
    }
}

impl Stream for Channel {
    type Item = Message;
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match this.inbound_rx.as_mut() {
            None => Poll::Ready(None),
            Some(ref mut inbound_rx) => {
                let message = ready!(Pin::new(inbound_rx).poll_next(cx));
                Poll::Ready(message)
            }
        }
    }
}

/// The handle for a channel that lives with the main Protocol.
#[derive(Clone, Debug)]
pub(crate) struct ChannelHandle {
    discovery_key: DiscoveryKey,
    local_state: Option<LocalState>,
    remote_state: Option<RemoteState>,
    inbound_tx: Option<Sender<Message>>,
    closed: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
struct LocalState {
    key: Key,
    local_id: usize,
}

#[derive(Clone, Debug)]
struct RemoteState {
    remote_id: usize,
    remote_capability: Option<Vec<u8>>,
}

impl ChannelHandle {
    fn new(discovery_key: DiscoveryKey) -> Self {
        Self {
            discovery_key,
            local_state: None,
            remote_state: None,
            inbound_tx: None,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }
    fn new_local(local_id: usize, discovery_key: DiscoveryKey, key: Key) -> Self {
        let mut this = Self::new(discovery_key);
        this.attach_local(local_id, key);
        this
    }

    fn new_remote(
        remote_id: usize,
        discovery_key: DiscoveryKey,
        remote_capability: Option<Vec<u8>>,
    ) -> Self {
        let mut this = Self::new(discovery_key);
        this.attach_remote(remote_id, remote_capability);
        this
    }

    pub(crate) fn discovery_key(&self) -> &[u8; 32] {
        &self.discovery_key
    }

    pub(crate) fn local_id(&self) -> Option<usize> {
        self.local_state.as_ref().map(|s| s.local_id)
    }

    pub(crate) fn remote_id(&self) -> Option<usize> {
        self.remote_state.as_ref().map(|s| s.remote_id)
    }

    pub(crate) fn attach_local(&mut self, local_id: usize, key: Key) {
        let local_state = LocalState { local_id, key };
        self.local_state = Some(local_state);
    }

    pub(crate) fn attach_remote(&mut self, remote_id: usize, remote_capability: Option<Vec<u8>>) {
        let remote_state = RemoteState {
            remote_id,
            remote_capability,
        };
        self.remote_state = Some(remote_state);
    }

    pub(crate) fn is_connected(&self) -> bool {
        self.local_state.is_some() && self.remote_state.is_some()
    }

    pub(crate) fn prepare_to_verify(&self) -> Result<(&Key, Option<&Vec<u8>>)> {
        if !self.is_connected() {
            return Err(error("Channel is not opened from both local and remote"));
        }
        // Safe because of the is_connected() check above.
        let local_state = self.local_state.as_ref().unwrap();
        let remote_state = self.remote_state.as_ref().unwrap();
        Ok((&local_state.key, remote_state.remote_capability.as_ref()))
    }

    pub(crate) fn open(&mut self, outbound_tx: Sender<Vec<ChannelMessage>>) -> Channel {
        let local_state = self
            .local_state
            .as_ref()
            .expect("May not open channel that is not locally attached");

        let (inbound_tx, inbound_rx) = async_channel::unbounded();
        let channel = Channel::new(
            Some(inbound_rx),
            inbound_tx.clone(),
            outbound_tx,
            self.discovery_key,
            local_state.key,
            local_state.local_id,
            self.closed.clone(),
        );

        self.inbound_tx = Some(inbound_tx);
        channel
    }

    pub(crate) fn try_send_inbound(&mut self, message: Message) -> std::io::Result<()> {
        if let Some(inbound_tx) = self.inbound_tx.as_mut() {
            inbound_tx
                .try_send(message)
                .map_err(|e| error(format!("Sending to channel failed: {e}").as_str()))
        } else {
            Err(error("Channel is not open"))
        }
    }

    pub(crate) fn try_send_inbound_tolerate_closed(
        &mut self,
        message: Message,
    ) -> std::io::Result<()> {
        if let Some(inbound_tx) = self.inbound_tx.as_mut() {
            if let Err(err) = inbound_tx.try_send(message) {
                match err {
                    TrySendError::Full(e) => {
                        return Err(error(format!("Sending to channel failed: {e}").as_str()))
                    }
                    TrySendError::Closed(_) => {}
                }
            }
        }
        Ok(())
    }
}

impl Drop for ChannelHandle {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

/// The ChannelMap maintains a list of open channels and their local (tx) and remote (rx) channel IDs.
#[derive(Debug)]
pub(crate) struct ChannelMap {
    channels: HashMap<String, ChannelHandle>,
    local_id: Vec<Option<String>>,
    remote_id: Vec<Option<String>>,
}

impl ChannelMap {
    pub(crate) fn new() -> Self {
        Self {
            channels: HashMap::new(),
            // Add a first None value to local_id to start ids at 1.
            // This makes sure that 0 may be used for stream-level extensions.
            local_id: vec![None],
            remote_id: vec![],
        }
    }

    pub(crate) fn attach_local(&mut self, key: Key) -> &ChannelHandle {
        let discovery_key = discovery_key(&key);
        let hdkey = hex::encode(discovery_key);
        let local_id = self.alloc_local();

        self.channels
            .entry(hdkey.clone())
            .and_modify(|channel| channel.attach_local(local_id, key))
            .or_insert_with(|| ChannelHandle::new_local(local_id, discovery_key, key));

        self.local_id[local_id] = Some(hdkey.clone());
        self.channels.get(&hdkey).unwrap()
    }

    pub(crate) fn attach_remote(
        &mut self,
        discovery_key: DiscoveryKey,
        remote_id: usize,
        remote_capability: Option<Vec<u8>>,
    ) -> &ChannelHandle {
        let hdkey = hex::encode(discovery_key);
        self.alloc_remote(remote_id);
        self.channels
            .entry(hdkey.clone())
            .and_modify(|channel| channel.attach_remote(remote_id, remote_capability.clone()))
            .or_insert_with(|| {
                ChannelHandle::new_remote(remote_id, discovery_key, remote_capability)
            });
        self.remote_id[remote_id] = Some(hdkey.clone());
        self.channels.get(&hdkey).unwrap()
    }

    pub(crate) fn get_remote_mut(&mut self, remote_id: usize) -> Option<&mut ChannelHandle> {
        if let Some(Some(hdkey)) = self.remote_id.get(remote_id).as_ref() {
            self.channels.get_mut(hdkey)
        } else {
            None
        }
    }

    pub(crate) fn get_remote(&self, remote_id: usize) -> Option<&ChannelHandle> {
        if let Some(Some(hdkey)) = self.remote_id.get(remote_id).as_ref() {
            self.channels.get(hdkey)
        } else {
            None
        }
    }

    pub(crate) fn get_local_mut(&mut self, local_id: usize) -> Option<&mut ChannelHandle> {
        if let Some(Some(hdkey)) = self.local_id.get(local_id).as_ref() {
            self.channels.get_mut(hdkey)
        } else {
            None
        }
    }

    pub(crate) fn get_local(&self, local_id: usize) -> Option<&ChannelHandle> {
        if let Some(Some(hdkey)) = self.local_id.get(local_id).as_ref() {
            self.channels.get(hdkey)
        } else {
            None
        }
    }

    pub(crate) fn has_channel(&self, discovery_key: &[u8]) -> bool {
        let hdkey = hex::encode(discovery_key);
        self.channels.contains_key(&hdkey)
    }

    pub(crate) fn remove(&mut self, discovery_key: &[u8]) {
        let hdkey = hex::encode(discovery_key);
        let channel = self.channels.get(&hdkey);
        if let Some(channel) = channel {
            if let Some(local_id) = channel.local_id() {
                self.local_id[local_id] = None;
            }
            if let Some(remote_id) = channel.remote_id() {
                self.remote_id[remote_id] = None;
            }
        }
        self.channels.remove(&hdkey);
    }

    pub(crate) fn prepare_to_verify(&self, local_id: usize) -> Result<(&Key, Option<&Vec<u8>>)> {
        let channel_handle = self
            .get_local(local_id)
            .ok_or_else(|| error("Channel not found"))?;
        channel_handle.prepare_to_verify()
    }

    pub(crate) fn accept(
        &mut self,
        local_id: usize,
        outbound_tx: Sender<Vec<ChannelMessage>>,
    ) -> Result<Channel> {
        let channel_handle = self
            .get_local_mut(local_id)
            .ok_or_else(|| error("Channel not found"))?;
        if !channel_handle.is_connected() {
            return Err(error("Channel is not opened from remote"));
        }
        let channel = channel_handle.open(outbound_tx);
        Ok(channel)
    }

    pub(crate) fn forward_inbound_message(
        &mut self,
        remote_id: usize,
        message: Message,
    ) -> Result<()> {
        if let Some(channel_handle) = self.get_remote_mut(remote_id) {
            channel_handle.try_send_inbound(message)?;
        }
        Ok(())
    }

    pub(crate) fn forward_inbound_message_tolerate_closed(
        &mut self,
        remote_id: usize,
        message: Message,
    ) -> Result<()> {
        if let Some(channel_handle) = self.get_remote_mut(remote_id) {
            channel_handle.try_send_inbound_tolerate_closed(message)?;
        }
        Ok(())
    }

    fn alloc_local(&mut self) -> usize {
        let empty_id = self
            .local_id
            .iter()
            .skip(1)
            .position(|x| x.is_none())
            .map(|position| position + 1);
        match empty_id {
            Some(empty_id) => empty_id,
            None => {
                self.local_id.push(None);
                self.local_id.len() - 1
            }
        }
    }

    fn alloc_remote(&mut self, id: usize) {
        if self.remote_id.len() > id {
            self.remote_id[id] = None;
        } else {
            self.remote_id.resize(id + 1, None)
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &ChannelHandle> {
        self.channels.values()
    }
}

fn error(message: &str) -> Error {
    Error::new(ErrorKind::Other, message)
}
