use crate::schema::*;
use bytes::Bytes;
use pretty_hash::fmt as pretty_fmt;
use prost::Message as ProstMessage;
use std::fmt;
use std::io::{Error, ErrorKind, Result};

use crate::constants::MAX_MESSAGE_SIZE;

/// A protocol message.
#[derive(Debug, Clone, PartialEq)]
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

impl Message {
    pub fn decode(typ: u64, body: Vec<u8>) -> Result<Self> {
        let bytes = Bytes::from(body);
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
            15 => Ok(Self::Extension(ExtensionMessage::decode(bytes)?)),
            _ => Err(Error::new(ErrorKind::InvalidData, "Invalid message type")),
        }
    }

    pub fn encode(&self) -> Result<(u64, Vec<u8>)> {
        match self {
            Self::Open(msg) => Ok((0, encode_msg(msg)?)),
            Self::Options(msg) => Ok((1, encode_msg(msg)?)),
            Self::Status(msg) => Ok((2, encode_msg(msg)?)),
            Self::Have(msg) => Ok((3, encode_msg(msg)?)),
            Self::Unhave(msg) => Ok((4, encode_msg(msg)?)),
            Self::Want(msg) => Ok((5, encode_msg(msg)?)),
            Self::Unwant(msg) => Ok((6, encode_msg(msg)?)),
            Self::Request(msg) => Ok((7, encode_msg(msg)?)),
            Self::Cancel(msg) => Ok((8, encode_msg(msg)?)),
            Self::Data(msg) => Ok((9, encode_msg(msg)?)),
            Self::Close(msg) => Ok((10, encode_msg(msg)?)),
            Self::Extension(msg) => Ok((15, msg.to_vec())),
        }
    }

    pub fn into_channel_message(self, channel: u64) -> ChannelMessage {
        ChannelMessage::new(channel, self)
    }
}

fn encode_msg(msg: &impl ProstMessage) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(msg.encoded_len());
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

/// A message on a channel.
pub struct ChannelMessage {
    pub channel: u64,
    pub message: Message,
}

impl fmt::Debug for ChannelMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChannelMessage({}, {})", self.channel, self.message)
    }
}

impl ChannelMessage {
    /// Create a new message.
    pub fn new(channel: u64, message: Message) -> Self {
        Self { channel, message }
    }

    /// Consume self and return (channel, Message).
    pub fn into_split(self) -> (u64, Message) {
        (self.channel, self.message)
    }

    /// Decode a channel message from a buffer.
    ///
    /// Note: `buf` has to have a valid length, and the length
    /// prefix has to be removed already.
    pub fn decode(mut buf: Vec<u8>) -> Result<Self> {
        if buf.is_empty() {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                "received empty message",
            ));
        }
        let mut header = 0 as u64;
        let headerlen = varinteger::decode(&buf, &mut header);
        let body = buf.split_off(headerlen);
        let channel = header >> 4;
        let typ = header & 0b1111;
        let message = Message::decode(typ, body)?;

        let channel_message = Self { channel, message };

        Ok(channel_message)
    }

    /// Encode a channel message into a buffer.
    ///
    /// The result has to be prefixed with a varint containing the buffer length
    /// before sending it over a stream.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let (typ, body) = self.message.encode()?;

        let header = self.channel << 4 | typ;
        let len_header = varinteger::length(header);
        let len = body.len() + len_header;

        if len as u64 > MAX_MESSAGE_SIZE {
            return Err(Error::new(ErrorKind::InvalidInput, "Message too long"));
        }

        let mut buf = vec![0u8; len];
        varinteger::encode(header, &mut buf[..len_header]);

        buf[len_header..].copy_from_slice(&body);
        Ok(buf)
    }
}

/// A extension message (not yet supported properly).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionMessage {
    pub id: u64,
    pub message: Vec<u8>,
}
impl ExtensionMessage {
    fn _new(id: u64, message: Vec<u8>) -> Self {
        Self { id, message }
    }

    fn decode(buf: impl AsRef<[u8]>) -> Result<Self> {
        let buf = buf.as_ref();
        let mut id: u64 = 0;
        let id_len = varinteger::decode(&buf, &mut id);
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

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! message_enc_dec {
        ($( $msg:expr ),*) => {
            $(
                let (typ, body) = $msg.encode().expect("Failed to encode proto");
                let decoded = Message::decode(typ, body).expect("Failed to decode message");
                assert_eq!($msg, decoded);
            )*
        }
    }

    #[test]
    fn encode_decode() {
        message_enc_dec! {
            Message::Open(Open{
                discovery_key: vec![2u8; 20],
                capability: None
            }),
            Message::Options(Options {
                extensions: vec!["test ext".to_string()],
                ack: None
            }),
            Message::Status(Status {
                uploading: Some(true),
                downloading: Some(false)
            }),
            Message::Have(Have {
                start: 0,
                length: Some(100),
                bitfield: None,
                ack: Some(true)
            }),
            Message::Unhave(Unhave {
                start: 0,
                length: Some(100),
            }),
            Message::Want(Want {
                start: 0,
                length: Some(100),
            }),
            Message::Request(Request {
                index: 0,
                bytes: None,
                hash: Some(true),
                nodes: None
            }),
            Message::Cancel(Cancel{
                index: 10,
                bytes: Some(10),
                hash: Some(true)
            }),
            Message::Data(Data {
                index: 1,
                value: None,
                nodes: vec![],
                signature: None
            }),
            Message::Close(Close {
                discovery_key: Some(vec![1u8; 10])
            })
        };
    }
}
