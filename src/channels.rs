use hex;
use std::collections::HashMap;

use crate::{discovery_key, handlers::ChannelHandlerType};

#[derive(Clone)]
pub struct Channel {
    pub(crate) handlers: ChannelHandlerType,
    pub(crate) discovery_key: Vec<u8>,
    pub(crate) key: Option<Vec<u8>>,
    pub(crate) local_id: Option<usize>,
    pub(crate) remote_id: Option<usize>,
}

impl Channel {
    // pub fn with_handlers(key: Vec<u8>, handlers: ChannelHandlerType) -> Self {
    //     let discovery_key = discovery_key(&key);
    //     Self {
    //         discovery_key: discovery_key,
    //         handlers: handlers,
    //         key: Some(key),
    //         local_id: None,
    //         remote_id: None,
    //     }
    // }
    // pub fn is_remote_open(&self) -> bool {
    //     self.remote_id.is_some()
    // }

    // pub fn is_local_open(&self) -> bool {
    //     self.local_id.is_some()
    // }

    // pub fn set_key(&mut self, key: Vec<u8>) {
    //     self.key = Some(key)
    // }
}

/// The Channelizer maintains a list of open channels and their local (tx) and remote (rx) channel IDs.
pub struct Channelizer {
    channels: HashMap<String, Channel>,
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
            while self.remote_id.len() < id + 1 {
                self.remote_id.push(None);
            }
        }
    }

    pub fn get(&self, discovery_key: &[u8]) -> Option<&Channel> {
        let hdkey = hex::encode(discovery_key);
        self.channels.get(&hdkey)
    }

    pub fn get_remote(&self, id: usize) -> Option<&Channel> {
        match self.remote_id.get(id) {
            None => None,
            Some(None) => None,
            Some(Some(hdkey)) => self.channels.get(hdkey),
        }
    }

    pub fn get_local_id(&self, discovery_key: &[u8]) -> Option<usize> {
        match self.get(&discovery_key) {
            None | Some(Channel { local_id: None, .. }) => None,
            Some(Channel {
                local_id: Some(local_id),
                ..
            }) => Some(local_id.clone()),
        }
    }

    pub fn _get_local(&self, id: usize) -> Option<&Channel> {
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
            let channel = Channel {
                key: Some(key),
                discovery_key: discovery_key.clone(),
                local_id: Some(local_id),
                remote_id: None,
                handlers,
            };
            self.channels.insert(hdkey.clone(), channel);
        }

        discovery_key
    }

    pub fn attach_remote(&mut self, discovery_key: Vec<u8>, remote_id: usize) {
        let hdkey = hex::encode(&discovery_key);

        self.alloc_remote(remote_id);
        self.remote_id[remote_id] = Some(hdkey.clone());

        if self.channels.contains_key(&hdkey) {
            let channel = self.channels.get_mut(&hdkey).unwrap();
            // TODO: If a remote opens a channel multiple times it could happen
            // that we keep a growing list of old remote_id mappings. Remove old?
            channel.remote_id = Some(remote_id);
        } else {
            // TODO: Throw an error? This may not happen, as attach_remote()
            // is never called on channels not opened by local also.
            // let channel = Channel {
            //     key: None,
            //     discovery_key: discovery_key,
            //     local_id: None,
            //     remote_id: Some(remote_id),
            //     handlers: None,
            // };
            // self.channels.insert(hdkey.clone(), channel);
        }
    }
}
