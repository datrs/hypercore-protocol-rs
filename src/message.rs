use crate::schema::*;
use bytes::Bytes;
use pretty_hash::fmt as pretty_fmt;
use prost::Message as ProstMessage;
use simple_message_channels::Message as ChannelMessage;
use std::fmt;
use std::io::{Error, ErrorKind, Result};

/// A protocol message.
#[derive(Debug)]
pub enum Message {
    Open(Open),
    Options(Options),
    Status(Status),
    Have(Have),
    Unhave(Unhave),
    Want(Want),
    Unwant(Unwant),
    Request(Request),
    Cancel(Cancel),
    Data(Data),
    Close(Close),
    Extension(ExtensionMessage),
}

/// A extension message (not yet supported properly).
#[derive(Debug)]
pub struct ExtensionMessage {
    pub id: u64,
    pub message: Vec<u8>,
}
impl ExtensionMessage {
    fn _new(id: u64, message: Vec<u8>) -> Self {
        Self { id, message }
    }

    fn decode(buf: &[u8]) -> Result<Self> {
        let mut id: u64 = 0;
        let id_len = varinteger::decode(buf, &mut id);
        Ok(Self {
            id,
            message: buf[id_len..].to_vec(),
        })
    }

    fn encoded_len(&self) -> usize {
        let id_len = varinteger::length(self.id);
        id_len + self.message.len()
    }

    fn encode(&self, buf: &mut [u8]) {
        let id_len = varinteger::length(self.id);
        varinteger::encode(self.id, &mut buf[..id_len]);
        buf[id_len..].copy_from_slice(&self.message)
    }

    fn to_vec(&self) -> Vec<u8> {
        let mut buf = vec![0u8; self.encoded_len()];
        self.encode(&mut buf);
        buf.to_vec()
    }
}

impl Message {
    pub fn decode(typ: u8, buf: Vec<u8>) -> Result<Self> {
        let bytes = Bytes::from(buf);
        // log::trace!("decode msg typ {}", typ);
        match typ {
            0 => Ok(Self::Open(Open::decode(bytes)?)),
            1 => Ok(Self::Options(Options::decode(bytes)?)),
            2 => Ok(Self::Status(Status::decode(bytes)?)),
            3 => Ok(Self::Have(Have::decode(bytes)?)),
            4 => Ok(Self::Unhave(Unhave::decode(bytes)?)),
            5 => Ok(Self::Want(Want::decode(bytes)?)),
            6 => Ok(Self::Unwant(Unwant::decode(bytes)?)),
            7 => Ok(Self::Request(Request::decode(bytes)?)),
            8 => Ok(Self::Cancel(Cancel::decode(bytes)?)),
            9 => Ok(Self::Data(Data::decode(bytes)?)),
            10 => Ok(Self::Close(Close::decode(bytes)?)),
            15 => Ok(Self::Extension(ExtensionMessage::decode(&bytes)?)),
            _ => Err(Error::new(ErrorKind::InvalidData, "Invalid message type")),
        }
    }

    pub fn encode(&mut self, ch: u64) -> Result<ChannelMessage> {
        match self {
            Self::Open(msg) => Ok(ChannelMessage::new(ch, 0, encode_msg(msg)?)),
            Self::Options(msg) => Ok(ChannelMessage::new(ch, 1, encode_msg(msg)?)),
            Self::Status(msg) => Ok(ChannelMessage::new(ch, 2, encode_msg(msg)?)),
            Self::Have(msg) => Ok(ChannelMessage::new(ch, 3, encode_msg(msg)?)),
            Self::Unhave(msg) => Ok(ChannelMessage::new(ch, 4, encode_msg(msg)?)),
            Self::Want(msg) => Ok(ChannelMessage::new(ch, 5, encode_msg(msg)?)),
            Self::Unwant(msg) => Ok(ChannelMessage::new(ch, 6, encode_msg(msg)?)),
            Self::Request(msg) => Ok(ChannelMessage::new(ch, 7, encode_msg(msg)?)),
            Self::Cancel(msg) => Ok(ChannelMessage::new(ch, 8, encode_msg(msg)?)),
            Self::Data(msg) => Ok(ChannelMessage::new(ch, 9, encode_msg(msg)?)),
            Self::Close(msg) => Ok(ChannelMessage::new(ch, 10, encode_msg(msg)?)),
            Self::Extension(msg) => Ok(ChannelMessage::new(ch, 15, msg.to_vec())),
            // _ => Err(Error::new(ErrorKind::InvalidData, "Invalid message type")),
        }
    }
}

fn encode_msg(msg: &mut impl ProstMessage) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; msg.encoded_len()];
    msg.encode(&mut buf)?;
    Ok(buf)
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Customize so only `x` and `y` are denoted.
        match self {
            Self::Open(msg) => write!(
                f,
                "Open(discovery_key: {}, capability <{}>)",
                pretty_fmt(&msg.discovery_key).unwrap(),
                msg.capability.as_ref().map_or(0, |c| c.len())
            ),
            Self::Data(msg) => write!(
                f,
                "Data(index {}, value: <{}>, nodes: {}, signature <{}>)",
                msg.index,
                msg.value.as_ref().map_or(0, |d| d.len()),
                msg.nodes.len(),
                msg.signature.as_ref().map_or(0, |d| d.len()),
            ),
            _ => write!(f, "{:?}", &self),
        }
    }
}
