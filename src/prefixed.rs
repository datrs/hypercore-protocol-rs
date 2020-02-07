use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use std::io::{Error, ErrorKind, Result};

// 4MB is the max message size (will be much smaller usually).
const MAX_MESSAGE_SIZE: u64 = 1024 * 1024 * 4;

/// Send a message with a varint prefix.
pub async fn write_prefixed<W>(mut writer: &mut W, buf: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    write_prefix(&mut writer, buf).await?;
    writer.write_all(&buf).await?;
    writer.flush().await
}

pub async fn _write_prefixed_batch<W>(mut writer: &mut W, bufs: Vec<&[u8]>) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    for buf in &bufs {
        write_prefix(&mut writer, buf).await?;
        writer.write_all(&buf).await?;
    }
    writer.flush().await
}

async fn write_prefix<W>(writer: &mut W, buf: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let len = buf.len();
    let prefix_len = varinteger::length(len as u64);
    let mut prefix_buf = vec![0u8; prefix_len];
    varinteger::encode(len as u64, &mut prefix_buf[..prefix_len]);
    writer.write_all(&prefix_buf).await
}

/// Receive a varint-prefixed message.
pub async fn read_prefixed<R>(reader: &mut R) -> Result<Vec<u8>>
where
    R: AsyncRead + Send + Unpin,
{
    let mut varint: u64 = 0;
    let mut factor = 1;
    let mut headerbuf = vec![0u8; 1];
    // Read initial varint (message length).
    loop {
        reader.read_exact(&mut headerbuf).await?;
        let byte = headerbuf[0];
        // Skip empty bytes (may be keepalive pings).
        if byte == 0 {
            continue;
        }

        varint = varint + (byte as u64 & 127) * factor;
        if byte < 128 {
            break;
        }
        if varint > MAX_MESSAGE_SIZE {
            return Err(Error::new(ErrorKind::InvalidInput, "Message too long"));
        }
        factor = factor * 128;
    }

    // Read main message.
    let mut messagebuf = vec![0u8; varint as usize];
    reader.read_exact(&mut messagebuf).await?;
    // log::trace!("recv len {}", messagebuf.len());
    Ok(messagebuf)
}

// enum VarintResult {
//     Ok(u4),
//     BufTooShort,
//     Overflow
// }
// fn read_varint(buf: &[u8]) -> Result<Option<u64>> {
// }

// struct Varint {
//     factor: u32,
//     result: u64
// }

// impl Varint {
//     pub fn new() -> Varint {
//         Varint { factor: 1, result: 0 }
//     }

//     pub fn read_byte(byte: &[u8; 1]) {
//     }
// }
