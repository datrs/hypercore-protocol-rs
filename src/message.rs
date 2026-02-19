use crate::schema::*;
use compact_encoding::{
    CompactEncoding, EncodingError, EncodingErrorKind, VecEncodable, decode_usize, take_array,
    write_array,
};
use pretty_hash::fmt as pretty_fmt;
use std::{fmt, io};
use tracing::{debug, instrument, trace, warn};

const OPEN_MESSAGE_PREFIX: [u8; 2] = [0, 1];
const CLOSE_MESSAGE_PREFIX: [u8; 2] = [0, 3];
const MULTI_MESSAGE_PREFIX: [u8; 2] = [0, 0];
const CHANNEL_CHANGE_SEPERATOR: [u8; 1] = [0];

#[instrument(skip_all err)]
pub(crate) fn decode_unframed_channel_messages(
    buf: &[u8],
) -> Result<(Vec<ChannelMessage>, usize), io::Error> {
    let og_len = buf.len();
    if og_len >= 3 && buf[0] == 0x00 {
        // batch of NOT open/close messages
        if buf[1] == 0x00 {
            let (_, mut buf) = take_array::<2>(buf)?;
            // Batch of messages
            let mut messages: Vec<ChannelMessage> = vec![];

            // First, there is the original channel
            let mut current_channel;
            (current_channel, buf) = u64::decode(buf)?;
            while !buf.is_empty() {
                // Length of the message is inbetween here
                let channel_message_length;
                (channel_message_length, buf) = decode_usize(buf)?;
                if channel_message_length > buf.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "received invalid message length: [{channel_message_length}]
\tbut we have [{}] remaining bytes.
\tInitial buffer size [{og_len}]",
                            buf.len()
                        ),
                    ));
                }
                // Then the actual message
                let channel_message;
                let bl = buf.len();
                (channel_message, buf) = ChannelMessage::decode_with_channel(buf, current_channel)?;
                trace!(
                    "Decoded ChannelMessage::{:?} using [{} bytes]",
                    channel_message.message,
                    bl - buf.len()
                );
                messages.push(channel_message);
                // After that, if there is an extra 0x00, that means the channel
                // changed. This works because of LE encoding, and channels starting
                // from the index 1.
                if !buf.is_empty() && buf[0] == 0x00 {
                    (current_channel, buf) = u64::decode(buf)?;
                }
            }
            Ok((messages, og_len - buf.len()))
        } else if buf[1] == 0x01 {
            // Open message
            let (channel_message, length) = ChannelMessage::decode_open_message(&buf[2..])?;
            Ok((vec![channel_message], length + 2))
        } else if buf[1] == 0x03 {
            // Close message
            let (channel_message, length) = ChannelMessage::decode_close_message(&buf[2..])?;
            Ok((vec![channel_message], length + 2))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "received invalid special message",
            ))
        }
    } else if buf.len() >= 2 {
        trace!("Decoding single ChannelMessage");
        // Single message
        let og_len = buf.len();
        let (channel_message, buf) = ChannelMessage::decode_from_channel_and_message(buf)?;
        Ok((vec![channel_message], og_len - buf.len()))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("received too short message, {buf:?}"),
        ))
    }
}

fn vec_channel_messages_encoded_size(messages: &[ChannelMessage]) -> Result<usize, EncodingError> {
    Ok(match messages {
        [] => 0,
        [msg] => match msg.message {
            Message::Open(_) | Message::Close(_) => 2 + msg.encoded_size()?,
            _ => msg.encoded_size()?,
        },
        msgs => {
            let mut out = MULTI_MESSAGE_PREFIX.len();
            let mut current_channel: u64 = messages[0].channel;
            out += current_channel.encoded_size()?;
            for message in msgs.iter() {
                if message.channel != current_channel {
                    // Channel changed, need to add a 0x00 in between and then the new
                    // channel
                    out += CHANNEL_CHANGE_SEPERATOR.len() + message.channel.encoded_size()?;
                    current_channel = message.channel;
                }
                let message_length = message.message.encoded_size()?;
                out += message_length + (message_length as u64).encoded_size()?;
            }
            out
        }
    })
}

/// A protocol message.
#[derive(Debug, Clone, PartialEq)]
#[expect(missing_docs)]
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
    /// A local signalling message never sent over the wire
    LocalSignal((String, Vec<u8>)),
}

macro_rules! message_from {
    ($($val:ident),+) => {
        $(
            impl From<$val> for Message {
                fn from(value: $val) -> Self {
                    Message::$val(value)
                }
            }
        )*
    }
}
message_from!(
    Open,
    Close,
    Synchronize,
    Request,
    Cancel,
    Data,
    NoData,
    Want,
    Unwant,
    Bitfield,
    Range,
    Extension
);

macro_rules! decode_message {
    ($type:ty, $buf:expr) => {{
        let (x, rest) = <$type>::decode($buf)?;
        (Message::from(x), rest)
    }};
}

impl CompactEncoding for Message {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        let typ_size = if let Self::Open(_) | Self::Close(_) = &self {
            0
        } else {
            self.typ().encoded_size()?
        };
        let msg_size = match self {
            Self::LocalSignal(_) => Ok(0),
            Self::Open(x) => x.encoded_size(),
            Self::Close(x) => x.encoded_size(),
            Self::Synchronize(x) => x.encoded_size(),
            Self::Request(x) => x.encoded_size(),
            Self::Cancel(x) => x.encoded_size(),
            Self::Data(x) => x.encoded_size(),
            Self::NoData(x) => x.encoded_size(),
            Self::Want(x) => x.encoded_size(),
            Self::Unwant(x) => x.encoded_size(),
            Self::Bitfield(x) => x.encoded_size(),
            Self::Range(x) => x.encoded_size(),
            Self::Extension(x) => x.encoded_size(),
        }?;
        Ok(typ_size + msg_size)
    }

    #[instrument(skip_all, fields(name = self.name()))]
    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        debug!("Encoding {self:?}");
        let rest = if let Self::Open(_) | Self::Close(_) = &self {
            buffer
        } else {
            self.typ().encode(buffer)?
        };
        match self {
            Self::Open(x) => x.encode(rest),
            Self::Close(x) => x.encode(rest),
            Self::Synchronize(x) => x.encode(rest),
            Self::Request(x) => x.encode(rest),
            Self::Cancel(x) => x.encode(rest),
            Self::Data(x) => x.encode(rest),
            Self::NoData(x) => x.encode(rest),
            Self::Want(x) => x.encode(rest),
            Self::Unwant(x) => x.encode(rest),
            Self::Bitfield(x) => x.encode(rest),
            Self::Range(x) => x.encode(rest),
            Self::Extension(x) => x.encode(rest),
            Self::LocalSignal(_) => unimplemented!("do not encode LocalSignal"),
        }
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let (typ, rest) = u64::decode(buffer)?;
        Ok(match typ {
            0 => decode_message!(Synchronize, rest),
            1 => decode_message!(Request, rest),
            2 => decode_message!(Cancel, rest),
            3 => decode_message!(Data, rest),
            4 => decode_message!(NoData, rest),
            5 => decode_message!(Want, rest),
            6 => decode_message!(Unwant, rest),
            7 => decode_message!(Bitfield, rest),
            8 => decode_message!(Range, rest),
            9 => decode_message!(Extension, rest),
            _ => {
                return Err(EncodingError::new(
                    EncodingErrorKind::InvalidData,
                    &format!("Invalid message type to decode: {typ}"),
                ));
            }
        })
    }
}
impl Message {
    /// Wire type of this message.
    pub(crate) fn typ(&self) -> u64 {
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
    /// Get the name of the message
    pub fn name(&self) -> &'static str {
        match self {
            Message::Open(_) => "Open",
            Message::Close(_) => "Close",
            Message::Synchronize(_) => "Synchronize",
            Message::Request(_) => "Request",
            Message::Cancel(_) => "Cancel",
            Message::Data(_) => "Data",
            Message::NoData(_) => "NoData",
            Message::Want(_) => "Want",
            Message::Unwant(_) => "Unwant",
            Message::Bitfield(_) => "Bitfield",
            Message::Range(_) => "Range",
            Message::Extension(_) => "Extension",
            Message::LocalSignal(_) => "LocalSignal",
        }
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
pub(crate) struct ChannelMessage {
    pub(crate) channel: u64,
    pub(crate) message: Message,
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

impl fmt::Display for ChannelMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ChannelMessage {{ channel {}, message {} }}",
            self.channel,
            self.message.name()
        )
    }
}

impl ChannelMessage {
    /// Create a new message.
    pub(crate) fn new(channel: u64, message: Message) -> Self {
        Self { channel, message }
    }

    /// Consume self and return (channel, Message).
    pub(crate) fn into_split(self) -> (u64, Message) {
        (self.channel, self.message)
    }

    /// Decodes an open message for a channel message from a buffer.
    ///
    /// Note: `buf` has to have a valid length, and without the 3 LE
    /// bytes in it
    #[instrument(skip_all, err)]
    pub(crate) fn decode_open_message(buf: &[u8]) -> io::Result<(Self, usize)> {
        debug!("Decode ChannelMessage::Open");
        let og_len = buf.len();
        if og_len <= 5 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "received too short Open message",
            ));
        }

        let (open_msg, buf) = Open::decode(buf)?;
        Ok((
            Self {
                channel: open_msg.channel,
                message: Message::Open(open_msg),
            },
            og_len - buf.len(),
        ))
    }

    /// Decodes a close message for a channel message from a buffer.
    ///
    /// Note: `buf` has to have a valid length, and without the 3 LE
    /// bytes in it
    pub(crate) fn decode_close_message(buf: &[u8]) -> io::Result<(Self, usize)> {
        debug!("Decode ChannelMessage::Close");
        let og_len = buf.len();
        if buf.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "received too short Close message",
            ));
        }
        let (close, buf) = Close::decode(buf)?;
        Ok((
            Self {
                channel: close.channel,
                message: Message::Close(close),
            },
            og_len - buf.len(),
        ))
    }

    #[instrument(err, skip_all)]
    pub(crate) fn decode_from_channel_and_message(
        buf: &[u8],
    ) -> Result<(Self, &[u8]), EncodingError> {
        //<ChannelMessage as CompactEncoding>::decode(buf)
        let (channel, buf) = u64::decode(buf)?;
        let (message, buf) = <Message as CompactEncoding>::decode(buf)?;
        debug!(
            "Decode ChannelMessage{{ channel: {channel}, message: {} }}",
            message.name()
        );
        Ok((Self { channel, message }, buf))
    }
    /// Decode a normal channel message from a buffer.
    ///
    /// Note: `buf` has to have a valid length, and without the 3 LE
    /// bytes in it
    #[instrument(err, skip(buf))]
    pub(crate) fn decode_with_channel(buf: &[u8], channel: u64) -> io::Result<(Self, &[u8])> {
        if buf.len() <= 1 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("received empty message [{buf:?}]"),
            ));
        }
        let (message, buf) = <Message as CompactEncoding>::decode(buf)?;
        Ok((Self { channel, message }, buf))
    }
}

/// NB: currently this is just for a standalone channel message. ChannelMessages in a vec decode &
/// encode differently
impl CompactEncoding for ChannelMessage {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        let channel_size = if let Message::Open(_) | Message::Close(_) = &self.message {
            0
        } else {
            self.channel.encoded_size()?
        };

        Ok(channel_size + self.message.encoded_size()?)
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        let rest = if let Message::Open(_) | Message::Close(_) = &self.message {
            buffer
        } else {
            self.channel.encode(buffer)?
        };
        <Message as CompactEncoding>::encode(&self.message, rest)
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        ChannelMessage::decode_from_channel_and_message(buffer)
    }
}

impl VecEncodable for ChannelMessage {
    #[instrument(skip_all, ret)]
    fn vec_encoded_size(vec: &[Self]) -> Result<usize, EncodingError>
    where
        Self: Sized,
    {
        vec_channel_messages_encoded_size(vec)
    }

    #[instrument(skip_all, err)]
    fn vec_encode<'a>(vec: &[Self], buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError>
    where
        Self: Sized,
    {
        let in_buf_len = buffer.len();
        trace!(
            "Vec<ChannelMessage>::encode to buf.len() = [{}]",
            buffer.len()
        );
        let mut rest = buffer;
        match vec {
            [] => Ok(rest),
            [msg] => {
                rest = match msg.message {
                    Message::Open(_) => write_array(&OPEN_MESSAGE_PREFIX, rest)?,
                    Message::Close(_) => write_array(&CLOSE_MESSAGE_PREFIX, rest)?,
                    _ => msg.channel.encode(rest)?,
                };
                msg.message.encode(rest)
            }
            msgs => {
                rest = write_array(&MULTI_MESSAGE_PREFIX, rest)?;
                let mut current_channel: u64 = msgs[0].channel;
                rest = current_channel.encode(rest)?;
                for msg in msgs {
                    if msg.channel != current_channel {
                        rest = write_array(&CHANNEL_CHANGE_SEPERATOR, rest)?;
                        rest = msg.channel.encode(rest)?;
                        current_channel = msg.channel;
                    }
                    let msg_len = msg.message.encoded_size()?;
                    rest = (msg_len as u64).encode(rest)?;
                    rest = msg.message.encode(rest)?;
                }
                trace!("wrote [{}] bytes to buffer", in_buf_len - rest.len());
                Ok(rest)
            }
        }
    }

    fn vec_decode(buffer: &[u8]) -> Result<(Vec<Self>, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let mut combined_messages: Vec<ChannelMessage> = vec![];
        let mut rest = buffer;
        while !rest.is_empty() {
            let (msgs, length) = decode_unframed_channel_messages(rest)
                .map_err(|e| EncodingError::external(&format!("{e}")))?;
            rest = &rest[length..];
            combined_messages.extend(msgs);
        }
        Ok((combined_messages, rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypercore_schema::{
        DataBlock, DataHash, DataSeek, DataUpgrade, Node, RequestBlock, RequestSeek, RequestUpgrade,
    };

    macro_rules! message_enc_dec {
        ($( $msg:expr ),*) => {
            $(
                let channel = rand::random::<u8>() as u64;
                let channel_message = ChannelMessage::new(channel, $msg);
                let encoded_size = channel_message.encoded_size()?;
                let mut buf = vec![0u8; encoded_size];
                let rest = <ChannelMessage as CompactEncoding>::encode(&channel_message, &mut buf)?;
                assert!(rest.is_empty());
                let (decoded, rest) = <ChannelMessage as CompactEncoding>::decode(&buf)?;
                assert!(rest.is_empty());
                assert_eq!(decoded, channel_message);
            )*
        }
    }

    #[test]
    fn message_encode_decode() -> Result<(), EncodingError> {
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
                }),
                manifest: false,
                priority: 0
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
        Ok(())
    }

    #[test]
    fn enc_dec_vec_chan_message() -> Result<(), EncodingError> {
        let one = Message::Synchronize(Synchronize {
            fork: 0,
            length: 4,
            remote_length: 0,
            downloading: true,
            uploading: true,
            can_upgrade: true,
        });
        let two = Message::Range(Range {
            drop: false,
            start: 0,
            length: 4,
        });
        let msgs = vec![ChannelMessage::new(1, one), ChannelMessage::new(1, two)];
        let buff = msgs.to_encoded_bytes()?;
        let (result, rest) = <Vec<ChannelMessage> as CompactEncoding>::decode(&buff)?;
        assert!(rest.is_empty());
        assert_eq!(result, msgs);
        Ok(())
    }
}
