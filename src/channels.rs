use crate::message::ChannelMessage;
use crate::schema::*;
use crate::util::{map_channel_err, pretty_hash};
use crate::Message;
use crate::{discovery_key, DiscoveryKey, Key};
use async_channel::{Receiver, Sender};
use futures_lite::stream::Stream;
use std::collections::HashMap;
use std::fmt;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::Poll;

/// A protocol channel.
///
/// This is the handle that can be sent to other threads.
pub struct Channel {
    inbound_rx: Option<Receiver<Message>>,
    outbound_tx: Sender<ChannelMessage>,
    key: Key,
    discovery_key: DiscoveryKey,
    local_id: usize,
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Channel")
            .field("discovery_key", &pretty_hash(&self.discovery_key))
            .finish()
    }
}

impl Channel {
    /// Get the discovery key of this channel.
    pub fn discovery_key(&self) -> &[u8; 32] {
        &self.discovery_key
    }

    /// Get the key of this channel.
    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }

    pub fn id(&self) -> usize {
        self.local_id
    }

    /// Send a message over the channel.
    pub async fn send(&mut self, message: Message) -> Result<()> {
        let message = ChannelMessage::new(self.local_id as u64, message);
        self.outbound_tx
            .send(message)
            .await
            .map_err(map_channel_err)
    }

    pub fn take_receiver(&mut self) -> Option<Receiver<Message>> {
        self.inbound_rx.take()
    }

    /// Send a status message.
    pub async fn status(&mut self, msg: Status) -> Result<()> {
        self.send(Message::Status(msg)).await
    }

    /// Send a options message.
    pub async fn options(&mut self, msg: Options) -> Result<()> {
        self.send(Message::Options(msg)).await
    }

    /// Send a have message.
    pub async fn have(&mut self, msg: Have) -> Result<()> {
        self.send(Message::Have(msg)).await
    }

    /// Send a unhave message.
    pub async fn unhave(&mut self, msg: Unhave) -> Result<()> {
        self.send(Message::Unhave(msg)).await
    }

    /// Send a want message.
    pub async fn want(&mut self, msg: Want) -> Result<()> {
        self.send(Message::Want(msg)).await
    }

    /// Send a unwant message.
    pub async fn unwant(&mut self, msg: Unwant) -> Result<()> {
        self.send(Message::Unwant(msg)).await
    }

    /// Send a request message.
    pub async fn request(&mut self, msg: Request) -> Result<()> {
        self.send(Message::Request(msg)).await
    }

    /// Send a cancel message.
    pub async fn cancel(&mut self, msg: Cancel) -> Result<()> {
        self.send(Message::Cancel(msg)).await
    }

    /// Send a data message.
    pub async fn data(&mut self, msg: Data) -> Result<()> {
        self.send(Message::Data(msg)).await
    }

    /// Send a close message and close this channel.
    pub async fn close(&mut self, msg: Close) -> Result<()> {
        self.send(Message::Close(msg)).await
    }
}

impl Stream for Channel {
    type Item = Message;
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.inbound_rx.as_mut() {
            None => Poll::Ready(None),
            Some(inbound_rx) => Pin::new(inbound_rx).poll_next(cx),
        }
    }
}

/// The handle for a channel that lives with the main Protocol.
///
/// This lives with the main protocol.
#[derive(Clone, Debug)]
pub(crate) struct ChannelHandle {
    discovery_key: DiscoveryKey,
    local_state: Option<LocalState>,
    remote_state: Option<RemoteState>,
    inbound_tx: Option<Sender<Message>>,
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

    pub fn discovery_key(&self) -> &[u8; 32] {
        &self.discovery_key
    }

    pub fn local_id(&self) -> Option<usize> {
        self.local_state.as_ref().map(|s| s.local_id)
    }

    pub fn remote_id(&self) -> Option<usize> {
        self.remote_state.as_ref().map(|s| s.remote_id)
    }

    pub fn attach_local(&mut self, local_id: usize, key: Key) {
        let local_state = LocalState { local_id, key };
        self.local_state = Some(local_state);
    }

    pub fn attach_remote(&mut self, remote_id: usize, remote_capability: Option<Vec<u8>>) {
        let remote_state = RemoteState {
            remote_id,
            remote_capability,
        };
        self.remote_state = Some(remote_state);
    }

    pub fn is_connected(&self) -> bool {
        self.local_state.is_some() && self.remote_state.is_some()
    }

    pub fn prepare_to_verify(&self) -> Result<(&Key, Option<&Vec<u8>>)> {
        if !self.is_connected() {
            return Err(error("Channel is not opened from both local and remote"));
        }
        // Safe because of the is_connected() check above.
        let local_state = self.local_state.as_ref().unwrap();
        let remote_state = self.remote_state.as_ref().unwrap();
        Ok((&local_state.key, remote_state.remote_capability.as_ref()))
    }

    pub fn open(&mut self, outbound_tx: Sender<ChannelMessage>) -> Channel {
        let local_state = self
            .local_state
            .as_ref()
            .expect("May not open channel that is not locally attached");

        let (inbound_tx, inbound_rx) = async_channel::unbounded();
        let channel = Channel {
            inbound_rx: Some(inbound_rx),
            outbound_tx,
            discovery_key: self.discovery_key.clone(),
            key: local_state.key.clone(),
            local_id: local_state.local_id,
        };
        self.inbound_tx = Some(inbound_tx);
        channel
    }

    pub fn try_send_inbound(&mut self, message: Message) -> std::io::Result<()> {
        if let Some(inbound_tx) = self.inbound_tx.as_mut() {
            inbound_tx
                .try_send(message)
                .map_err(|_e| error("Channel is full"))
        } else {
            Err(error("Channel is not open"))
        }
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
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
            local_id: Vec::new(),
            remote_id: Vec::new(),
        }
    }

    pub fn attach_local(&mut self, key: Key) -> &ChannelHandle {
        let discovery_key = discovery_key(&key);
        let hdkey = hex::encode(&discovery_key);
        let local_id = self.alloc_local();

        self.channels
            .entry(hdkey.clone())
            .and_modify(|channel| channel.attach_local(local_id, key.clone()))
            .or_insert_with(|| {
                ChannelHandle::new_local(local_id, discovery_key.clone(), key.clone())
            });

        self.local_id[local_id] = Some(hdkey.clone());
        self.channels.get(&hdkey).unwrap()
    }

    pub fn attach_remote(
        &mut self,
        discovery_key: DiscoveryKey,
        remote_id: usize,
        remote_capability: Option<Vec<u8>>,
    ) -> &ChannelHandle {
        let hdkey = hex::encode(&discovery_key);
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

    pub fn get_remote_mut(&mut self, remote_id: usize) -> Option<&mut ChannelHandle> {
        if let Some(Some(hdkey)) = self.remote_id.get(remote_id).as_ref() {
            self.channels.get_mut(hdkey)
        } else {
            None
        }
    }

    pub fn get_remote(&self, remote_id: usize) -> Option<&ChannelHandle> {
        if let Some(Some(hdkey)) = self.remote_id.get(remote_id).as_ref() {
            self.channels.get(hdkey)
        } else {
            None
        }
    }

    pub fn get_local_mut(&mut self, local_id: usize) -> Option<&mut ChannelHandle> {
        if let Some(Some(hdkey)) = self.local_id.get(local_id).as_ref() {
            self.channels.get_mut(hdkey)
        } else {
            None
        }
    }

    pub fn get_local(&self, local_id: usize) -> Option<&ChannelHandle> {
        if let Some(Some(hdkey)) = self.local_id.get(local_id).as_ref() {
            self.channels.get(hdkey)
        } else {
            None
        }
    }

    pub fn remove(&mut self, discovery_key: &[u8]) {
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

    pub fn prepare_to_verify(&self, local_id: usize) -> Result<(&Key, Option<&Vec<u8>>)> {
        let channel_handle = self
            .get_local(local_id)
            .ok_or_else(|| error("Channel not found"))?;
        channel_handle.prepare_to_verify()
    }

    pub fn accept(
        &mut self,
        local_id: usize,
        outbound_tx: Sender<ChannelMessage>,
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

    pub fn forward_inbound_message(&mut self, remote_id: usize, message: Message) -> Result<()> {
        if let Some(channel_handle) = self.get_remote_mut(remote_id) {
            channel_handle.try_send_inbound(message)?;
        }
        Ok(())
    }

    fn alloc_local(&mut self) -> usize {
        let empty_id = self.local_id.iter().position(|x| x.is_none());
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
}

fn error(message: &str) -> Error {
    Error::new(ErrorKind::Other, message)
}
