use crate::discovery_key;
use crate::message::ChannelMessage;
use crate::schema::Open;
use crate::schema::*;
use crate::util::{map_channel_err, pretty_hash};
use crate::Message;
use futures::channel::mpsc::{Receiver, Sender};
use futures::sink::SinkExt;
use futures::stream::{Stream, StreamExt};
use hex;
use std::collections::HashMap;
use std::fmt;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;

const CHANNEL_CAP: usize = 1000;

/// A protocol channel.
///
/// This is the outer channel handler that can be sent to other threads.
pub struct Channel {
    pub(crate) recv_rx: Receiver<Message>,
    pub(crate) send_tx: Sender<Message>,
    pub(crate) discovery_key: Vec<u8>,
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

    /// Send a message over the channel.
    pub async fn send(&mut self, message: Message) -> Result<()> {
        self.send_tx.send(message).await.map_err(map_channel_err)
    }
}

impl Stream for Channel {
    type Item = Message;
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.recv_rx).poll_next(cx)
    }
}

/// The inner part of a channel.
///
/// This lives with the main protocol.
#[derive(Clone, Debug)]
pub(crate) struct InnerChannel {
    pub discovery_key: Vec<u8>,
    pub key: Option<Vec<u8>>,
    pub local_id: Option<usize>,
    pub remote_id: Option<usize>,
    pub recv_tx: Option<Sender<Message>>,
    pub remote_capability: Option<Vec<u8>>,
}

impl InnerChannel {
    pub(crate) fn new_by_local(local_id: usize, discovery_key: Vec<u8>, key: Vec<u8>) -> Self {
        Self {
            key: Some(key),
            discovery_key,
            local_id: Some(local_id),
            remote_id: None,
            recv_tx: None,
            remote_capability: None,
        }
    }

    pub(crate) fn new_by_remote(
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
            recv_tx: None,
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

    pub(crate) async fn recv(&mut self, message: Message) -> Result<()> {
        if let Some(recv_tx) = self.recv_tx.as_mut() {
            match recv_tx.send(message).await {
                Ok(_) => Ok(()),
                Err(err) => {
                    if err.is_disconnected() {
                        Ok(())
                    } else {
                        Err(Error::new(
                            ErrorKind::BrokenPipe,
                            "Cannot forward on channel: Channel is full",
                        ))
                    }
                }
            }
        } else {
            Err(Error::new(
                ErrorKind::BrokenPipe,
                "Cannot forward: Channel missing",
            ))
        }
    }

    pub(crate) async fn open(&mut self) -> Result<(Channel, impl Stream<Item = ChannelMessage>)> {
        let local_id = self
            .local_id
            .expect("May not open channel that is not locally attached");

        let (send_tx, send_rx) = futures::channel::mpsc::channel(CHANNEL_CAP);
        let (recv_tx, recv_rx) = futures::channel::mpsc::channel(CHANNEL_CAP);

        let outer_channel = Channel {
            recv_rx,
            send_tx,
            discovery_key: self.discovery_key.clone(),
        };

        let send_rx_mapped =
            send_rx.map(move |message| message.into_channel_message(local_id as u64));

        let message = Open {
            discovery_key: self.discovery_key.clone(),
            capability: None,
        };
        self.recv_tx = Some(recv_tx);
        self.recv(Message::Open(message)).await?;
        Ok((outer_channel, send_rx_mapped))
    }

    pub(crate) async fn recv_close(&mut self, message: Option<Close>) -> Result<()> {
        let message = message.unwrap_or_else(|| Close {
            discovery_key: None,
        });
        self.recv(Message::Close(message)).await
    }
}

/// The Channelizer maintains a list of open channels and their local (tx) and remote (rx) channel IDs.
#[derive(Debug)]
pub(crate) struct Channelizer {
    channels: HashMap<String, InnerChannel>,
    local_id: Vec<Option<String>>,
    remote_id: Vec<Option<String>>,
}

impl Channelizer {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
            local_id: Vec::new(),
            remote_id: Vec::new(),
        }
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

    pub fn _get_mut(&mut self, discovery_key: &[u8]) -> Option<&mut InnerChannel> {
        let hdkey = hex::encode(discovery_key);
        self.channels.get_mut(&hdkey)
    }

    pub async fn forward(&mut self, remote_id: usize, message: Message) -> Result<()> {
        self.get_remote_mut(remote_id)
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::BrokenPipe,
                    format!("Cannot forward: Channel {} is not open", remote_id),
                )
            })?
            .recv(message)
            .await
    }

    pub fn _get_remote(&self, id: usize) -> Option<&InnerChannel> {
        if let Some(Some(hdkey)) = self.remote_id.get(id).as_ref() {
            self.channels.get(hdkey)
        } else {
            None
        }
    }

    pub fn get_remote_mut(&mut self, id: usize) -> Option<&mut InnerChannel> {
        if let Some(Some(hdkey)) = self.remote_id.get(id).as_ref() {
            self.channels.get_mut(hdkey)
        } else {
            None
        }
    }

    pub fn get_local_mut(&mut self, id: usize) -> Option<&mut InnerChannel> {
        if let Some(Some(hdkey)) = self.local_id.get(id).as_ref() {
            self.channels.get_mut(hdkey)
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

    pub fn attach_local(&mut self, key: Vec<u8>) -> &InnerChannel {
        let discovery_key = discovery_key(&key);
        let hdkey = hex::encode(&discovery_key);
        let local_id = self.alloc_local();

        self.channels
            .entry(hdkey.clone())
            .and_modify(|channel| channel.attach_local(local_id, key.clone()))
            .or_insert_with(|| {
                InnerChannel::new_by_local(local_id, discovery_key.clone(), key.clone())
            });

        self.local_id[local_id] = Some(hdkey.clone());
        self.channels.get(&hdkey).unwrap()
    }

    pub fn attach_remote(
        &mut self,
        discovery_key: Vec<u8>,
        remote_id: usize,
        remote_capability: Option<Vec<u8>>,
    ) -> &InnerChannel {
        let hdkey = hex::encode(&discovery_key);
        self.alloc_remote(remote_id);
        self.channels
            .entry(hdkey.clone())
            .and_modify(|channel| channel.attach_remote(remote_id, remote_capability.clone()))
            .or_insert_with(|| {
                InnerChannel::new_by_remote(remote_id, discovery_key, remote_capability)
            });
        self.remote_id[remote_id] = Some(hdkey.clone());
        self.channels.get(&hdkey).unwrap()
    }
}
