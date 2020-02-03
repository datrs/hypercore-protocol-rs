use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};
use prost::Message;
use rand::Rng;
use snow;
use snow::{Builder, Error as SnowError, HandshakeState};
use std::io;
use std::io::{Error, ErrorKind, Result};
// use std::clone::Clone;
// use std::sync::Arc;
// use crate::CloneableStream;
use varinteger;

use crate::schema;

const MAX_MESSAGE_SIZE: u64 = 65535;

pub fn build_handshake_state(is_initiator: bool) -> std::result::Result<HandshakeState, SnowError> {
    static PATTERN: &'static str = "Noise_XX_25519_XChaChaPoly_BLAKE2b";
    let builder: Builder<'_> = Builder::new(PATTERN.parse()?);
    let key_pair = builder.generate_keypair().unwrap();
    eprintln!("local pubkey: {:x?}", &key_pair.public);
    let noise = if is_initiator {
        builder
            .local_private_key(&key_pair.private)
            .build_initiator()
    } else {
        builder
            .local_private_key(&key_pair.private)
            .build_responder()
    };
    noise
}

pub async fn handshake<R, W>(
    reader: R,
    writer: W,
    is_initiator: bool,
) -> std::result::Result<(R, W), Error>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin,
{
    eprintln!("start handshaking, initiator: {}", is_initiator);

    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    let map_err = |e| {
        Error::new(
            ErrorKind::PermissionDenied,
            format!("Handshake error: {}", e),
        )
    };

    let mut noise = build_handshake_state(is_initiator).map_err(map_err)?;

    let local_nonce = generate_nonce();
    let payload = encode_nonce_msg(local_nonce);

    let mut tx_buf = vec![0u8; 65535];
    let mut rx_buf = vec![0u8; 65535];
    let mut rx_len;
    let mut tx_len;

    if is_initiator {
        tx_len = noise
            .write_message(&payload, &mut tx_buf)
            .map_err(map_err)?;
        send(&mut writer, &tx_buf[..tx_len]).await?;
    }

    let msg = recv(&mut reader).await?;
    rx_len = noise.read_message(&msg, &mut rx_buf).map_err(map_err)?;

    tx_len = noise
        .write_message(&payload, &mut tx_buf)
        .map_err(map_err)?;
    send(&mut writer, &tx_buf[..tx_len]).await?;

    if !is_initiator {
        let msg = recv(&mut reader).await?;
        rx_len = noise.read_message(&msg, &mut rx_buf).map_err(map_err)?;
    }

    eprintln!("handshake complete!");
    // eprintln!("loc pk {:x?}", &local_keypair.public);
    eprintln!("rem pk {:x?}", noise.get_remote_static().unwrap());
    eprintln!("handshakehash len: {}", noise.get_handshake_hash().len());
    eprintln!("handshakehash: {:x?}", noise.get_handshake_hash());
    eprintln!("remote payload len: {}", &rx_buf[..rx_len].len());
    let _remote_nonce = decode_nonce_msg(&rx_buf[..rx_len]);

    Ok((reader.into_inner(), writer.into_inner()))
}

fn generate_nonce() -> Vec<u8> {
    let random_bytes = rand::thread_rng().gen::<[u8; 24]>();
    random_bytes.to_vec()
}

fn encode_nonce_msg(nonce: Vec<u8>) -> Vec<u8> {
    // eprintln!("nonce len {} data {:x?}", nonce.len(), &nonce);
    let nonce_msg = schema::NoisePayload { nonce };
    let mut buf = vec![0u8; 0];
    nonce_msg.encode(&mut buf).unwrap();
    buf
}

fn decode_nonce_msg(msg: &[u8]) -> Result<Vec<u8>> {
    let decoded = schema::NoisePayload::decode(msg)?;
    Ok(decoded.nonce)
}

/// Send a message with a varint prefix.
async fn send<W>(writer: &mut BufWriter<W>, buf: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    eprintln!("[send] len {}", buf.len());
    let buf_delimited = with_delimiter(buf);
    writer.write_all(&buf_delimited).await?;
    writer.flush().await?;
    Ok(())
}

/// Receive a varint-prefixed message.
pub async fn recv<'a, R>(reader: &mut BufReader<R>) -> Result<Vec<u8>>
where
    R: AsyncRead + Send + Unpin + 'static,
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

    // eprintln!("read delim, len {}", varint);

    // Read main message.
    let mut messagebuf = vec![0u8; varint as usize];
    reader.read_exact(&mut messagebuf).await?;
    eprintln!("[recv] len {}", messagebuf.len());
    Ok(messagebuf)
}

fn with_delimiter(data: &[u8]) -> Vec<u8> {
    let len = data.len();
    let varint_len = varinteger::length(len as u64);
    let mut buf = vec![0u8; len + varint_len];
    varinteger::encode(len as u64, &mut buf[..varint_len]);
    &mut buf[varint_len..].copy_from_slice(&data);
    buf
}
