use crate::constants::MAX_MESSAGE_SIZE;
use crate::message::{ChannelMessage, ExtensionMessage, Message};
use crate::schema::*;
use async_channel::{Receiver, Sender};
use futures_lite::{ready, AsyncRead, AsyncWrite, FutureExt, Stream};
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

const MAX_BODY_SIZE: usize = MAX_MESSAGE_SIZE as usize - 16;

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

    pub async fn register(&mut self, name: String) -> Extension {
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
            write_state: WriteState::Idle,
            read_state: None,
        };
        self.extensions.insert(name, handle);

        let message = Options {
            extensions: self.local_ids.clone(),
            ack: None,
        };
        let message = ChannelMessage::new(self.channel, Message::Options(message));
        self.outbound_tx.send(message).await.unwrap();

        extension
    }

    pub fn on_remote_update(&mut self, names: Vec<String>) {
        self.remote_ids = names;
    }

    pub fn on_message(&mut self, message: ExtensionMessage) {
        let ExtensionMessage { id, message } = message;
        if let Some(name) = self.remote_ids.get(id as usize) {
            if let Some(handle) = self.extensions.get_mut(name) {
                handle.inbound_send(message);
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
    fn inbound_send(&mut self, message: Vec<u8>) {
        // This should be safe because inbound_tx is an unbounded channel,
        // and is only dropped when the whole channel is dropped.
        let _ = self.inbound_tx.try_send(message);
    }
}

#[derive(Debug)]
pub struct Extension {
    name: String,
    channel: u64,
    local_id: u64,
    outbound_tx: Sender<ChannelMessage>,
    inbound_rx: Receiver<Vec<u8>>,
    write_state: WriteState,
    read_state: Option<Vec<u8>>,
}

impl std::clone::Clone for Extension {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            channel: self.channel,
            local_id: self.local_id,
            outbound_tx: self.outbound_tx.clone(),
            inbound_rx: self.inbound_rx.clone(),
            write_state: WriteState::Idle,
            read_state: None,
        }
    }
}

type SendFuture = Pin<Box<dyn Future<Output = io::Result<()>> + Send + Sync + 'static>>;

enum WriteState {
    Sending(SendFuture, usize),
    Idle,
}

impl std::fmt::Debug for WriteState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteState::Sending(_, len) => {
                write!(f, "Sending(len={})", len)
            }
            WriteState::Idle => write!(f, "Idle"),
        }
    }
}

impl Extension {
    pub async fn send(&self, message: Vec<u8>) {
        let message = ExtensionMessage::new(self.local_id, message);
        let message = ChannelMessage::new(self.channel, Message::Extension(message));
        self.outbound_tx.send(message).await.unwrap()
    }

    pub fn send_pinned(&self, message: Vec<u8>) -> SendFuture {
        let message = ExtensionMessage::new(self.local_id, message);
        let message = ChannelMessage::new(self.channel, Message::Extension(message));
        // TODO: It would be nice to do this without cloning, but I didn't find a way so far.
        let fut = send_message(self.outbound_tx.clone(), message);
        Box::pin(fut)
    }
}

pub async fn send_message(
    sender: Sender<ChannelMessage>,
    message: ChannelMessage,
) -> io::Result<()> {
    sender
        .send(message)
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::Interrupted, format!("Channel error: {}", e)))
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

impl AsyncRead for Extension {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let mut this = self.get_mut();
        let message = if let Some(message) = this.read_state.take() {
            message
        } else {
            let message = ready!(Pin::new(&mut this).poll_next(cx));
            message.ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "Channel closed"))?
        };
        let len = message.len().min(buf.len());
        buf[..len].copy_from_slice(&message[..len]);
        if message.len() > len {
            this.read_state = Some(message[len..].to_vec());
        } else {
            this.read_state = None
        }
        Poll::Ready(Ok(len))
    }
}

impl AsyncWrite for Extension {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        loop {
            match this.write_state {
                WriteState::Idle => {
                    let len = buf.len().min(MAX_BODY_SIZE);
                    let fut = this.send_pinned(buf.to_vec());
                    this.write_state = WriteState::Sending(fut, len);
                }
                WriteState::Sending(ref mut fut, len) => {
                    let res = ready!(fut.poll(cx));
                    let res = res.map(|_| len);
                    this.write_state = WriteState::Idle;
                    return Poll::Ready(res);
                }
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
