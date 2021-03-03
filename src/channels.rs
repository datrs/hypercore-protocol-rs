use crate::message::ChannelMessage;
use crate::schema::*;
use crate::util::{map_channel_err, pretty_hash};
use crate::Message;
use crate::{discovery_key, DiscoveryKey, Key};
use futures::task::Context;
// use futures::channel::mpsc::{Receiver, Sender};
use async_channel::{Receiver, Sender};
use futures::stream::{SelectAll, Stream, StreamExt};
use std::collections::HashMap;
use std::fmt;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::Poll;

/// The output of the set of channel senders.
type OutboundRx = SelectAll<Box<dyn Stream<Item = ChannelMessage> + Send + Unpin>>;

/// A protocol channel.
///
/// This is the handle that can be sent to other threads.
pub struct Channel {
    pub(crate) inbound_rx: Option<Receiver<Message>>,
    pub(crate) outbound_tx: Sender<Message>,
    pub(crate) discovery_key: DiscoveryKey,
    pub(crate) local_id: usize,
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
    pub fn discovery_key(&self) -> &[u8] {
        &self.discovery_key
    }

    pub fn id(&self) -> usize {
        self.local_id
    }

    /// Send a message over the channel.
    pub async fn send(&mut self, message: Message) -> Result<()> {
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
        if self.inbound_rx.is_none() {
            Poll::Ready(None)
        } else {
            Pin::new(&mut self.inbound_rx.as_mut().unwrap()).poll_next(cx)
        }
    }
}

/// The handle for a channel that lives with the main Protocol.
///
/// This lives with the main protocol.
#[derive(Clone, Debug)]
pub(crate) struct ChannelHandle {
    pub discovery_key: DiscoveryKey,
    pub key: Option<Key>,
    pub local_id: Option<usize>,
    pub remote_id: Option<usize>,
    pub inbound_tx: Option<Sender<Message>>,
    pub remote_capability: Option<Vec<u8>>,
}

impl ChannelHandle {
    fn new_local(local_id: usize, discovery_key: Vec<u8>, key: Vec<u8>) -> Self {
        Self {
            key: Some(key),
            discovery_key,
            local_id: Some(local_id),
            remote_id: None,
            inbound_tx: None,
            remote_capability: None,
        }
    }

    fn new_remote(
        remote_id: usize,
        discovery_key: Vec<u8>,
        remote_capability: Option<Vec<u8>>,
    ) -> Self {
        Self {
            discovery_key,
            remote_id: Some(remote_id),
            remote_capability,
            local_id: None,
            key: None,
            inbound_tx: None,
        }
    }

    pub(crate) fn attach_local(&mut self, local_id: usize, key: Vec<u8>) {
        self.local_id = Some(local_id);
        self.key = Some(key);
    }

    pub(crate) fn attach_remote(&mut self, remote_id: usize, remote_capability: Option<Vec<u8>>) {
        self.remote_id = Some(remote_id);
        self.remote_capability = remote_capability;
    }

    pub fn is_connected(&self) -> bool {
        self.local_id.is_some() && self.remote_id.is_some()
    }

    pub(crate) fn open(&mut self) -> (Channel, impl Stream<Item = ChannelMessage>) {
        let local_id = self
            .local_id
            .expect("May not open channel that is not locally attached");

        let (outbound_tx, outbound_rx) = async_channel::unbounded();
        let (inbound_tx, inbound_rx) = async_channel::unbounded();

        let channel = Channel {
            inbound_rx: Some(inbound_rx),
            outbound_tx,
            discovery_key: self.discovery_key.clone(),
            local_id,
        };

        let outbound_rx_mapped =
            outbound_rx.map(move |message| message.into_channel_message(local_id as u64));

        self.inbound_tx = Some(inbound_tx);
        (channel, outbound_rx_mapped)
    }

    pub(crate) fn try_send_inbound(&mut self, message: Message) -> std::io::Result<()> {
        if let Some(inbound_tx) = self.inbound_tx.as_mut() {
            inbound_tx
                .try_send(message)
                .map_err(|_e| std::io::Error::new(ErrorKind::Other, "Channel is full"))
        // inbound_tx.send(message)
        } else {
            Err(std::io::Error::new(
                ErrorKind::Other,
                "Channel is not opened",
            ))
        }
    }
}

/// The ChannelMap maintains a list of open channels and their local (tx) and remote (rx) channel IDs.
// #[derive(Debug)]
pub(crate) struct ChannelMap {
    channels: HashMap<String, ChannelHandle>,
    local_id: Vec<Option<String>>,
    remote_id: Vec<Option<String>>,
    outbound_rx: OutboundRx,
}

impl ChannelMap {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
            local_id: Vec::new(),
            remote_id: Vec::new(),
            outbound_rx: SelectAll::new(),
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

    pub fn get_remote_mut(&mut self, id: usize) -> Option<&mut ChannelHandle> {
        if let Some(Some(hdkey)) = self.remote_id.get(id).as_ref() {
            self.channels.get_mut(hdkey)
        } else {
            None
        }
    }

    pub fn get_remote(&self, id: usize) -> Option<&ChannelHandle> {
        if let Some(Some(hdkey)) = self.remote_id.get(id).as_ref() {
            self.channels.get(hdkey)
        } else {
            None
        }
    }

    pub fn get_local_mut(&mut self, id: usize) -> Option<&mut ChannelHandle> {
        if let Some(Some(hdkey)) = self.local_id.get(id).as_ref() {
            self.channels.get_mut(hdkey)
        } else {
            None
        }
    }

    pub fn get_local(&self, id: usize) -> Option<&ChannelHandle> {
        if let Some(Some(hdkey)) = self.local_id.get(id).as_ref() {
            self.channels.get(hdkey)
        } else {
            None
        }
    }

    pub fn remove(&mut self, discovery_key: &[u8]) {
        let hdkey = hex::encode(discovery_key);
        let channel = self.channels.get(&hdkey);
        if let Some(channel) = channel {
            if let Some(local_id) = channel.local_id {
                self.local_id[local_id] = None;
            }
            if let Some(remote_id) = channel.remote_id {
                self.remote_id[remote_id] = None;
            }
        }
        self.channels.remove(&hdkey);
    }

    pub fn prepare_to_verify(&self, local_id: usize) -> Result<(&Vec<u8>, Option<&Vec<u8>>)> {
        let channel_handle = self
            .get_local(local_id)
            .ok_or_else(|| Error::new(ErrorKind::Other, "Channel not found"))?;
        let remote_capability = channel_handle.remote_capability.as_ref();
        let key = channel_handle
            .key
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::Other, "Missing key"))?;
        Ok((key, remote_capability))
    }

    pub fn accept(&mut self, local_id: usize) -> Result<Channel> {
        let channel_handle = self
            .get_local_mut(local_id)
            .ok_or_else(|| Error::new(ErrorKind::Other, "Channel not found"))?;
        if !channel_handle.is_connected() {
            return Err(Error::new(
                ErrorKind::Other,
                "Channel is not opened from remote",
            ));
        }
        let (channel, send_rx) = channel_handle.open();
        self.outbound_rx.push(Box::new(send_rx));
        Ok(channel)
    }

    pub fn forward_inbound_message(&mut self, remote_id: usize, message: Message) -> Result<()> {
        if let Some(channel_handle) = self.get_remote_mut(remote_id) {
            channel_handle.try_send_inbound(message)?;
        }
        Ok(())
    }

    pub fn poll_next_outbound(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<ChannelMessage>> {
        Pin::new(&mut self.outbound_rx).poll_next(cx)
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
