use crate::discovery_key;
use crate::schema::Open;
use crate::schema::*;
use crate::Message;
use futures::channel::mpsc::Sender;
use futures::sink::SinkExt;
use hex;
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Result};

#[derive(Clone, Debug)]
pub struct ChannelInfo {
    // pub(crate) handlers: ChannelHandlerType,
    pub(crate) discovery_key: Vec<u8>,
    pub(crate) key: Option<Vec<u8>>,
    pub(crate) local_id: Option<usize>,
    pub(crate) remote_id: Option<usize>,
    pub(crate) recv_tx: Option<Sender<Message>>,
    pub(crate) remote_capability: Option<Vec<u8>>,
}

impl ChannelInfo {
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
pub struct Channelizer {
    channels: HashMap<String, ChannelInfo>,
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

    pub fn get_mut(&mut self, discovery_key: &[u8]) -> Option<&mut ChannelInfo> {
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

    pub fn get_remote_mut(&mut self, id: usize) -> Option<&mut ChannelInfo> {
        if let Some(Some(hdkey)) = self.remote_id.get(id).as_ref() {
            self.channels.get_mut(hdkey)
        } else {
            None
        }
    }

    pub fn get_local_mut(&mut self, id: usize) -> Option<&mut ChannelInfo> {
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

    pub fn attach_local(&mut self, key: Vec<u8>) -> &ChannelInfo {
        let discovery_key = discovery_key(&key);
        let hdkey = hex::encode(&discovery_key);
        let local_id = self.alloc_local();

        self.channels
            .entry(hdkey.clone())
            .and_modify(|channel| {
                channel.local_id = Some(local_id);
                channel.key = Some(key.clone());
            })
            .or_insert_with(|| ChannelInfo {
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
    ) -> &ChannelInfo {
        let hdkey = hex::encode(&discovery_key);
        self.alloc_remote(remote_id);

        self.channels
            .entry(hdkey.clone())
            .and_modify(|channel| {
                channel.remote_id = Some(remote_id);
                channel.remote_capability = remote_capability.clone();
            })
            .or_insert_with(|| ChannelInfo {
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
