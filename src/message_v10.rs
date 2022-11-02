use crate::message::{EncodeError, FrameType};
use crate::schema::*;
use crate::util::{stat_uint24_le, write_uint24_le};
use hypercore::compact_encoding::{CompactEncoding, State};
use pretty_hash::fmt as pretty_fmt;
use std::fmt;
use std::io;

/// Encode data into a buffer.
///
/// This trait is implemented on data frames and their components
/// (channel messages, messages, and individual message types through prost).
pub trait Encoder: Sized + fmt::Debug {
    /// Calculates the length that the encoded message needs.
    fn encoded_len(&mut self) -> usize;

    /// Encodes the message to a buffer.
    ///
    /// An error will be returned if the buffer does not have sufficient capacity.
    fn encode(&mut self, buf: &mut [u8]) -> Result<usize, EncodeError>;
}

impl Encoder for &[u8] {
    fn encoded_len(&mut self) -> usize {
        self.len()
    }

    fn encode(&mut self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if len > buf.len() {
            return Err(EncodeError::new(len));
        }
        buf[..len].copy_from_slice(&self[..]);
        Ok(len)
    }
}

/// A frame of data, either a buffer or a message.
#[derive(Clone, PartialEq)]
pub enum Frame {
    /// A raw batch binary buffer. Used in the handshaking phase.
    RawBatch(Vec<Vec<u8>>),
    /// Message batch, containing one or more channel messsages. Used for everything after the handshake.
    MessageBatch(Vec<ChannelMessage>),
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Frame::RawBatch(batch) => write!(f, "Frame(RawBatch <{}>)", batch.len()),
            Frame::MessageBatch(messages) => write!(f, "Frame({:?})", messages),
        }
    }
}

impl From<ChannelMessage> for Frame {
    fn from(m: ChannelMessage) -> Self {
        Self::MessageBatch(vec![m])
    }
}

impl From<Vec<u8>> for Frame {
    fn from(m: Vec<u8>) -> Self {
        Self::RawBatch(vec![m])
    }
}

impl Frame {
    /// Decodes a frame from a buffer containing multiple concurrent messages.
    pub fn decode_multiple(buf: &[u8], frame_type: &FrameType) -> Result<Self, io::Error> {
        match frame_type {
            FrameType::Raw => {
                let mut index = 0;
                let mut raw_batch: Vec<Vec<u8>> = vec![];
                while index < buf.len() {
                    // There might be zero bytes in between, and with LE, the next message will
                    // start with a non-zero
                    if buf[index] == 0 {
                        index += 1;
                        continue;
                    }
                    let stat = stat_uint24_le(&buf[index..]);
                    if let Some((header_len, body_len)) = stat {
                        raw_batch.push(
                            buf[index + header_len..index + header_len + body_len as usize]
                                .to_vec(),
                        );
                        index += header_len + body_len as usize;
                    } else {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "received invalid data in raw batch",
                        ));
                    }
                }
                Ok(Frame::RawBatch(raw_batch))
            }
            FrameType::Message => {
                let mut index = 0;
                let mut combined_messages: Vec<ChannelMessage> = vec![];
                while index < buf.len() {
                    // There might be zero bytes in between, and with LE, the next message will
                    // start with a non-zero
                    if buf[index] == 0 {
                        index += 1;
                        continue;
                    }

                    let stat = stat_uint24_le(&buf[index..]);
                    if let Some((header_len, body_len)) = stat {
                        let (frame, length) = Self::decode_message(
                            &buf[index + header_len..index + header_len + body_len as usize],
                        )?;
                        println!(
                            "Frame::decode_multiple, index={},len={}, frame_length={}, ",
                            index,
                            buf.len(),
                            length
                        );
                        if length != body_len as usize {
                            println!("WARNING: Did not know what to do with all the bytes, got {} but decoded {}", body_len, length);
                        }
                        if let Frame::MessageBatch(messages) = frame {
                            for message in messages {
                                combined_messages.push(message);
                            }
                        } else {
                            unreachable!("Can not get Raw messages");
                        }
                        index += header_len + body_len as usize;
                    } else {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "received invalid data in multi-message chunk",
                        ));
                    }
                }
                Ok(Frame::MessageBatch(combined_messages))
            }
        }
    }

    /// Decode a frame from a buffer.
    pub fn decode(buf: &[u8], frame_type: &FrameType) -> Result<Self, io::Error> {
        match frame_type {
            FrameType::Raw => Ok(Frame::RawBatch(vec![buf.to_vec()])),
            FrameType::Message => {
                let (frame, _) = Self::decode_message(buf)?;
                Ok(frame)
            }
        }
    }

    fn decode_message(buf: &[u8]) -> Result<(Self, usize), io::Error> {
        if buf.len() >= 5 && buf[0] == 0x00 {
            if buf[1] == 0x00 {
                // Batch of messages
                let mut messages: Vec<ChannelMessage> = vec![];
                let mut state = State::new_with_start_and_end(2, buf.len());

                // First, there is the original channel
                let mut current_channel: u64 = state.decode(&buf);
                while state.start < state.end {
                    // Length of the message is inbetween here
                    let channel_message_length: usize = state.decode(&buf);
                    println!(
                        "CHL={}, CC={}, s={:?}",
                        channel_message_length, current_channel, state
                    );
                    if state.start + channel_message_length > state.end {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "received invalid message length",
                        ));
                    }
                    // Then the actual message
                    let (channel_message, _) = ChannelMessage::decode(
                        &buf[state.start..state.start + channel_message_length],
                        current_channel,
                    )?;
                    messages.push(channel_message);
                    state.start += channel_message_length;

                    // After that, if there is an extra 0x00, that means the channel
                    // changed. This works because of LE encoding, and channels starting
                    // from the index 1.
                    if state.start < state.end && buf[state.start + 1] == 0x00 {
                        state.start += 1;
                        current_channel = state.decode(&buf);
                    }
                }
                Ok((Frame::MessageBatch(messages), state.start))
            } else if buf[1] == 0x01 {
                // Open message
                let (channel_message, length) = ChannelMessage::decode_open_message(&buf[2..])?;
                Ok((Frame::MessageBatch(vec![channel_message]), length))
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "received too short special message",
                ))
            }
        } else if buf.len() >= 2 {
            // Single message
            let mut state = State::from_buffer(buf);
            let channel: u64 = state.decode(&buf);
            let (channel_message, length) = ChannelMessage::decode(&buf[state.start..], channel)?;
            Ok((
                Frame::MessageBatch(vec![channel_message]),
                state.start + length,
            ))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("received too short message, {:02X?}", buf),
            ))
        }
    }

    fn preencode(&mut self, state: &mut State) -> usize {
        match self {
            Self::RawBatch(raw_batch) => {
                for raw in raw_batch {
                    state.end += raw.as_slice().encoded_len();
                }
            }
            Self::MessageBatch(messages) => {
                if messages.len() == 1 {
                    if let Message::Open(_) = &messages[0].message {
                        // This is a special case with 0x00, 0x01 intro bytes
                        state.end += 2 + &messages[0].encoded_len();
                    } else {
                        state.preencode(&messages[0].channel);
                        state.end += messages[0].encoded_len();
                    }
                } else if messages.len() > 1 {
                    // Two intro bytes 0x00 0x00, then channel id, then lengths
                    state.end += 2;
                    let mut current_channel: u64 = messages[0].channel;
                    state.preencode(&current_channel);
                    for message in messages.iter_mut() {
                        if message.channel != current_channel {
                            // Channel changed, need to add a 0x00 in between and then the new
                            // channel
                            state.end += 1;
                            state.preencode(&message.channel);
                            current_channel = message.channel;
                        }
                        let message_length = message.encoded_len();
                        state.preencode(&message_length);
                        state.end += message_length;
                    }
                }
            }
        }
        state.end
    }
}

impl Encoder for Frame {
    fn encoded_len(&mut self) -> usize {
        let body_len = self.preencode(&mut State::new());
        match self {
            Self::RawBatch(_) => body_len,
            Self::MessageBatch(_) => 3 + body_len,
        }
    }

    fn encode(&mut self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let mut state = State::new();
        let header_len = if let Self::RawBatch(_) = self { 0 } else { 3 };
        let body_len = self.preencode(&mut state);
        let len = body_len + header_len;
        if buf.len() < len {
            return Err(EncodeError::new(len));
        }
        match self {
            Self::RawBatch(ref raw_batch) => {
                for raw in raw_batch {
                    raw.as_slice().encode(buf)?;
                }
            }
            Self::MessageBatch(ref mut messages) => {
                write_uint24_le(body_len, buf);
                state.start += 3;
                if messages.len() == 1 {
                    if let Message::Open(_) = &messages[0].message {
                        // This is a special case with 0x00, 0x01 intro bytes
                        state.encode(&(0 as u8), buf);
                        state.encode(&(1 as u8), buf);
                        state.start += messages[0].encode(&mut buf[state.start..])?;
                    } else {
                        state.encode(&messages[0].channel, buf);
                        state.start += messages[0].encode(&mut buf[state.start..])?;
                    }
                } else if messages.len() > 1 {
                    // Two intro bytes 0x00 0x00, then channel id, then lengths
                    buf[header_len] = 0x00;
                    buf[header_len + 1] = 0x00;
                    state.start += 2;
                    let mut current_channel: u64 = messages[0].channel;
                    state.encode(&current_channel, buf);
                    for message in messages.iter_mut() {
                        if message.channel != current_channel {
                            // Channel changed, need to add a 0x00 in between and then the new
                            // channel
                            state.encode(&(0 as u8), buf);
                            state.encode(&message.channel, buf);
                            current_channel = message.channel;
                        }
                        let message_length = message.encoded_len();
                        state.encode(&message_length, buf);
                        state.start += message.encode(&mut buf[state.start..])?;
                    }
                }
            }
        };
        Ok(len)
    }
}

/// A protocol message.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub enum Message {
    Open(Open),
    Close(Close),
    Synchronize(Synchronize),
    Request(Request),
    Cancel(Cancel),
    Data(Data),
    NoData(NoData),
    Want(Want),
    Unwant(Unwant),
    Bitfield(Bitfield),
    Range(Range),
    Extension(Extension),
}

impl Message {
    /// Wire type of this message.
    pub fn typ(&self) -> u64 {
        match self {
            Self::Synchronize(_) => 0,
            Self::Request(_) => 1,
            Self::Cancel(_) => 2,
            Self::Data(_) => 3,
            Self::NoData(_) => 4,
            Self::Want(_) => 5,
            Self::Unwant(_) => 6,
            Self::Bitfield(_) => 7,
            Self::Range(_) => 8,
            Self::Extension(_) => 9,
            value => unimplemented!("{} does not have a type", value),
        }
    }

    /// Decode a message from a buffer based on type.
    pub fn decode(buf: &[u8], typ: u64) -> io::Result<(Self, usize)> {
        let mut state = State::from_buffer(buf);
        let message = match typ {
            0 => Ok(Self::Synchronize(state.decode(buf))),
            1 => Ok(Self::Request(state.decode(buf))),
            2 => Ok(Self::Cancel(state.decode(buf))),
            3 => Ok(Self::Data(state.decode(buf))),
            4 => Ok(Self::NoData(state.decode(buf))),
            5 => Ok(Self::Want(state.decode(buf))),
            6 => Ok(Self::Unwant(state.decode(buf))),
            7 => Ok(Self::Bitfield(state.decode(buf))),
            8 => Ok(Self::Range(state.decode(buf))),
            9 => Ok(Self::Extension(state.decode(buf))),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid message type: {}", typ),
            )),
        }?;
        Ok((message, state.start))
    }

    /// Pre-encodes a message to state, returns length
    pub fn preencode(&mut self, state: &mut State) -> usize {
        match self {
            Self::Open(ref message) => state.preencode(message),
            Self::Synchronize(ref message) => state.preencode(message),
            Self::Request(ref message) => state.preencode(message),
            Self::Cancel(ref message) => state.preencode(message),
            Self::Data(ref message) => state.preencode(message),
            Self::NoData(ref message) => state.preencode(message),
            Self::Want(ref message) => state.preencode(message),
            Self::Unwant(ref message) => state.preencode(message),
            Self::Bitfield(ref message) => state.preencode(message),
            Self::Range(ref message) => state.preencode(message),
            Self::Extension(ref message) => state.preencode(message),
            value => unimplemented!("{} can not be pre-encoded", value),
        }
        state.end
    }

    /// Encodes a message to a given buffer, using preencoded state, results size
    pub fn encode(&mut self, state: &mut State, buf: &mut [u8]) -> usize {
        match self {
            Self::Open(ref message) => state.encode(message, buf),
            Self::Synchronize(ref message) => state.encode(message, buf),
            Self::Request(ref message) => state.encode(message, buf),
            Self::Cancel(ref message) => state.encode(message, buf),
            Self::Data(ref message) => state.encode(message, buf),
            Self::NoData(ref message) => state.encode(message, buf),
            Self::Want(ref message) => state.encode(message, buf),
            Self::Unwant(ref message) => state.encode(message, buf),
            Self::Bitfield(ref message) => state.encode(message, buf),
            Self::Range(ref message) => state.encode(message, buf),
            Self::Extension(ref message) => state.encode(message, buf),
            value => unimplemented!("{} can not be encoded", value),
        }
        state.start
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(msg) => write!(
                f,
                "Open(discovery_key: {}, capability <{}>)",
                pretty_fmt(&msg.discovery_key).unwrap(),
                msg.capability.as_ref().map_or(0, |c| c.len())
            ),
            Self::Data(msg) => write!(
                f,
                "Data(request: {}, fork: {}, block: {}, hash: {}, seek: {}, upgrade: {})",
                msg.request,
                msg.fork,
                msg.block.is_some(),
                msg.hash.is_some(),
                msg.seek.is_some(),
                msg.upgrade.is_some(),
            ),
            _ => write!(f, "{:?}", &self),
        }
    }
}

/// A message on a channel.
#[derive(Clone)]
pub struct ChannelMessage {
    pub channel: u64,
    pub message: Message,
    state: Option<State>,
}

impl PartialEq for ChannelMessage {
    fn eq(&self, other: &Self) -> bool {
        self.channel == other.channel && self.message == other.message
    }
}

impl fmt::Debug for ChannelMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChannelMessage({}, {})", self.channel, self.message)
    }
}

impl ChannelMessage {
    /// Create a new message.
    pub fn new(channel: u64, message: Message) -> Self {
        Self {
            channel,
            message,
            state: None,
        }
    }

    /// Consume self and return (channel, Message).
    pub fn into_split(self) -> (u64, Message) {
        (self.channel, self.message)
    }

    /// Decodes an open message for a channel message from a buffer.
    ///
    /// Note: `buf` has to have a valid length, and without the 3 LE
    /// bytes in it
    pub fn decode_open_message(buf: &[u8]) -> io::Result<(Self, usize)> {
        if buf.len() <= 5 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "received too short Open message",
            ));
        }

        let mut state = State::new_with_start_and_end(0, buf.len());
        let open_msg: Open = state.decode(buf);
        Ok((
            Self {
                channel: open_msg.channel,
                message: Message::Open(open_msg),
                state: None,
            },
            state.start,
        ))
    }

    /// Decode a normal channel message from a buffer.
    ///
    /// Note: `buf` has to have a valid length, and without the 3 LE
    /// bytes in it
    pub fn decode(buf: &[u8], channel: u64) -> io::Result<(Self, usize)> {
        println!("ChannelMessage::decode buf({}): {:02X?}", buf.len(), buf);
        if buf.len() <= 1 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "received empty message",
            ));
        }
        let mut state = State::from_buffer(buf);
        let typ: u64 = state.decode(&buf);
        let (message, length) = Message::decode(&buf[state.start..], typ)?;
        Ok((
            Self {
                channel,
                message,
                state: None,
            },
            state.start + length,
        ))
    }

    /// Performance optimization for letting calling encoded_len() already do
    /// the preencode phase of compact_encoding.
    fn prepare_state(&mut self) {
        if self.state.is_none() {
            let state = if let Message::Open(_) = self.message {
                // Open message doesn't have a type
                // https://github.com/mafintosh/protomux/blob/43d5192f31e7a7907db44c11afef3195b7797508/index.js#L41
                let mut state = State::new();
                self.message.preencode(&mut state);
                state
            } else if let Message::Close(_) = self.message {
                // Close message isn't sent
                State::new()
            } else {
                // The header is the channel id uint followed by message type uint
                // https://github.com/mafintosh/protomux/blob/43d5192f31e7a7907db44c11afef3195b7797508/index.js#L179
                let mut state = State::new();
                let typ = self.message.typ();
                state.preencode(&typ);
                self.message.preencode(&mut state);
                state
            };
            self.state = Some(state);
        }
    }
}

impl Encoder for ChannelMessage {
    fn encoded_len(&mut self) -> usize {
        self.prepare_state();
        self.state.as_ref().unwrap().end
    }

    fn encode(&mut self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        println!("ChannelMessage::encode, {:02X?}", self.message);
        self.prepare_state();
        let state = self.state.as_mut().unwrap();
        println!("ChannelMessage::encode, state found at {:?}", state);
        let len: usize = if let Message::Open(_) = self.message {
            // Open message is different in that the type byte is missing
            self.message.encode(state, buf);
            state.start
        } else if let Message::Close(_) = self.message {
            // Close message isn't sent
            state.start
        } else {
            let typ = self.message.typ();
            state.encode(&typ, buf);
            self.message.encode(state, buf);
            state.start
        };
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypercore::{
        DataBlock, DataHash, DataSeek, DataUpgrade, Node, RequestBlock, RequestSeek, RequestUpgrade,
    };

    macro_rules! message_enc_dec {
        ($( $msg:expr ),*) => {
            $(
                let channel = rand::random::<u8>() as u64;
                let mut channel_message = ChannelMessage::new(channel, $msg);
                let encoded_len = channel_message.encoded_len();
                let mut buf = vec![0u8; encoded_len];
                let n = channel_message.encode(&mut buf[..]).expect("Failed to encode message");
                let decoded = ChannelMessage::decode(&buf[..n], channel).expect("Failed to decode message").0.into_split();
                assert_eq!(channel, decoded.0);
                assert_eq!($msg, decoded.1);
            )*
        }
    }

    #[test]
    fn message_encode_decode() {
        message_enc_dec! {
            Message::Synchronize(Synchronize{
                fork: 0,
                can_upgrade: true,
                downloading: true,
                uploading: true,
                length: 5,
                remote_length: 0,
            }),
            Message::Request(Request {
                id: 1,
                fork: 1,
                block: Some(RequestBlock {
                    index: 5,
                    nodes: 10,
                }),
                hash: Some(RequestBlock {
                    index: 20,
                    nodes: 0
                }),
                seek: Some(RequestSeek {
                    bytes: 10
                }),
                upgrade: Some(RequestUpgrade {
                    start: 0,
                    length: 10
                })
            }),
            Message::Cancel(Cancel {
                request: 1,
            }),
            Message::Data(Data{
                request: 1,
                fork: 5,
                block: Some(DataBlock {
                    index: 5,
                    nodes: vec![Node::new(1, vec![0x01; 32], 100)],
                    value: vec![0xFF; 10]
                }),
                hash: Some(DataHash {
                    index: 20,
                    nodes: vec![Node::new(2, vec![0x02; 32], 200)],
                }),
                seek: Some(DataSeek {
                    bytes: 10,
                    nodes: vec![Node::new(3, vec![0x03; 32], 300)],
                }),
                upgrade: Some(DataUpgrade {
                    start: 0,
                    length: 10,
                    nodes: vec![Node::new(4, vec![0x04; 32], 400)],
                    additional_nodes: vec![Node::new(5, vec![0x05; 32], 500)],
                    signature: vec![0xAB; 32]
                })
            }),
            Message::NoData(NoData {
                request: 2,
            }),
            Message::Want(Want {
                start: 0,
                length: 100,
            }),
            Message::Unwant(Unwant {
                start: 10,
                length: 2,
            }),
            Message::Bitfield(Bitfield {
                start: 20,
                bitfield: vec![0x89ABCDEF, 0x00, 0xFFFFFFFF],
            }),
            Message::Range(Range {
                drop: true,
                start: 12345,
                length: 100000
            }),
            Message::Extension(Extension {
                name: "custom_extension/v1/open".to_string(),
                message: vec![0x44, 20]
            })
        };
    }
}
