use crate::discovery_key;
use crate::schema::Open;
use crate::schema::*;
use crate::util::{map_channel_err, pretty_hash};
use crate::Message;
use futures::channel::mpsc::{Receiver, Sender};
use futures::sink::SinkExt;
use futures::stream::{SelectAll, Stream, StreamExt};
use hex;
use std::collections::HashMap;
use std::fmt;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;

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
    pub fn sender(&self) -> Sender<Message> {
        self.send_tx.clone()
    }
    pub fn discovery_key(&self) -> &[u8] {
        &self.discovery_key
    }
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

/// The part of a channel.
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
    pub(crate) async fn recv(&mut self, message: Message) -> Result<()> {
        if let Some(recv_tx) = self.recv_tx.as_mut() {
            match recv_tx.send(message).await {
                Ok(_) => Ok(()),
                Err(err) => match err.is_disconnected() {
                    // TODO: Close channel?
                    true => Ok(()),
                    false => Err(Error::new(
                        ErrorKind::BrokenPipe,
                        "Cannot forward on channel: Channel is full",
                    )),
                },
            }
        } else {
            Err(Error::new(
                ErrorKind::BrokenPipe,
                "Cannot forward: Channel missing",
            ))
        }
    }

    pub(crate) fn local_open(&self) -> bool {
        self.local_id.is_some()
    }

    pub(crate) fn remote_open(&self) -> bool {
        self.remote_id.is_some()
    }

    pub(crate) async fn recv_open(
        &mut self,
        recv_tx: Sender<Message>,
        message: Open,
    ) -> Result<()> {
        self.recv_tx = Some(recv_tx);
        self.recv(Message::Open(message)).await
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

    pub fn get_mut(&mut self, discovery_key: &[u8]) -> Option<&mut InnerChannel> {
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

    // pub fn get_remote(&self, id: usize) -> Option<&ChannelInfo> {
    //     if let Some(Some(hdkey)) = self.remote_id.get(id).as_ref() {
    //         self.channels.get(hdkey)
    //     } else {
    //         None
    //     }
    // }

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

    pub async fn open(&mut self, discovery_key: &[u8], recv_tx: Sender<Message>) -> Result<()> {
        if let Some(channel) = self.get_mut(discovery_key) {
            let message = Open {
                discovery_key: discovery_key.to_vec(),
                capability: None,
            };
            channel.recv_open(recv_tx, message).await
        } else {
            Ok(())
        }
    }

    pub fn attach_local(&mut self, key: Vec<u8>) -> &InnerChannel {
        let discovery_key = discovery_key(&key);
        let hdkey = hex::encode(&discovery_key);
        let local_id = self.alloc_local();

        self.channels
            .entry(hdkey.clone())
            .and_modify(|channel| {
                channel.local_id = Some(local_id);
                channel.key = Some(key.clone());
            })
            .or_insert_with(|| InnerChannel {
                key: Some(key),
                discovery_key: discovery_key.clone(),
                local_id: Some(local_id),
                remote_id: None,
                recv_tx: None,
                remote_capability: None,
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
            .and_modify(|channel| {
                channel.remote_id = Some(remote_id);
                channel.remote_capability = remote_capability.clone();
            })
            .or_insert_with(|| InnerChannel {
                discovery_key: discovery_key.clone(),
                remote_id: Some(remote_id),
                remote_capability,
                local_id: None,
                key: None,
                recv_tx: None,
            });
        self.remote_id[remote_id] = Some(hdkey.clone());
        self.channels.get(&hdkey).unwrap()
    }
}
