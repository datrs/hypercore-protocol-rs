use crate::{discovery_key, handlers::ChannelHandlerType};
use hex;
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Result};

#[derive(Clone)]
pub struct ChannelInfo {
    pub(crate) handlers: ChannelHandlerType,
    pub(crate) discovery_key: Vec<u8>,
    pub(crate) key: Option<Vec<u8>>,
    pub(crate) local_id: Option<usize>,
    pub(crate) remote_id: Option<usize>,
}

/// The Channelizer maintains a list of open channels and their local (tx) and remote (rx) channel IDs.
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

    pub fn get(&self, discovery_key: &[u8]) -> Option<&ChannelInfo> {
        let hdkey = hex::encode(discovery_key);
        self.channels.get(&hdkey)
    }

    pub fn get_key(&self, discovery_key: &[u8]) -> Option<Vec<u8>> {
        match self.get(&discovery_key) {
            None => None,
            Some(channel) => channel.key.as_ref().map(|k| k.to_vec()),
        }
    }

    pub fn resolve_remote(&self, id: usize) -> Result<Vec<u8>> {
        match self.get_remote(id) {
            Some(channel) => Ok(channel.discovery_key.clone()),
            None => Err(Error::new(
                ErrorKind::BrokenPipe,
                "Remote channel is not open",
            )),
        }
    }

    pub fn get_remote(&self, id: usize) -> Option<&ChannelInfo> {
        match self.remote_id.get(id) {
            Some(Some(hdkey)) => self.channels.get(hdkey),
            _ => None,
        }
    }

    pub fn get_local_id(&self, discovery_key: &[u8]) -> Option<usize> {
        match self.get(&discovery_key) {
            Some(channel) => channel.local_id,
            None => None,
        }
    }

    pub fn _get_local(&self, id: usize) -> Option<&ChannelInfo> {
        match self.local_id.get(id) {
            None => None,
            Some(None) => None,
            Some(Some(hdkey)) => self.channels.get(hdkey),
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

    pub fn attach_local(&mut self, key: Vec<u8>, handlers: ChannelHandlerType) -> Vec<u8> {
        let discovery_key = discovery_key(&key);
        let hdkey = hex::encode(&discovery_key);

        let local_id = self.alloc_local();
        self.local_id[local_id] = Some(hdkey.clone());

        if self.channels.contains_key(&hdkey) {
            let channel = self.channels.get_mut(&hdkey).unwrap();
            channel.local_id = Some(local_id);
            channel.key = Some(key);
        } else {
            let channel = ChannelInfo {
                key: Some(key),
                discovery_key: discovery_key.clone(),
                local_id: Some(local_id),
                remote_id: None,
                handlers,
            };
            self.channels.insert(hdkey, channel);
        }

        discovery_key
    }

    pub fn attach_remote(&mut self, discovery_key: Vec<u8>, remote_id: usize) -> Result<()> {
        let hdkey = hex::encode(&discovery_key);
        match self.channels.get_mut(&hdkey) {
            Some(mut channel) => {
                channel.remote_id = Some(remote_id);
                self.alloc_remote(remote_id);
                self.remote_id[remote_id] = Some(hdkey.clone());
                Ok(())
            }
            None => Err(Error::new(
                ErrorKind::BrokenPipe,
                "Cannot attach channel if not opened locally before",
            )),
        }
    }
}
