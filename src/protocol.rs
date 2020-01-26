use bytes::Bytes;
use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use futures::stream::TryStreamExt;
use prost::Message as ProstMessage;
use simple_message_channels::{Message, Reader, Writer};
use std::io::{Error, ErrorKind, Result};
use std::time::Duration;
use async_std::future;

use crate::schema;

const TIMEOUT_SECS: u64 = 10;

#[derive(Debug)]
enum ProtoMsg {
    Open(schema::Open),
    Options(schema::Options),
    Status(schema::Status),
    Have(schema::Have),
    Unhave(schema::Unhave),
    Want(schema::Want),
    Unwant(schema::Unwant),
    Request(schema::Request),
    Cancel(schema::Cancel),
    Data(schema::Data),
    Close(schema::Close),
}

impl ProtoMsg {
    fn decode(typ: u8, buf: Vec<u8>) -> Result<Self> {
        let bytes = Bytes::from(buf);
        match typ {
            0 => Ok(Self::Open(schema::Open::decode(bytes)?)),
            1 => Ok(Self::Options(schema::Options::decode(bytes)?)),
            2 => Ok(Self::Status(schema::Status::decode(bytes)?)),
            3 => Ok(Self::Have(schema::Have::decode(bytes)?)),
            4 => Ok(Self::Unhave(schema::Unhave::decode(bytes)?)),
            5 => Ok(Self::Want(schema::Want::decode(bytes)?)),
            6 => Ok(Self::Unwant(schema::Unwant::decode(bytes)?)),
            7 => Ok(Self::Request(schema::Request::decode(bytes)?)),
            8 => Ok(Self::Cancel(schema::Cancel::decode(bytes)?)),
            9 => Ok(Self::Data(schema::Data::decode(bytes)?)),
            10 => Ok(Self::Close(schema::Close::decode(bytes)?)),
            _ => Err(Error::new(ErrorKind::InvalidData, "Invalid message type")),
        }
    }

    fn encode(&mut self, ch: u64) -> Result<Message> {
        match self {
            Self::Open(msg) => Ok(Message::new(ch, 0, encode_msg(msg)?)),
            Self::Options(msg) => Ok(Message::new(ch, 1, encode_msg(msg)?)),
            Self::Status(msg) => Ok(Message::new(ch, 2, encode_msg(msg)?)),
            Self::Have(msg) => Ok(Message::new(ch, 3, encode_msg(msg)?)),
            Self::Unhave(msg) => Ok(Message::new(ch, 4, encode_msg(msg)?)),
            Self::Want(msg) => Ok(Message::new(ch, 5, encode_msg(msg)?)),
            Self::Unwant(msg) => Ok(Message::new(ch, 6, encode_msg(msg)?)),
            Self::Request(msg) => Ok(Message::new(ch, 7, encode_msg(msg)?)),
            Self::Cancel(msg) => Ok(Message::new(ch, 8, encode_msg(msg)?)),
            Self::Data(msg) => Ok(Message::new(ch, 9, encode_msg(msg)?)),
            Self::Close(msg) => Ok(Message::new(ch, 10, encode_msg(msg)?)),
            // _ => Err(Error::new(ErrorKind::InvalidData, "Invalid message type")),
        }
    }
}

pub async fn handle_connection<R, W>(reader: R, writer: W, is_initiator: bool) -> Result<()>
where
    R: AsyncRead + Clone + Send + Unpin + 'static,
    W: AsyncWrite + Clone + Unpin,
{
    let mut protocol = Protocol::new(reader, writer, is_initiator);
    protocol.listen().await?;
    Ok(())
}

struct Protocol<R, W> {
    // raw_reader: R,
    raw_writer: W,
    reader: Reader<R>,
    writer: Writer<W>,
    // is_initiator: bool,
}

impl<R, W> Protocol<R, W>
where
    R: AsyncRead + Clone + Send + Unpin + 'static,
    W: AsyncWrite + Clone + Unpin,
{
    fn new(reader: R, writer: W, _is_initiator: bool) -> Self {
        Protocol {
            // raw_reader: reader.clone(),
            raw_writer: writer.clone(),
            reader: Reader::new(reader.clone()),
            writer: Writer::new(writer.clone()),
            // is_initiator,
        }
    }

    async fn listen(&mut self) -> Result<()> {
        loop {
            let timeout = Duration::from_secs(TIMEOUT_SECS);
            let next = self.reader.try_next();

            match future::timeout(timeout, next).await {
                Err(_timeout_err) => {
                    self.ping().await?;
                }
                Ok(Ok(Some(message))) => {
                    self.onmessage(message).await?;
                }
                Ok(Ok(None)) => {
                    return Err(Error::new(ErrorKind::UnexpectedEof, "connection closed"));
                }
                Ok(Err(e)) => {
                    return Err(e);
                }
            }
        }
    }

    async fn onmessage(&mut self, message: Message) -> Result<()> {
        let parsed = ProtoMsg::decode(message.typ, message.message)?;
        eprintln!("recv: {:?}", parsed);
        let _result = match parsed {
            ProtoMsg::Open(msg) => self.onopen(message.channel, msg).await,
            _ => Ok(()),
        };
        Ok(())
    }

    async fn onopen(&mut self, ch: u64, msg: schema::Open) -> Result<()> {
        self.send(
            ch,
            ProtoMsg::Open(schema::Open {
                discovery_key: msg.discovery_key,
                capability: None,
            }),
        )
        .await?;
        self.send(
            ch,
            ProtoMsg::Want(schema::Want {
                start: 0,
                length: Some(100),
            }),
        )
        .await?;
        self.send(
            ch,
            ProtoMsg::Request(schema::Request {
                index: 0,
                bytes: None,
                hash: None,
                nodes: None,
            }),
        )
        .await?;
        Ok(())
    }

    async fn send(&mut self, ch: u64, mut msg: ProtoMsg) -> Result<()> {
        eprintln!("send {} {:?}", ch, msg);
        let encoded = msg.encode(ch)?;
        self.writer.send(encoded).await?;
        Ok(())
    }

    async fn ping(&mut self) -> Result<()> {
        let buf = vec![0u8];
        self.raw_writer.write_all(&buf).await?;
        self.raw_writer.flush().await?;
        Ok(())
    }
}

fn encode_msg(msg: &mut impl ProstMessage) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; msg.encoded_len()];
    msg.encode(&mut buf)?;
    Ok(buf)
}
