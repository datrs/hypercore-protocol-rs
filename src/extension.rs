use crate::message::{ChannelMessage, ExtensionMessage, Message};
use crate::schema::*;
use async_channel::{Receiver, Sender};
use futures_lite::stream::Stream;
use std::collections::HashMap;
use std::pin::Pin;

#[derive(Debug)]
pub struct Extensions {
    extensions: HashMap<String, ExtensionHandle>,
    channel: u64,
    local_ids: Vec<String>,
    remote_ids: Vec<String>,
    outbound_tx: Sender<ChannelMessage>,
}

impl Extensions {
    pub fn new(outbound_tx: Sender<ChannelMessage>, channel: u64) -> Self {
        Self {
            channel,
            extensions: HashMap::new(),
            local_ids: vec![],
            remote_ids: vec![],
            outbound_tx,
        }
    }

    pub fn add_local_name(&mut self, name: String) -> u64 {
        self.local_ids.push(name.clone());
        self.local_ids.sort();
        let local_id = self.local_ids.iter().position(|x| x == &name).unwrap();
        local_id as u64
    }

    pub fn register(&mut self, name: String) -> Extension {
        let local_id = self.add_local_name(name.clone());
        let (inbound_tx, inbound_rx) = async_channel::unbounded();
        let handle = ExtensionHandle {
            name: name.clone(),
            channel: self.channel,
            local_id,
            inbound_tx,
        };
        let extension = Extension {
            name: name.clone(),
            channel: self.channel,
            local_id,
            outbound_tx: self.outbound_tx.clone(),
            inbound_rx,
        };
        self.extensions.insert(name, handle);

        let message = Options {
            extensions: self.local_ids.clone(),
            ack: None,
        };
        let message = ChannelMessage::new(self.channel, Message::Options(message));
        self.outbound_tx.try_send(message).unwrap();

        extension
    }

    pub fn on_remote_update(&mut self, names: Vec<String>) {
        self.remote_ids = names;
    }

    pub fn on_message(&mut self, message: ExtensionMessage) {
        let ExtensionMessage { id, message } = message;
        if let Some(name) = self.remote_ids.get(id as usize) {
            if let Some(handle) = self.extensions.get_mut(name) {
                handle.send(message);
            }
        }
    }
}

#[derive(Debug)]
pub struct ExtensionHandle {
    name: String,
    channel: u64,
    local_id: u64,
    inbound_tx: Sender<Vec<u8>>,
}

impl ExtensionHandle {
    fn send(&mut self, message: Vec<u8>) {
        self.inbound_tx.try_send(message).unwrap()
    }
}

#[derive(Debug)]
pub struct Extension {
    name: String,
    channel: u64,
    local_id: u64,
    outbound_tx: Sender<ChannelMessage>,
    inbound_rx: Receiver<Vec<u8>>,
}

impl Stream for Extension {
    type Item = Vec<u8>;
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.inbound_rx).poll_next(cx)
    }
}

impl Extension {
    pub async fn send(&mut self, message: Vec<u8>) {
        let message = ExtensionMessage::new(self.local_id, message);
        let message = ChannelMessage::new(self.channel, Message::Extension(message));
        self.outbound_tx.send(message).await.unwrap()
    }
}
