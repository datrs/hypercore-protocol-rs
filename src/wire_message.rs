use std::io::{Error, ErrorKind, Result};

use crate::constants::MAX_MESSAGE_SIZE;

/// A SMC message.
#[derive(Debug)]
pub struct Message {
    pub channel: u64,
    pub typ: u8,
    pub message: Vec<u8>,
}

impl Message {
    /// Create a new message.
    pub fn new(channel: u64, typ: u8, message: Vec<u8>) -> Message {
        Message {
            channel,
            typ,
            message,
        }
    }

    /// Decode a message from `buf` (bytes).
    ///
    /// Note: `buf` has to have a valid length, and the length
    /// prefix has to be removed already.
    pub fn from_buf(buf: &[u8]) -> Result<Message> {
        if buf.is_empty() {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                "received empty message",
            ));
        }
        let mut header = 0 as u64;
        let headerlen = varinteger::decode(buf, &mut header);
        let msg = &buf[headerlen..];
        let channel = header >> 4;
        let typ = header & 0b1111;
        let message = Message {
            channel,
            typ: typ as u8,
            message: msg.to_vec(),
        };
        Ok(message)
    }

    /// Encode a message body into a buffer.
    ///
    /// The result has to be prefixed with a varint containing the buffer length
    /// before sending it over a stream.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let header = self.channel << 4 | self.typ as u64;
        let len_header = varinteger::length(header);
        let len = self.message.len() + len_header;

        if len as u64 > MAX_MESSAGE_SIZE {
            return Err(Error::new(ErrorKind::InvalidInput, "Message too long"));
        }

        let mut buf = vec![0u8; len];
        varinteger::encode(header, &mut buf[..len_header]);
        buf[len_header..].copy_from_slice(&self.message);
        Ok(buf)
    }
}
