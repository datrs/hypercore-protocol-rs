use async_std::net::TcpStream;
use snow;
// use futures::task::{Context, Poll};
// use bytes::{BufMut, BytesMut};
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};
use prost::Message;
use rand::Rng;
use snow::{Builder, Error as SnowError, HandshakeState};
use std::io;
use std::io::{Error, ErrorKind, Result};
use std::sync::Arc;
// use std::task::{Context, Poll};
use std::clone::Clone;
use varinteger;

use crate::schema;
use crate::CloneableStream;

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

pub async fn handshake(
    stream: TcpStream,
    is_initiator: bool,
) -> std::result::Result<(), SnowError> {
    eprintln!("[init] initiator {}", is_initiator);
    let stream = CloneableStream(Arc::new(stream));
    let mut reader = BufReader::new(stream.clone());
    let mut writer = BufWriter::new(stream.clone());

    let mut buf_tx = vec![0u8; 65535];
    let mut buf_rx = vec![0u8; 65535];
    let mut noise = build_handshake_state(is_initiator)?;

    let local_nonce = generate_nonce();
    eprintln!("local nonce: {:x?}", local_nonce);
    let nonce_msg = encode_nonce_msg(local_nonce);
    // let mut nonce_sent = false;
    // let nonce_msg = [];

    if is_initiator {
        let len = noise.write_message(&nonce_msg, &mut buf_tx).unwrap();
        eprintln!("[send] len {}", len);
        send(&mut writer, &buf_tx[..len]).await.unwrap();
    }

    let mut remote_payload_len;

    loop {
        let msg = recv(&mut reader).await.unwrap();
        eprintln!("[recv] len {}", msg.len());
        let result = noise.read_message(&msg, &mut buf_rx);
        match result {
            Ok(len) => {
                remote_payload_len = len;
                eprintln!("[recv decoded] len {}", len);
            }
            Err(e) => panic!("[error] at first noise read: {:?}", e),
        }

        if noise.is_handshake_finished() {
            break;
        }

        let result = noise.write_message(&nonce_msg, &mut buf_tx);
        match result {
            Ok(len) => {
                eprintln!("[send] len {}", len);
                send(&mut writer, &buf_tx[..len]).await.unwrap();
            }
            Err(e) => panic!("[error] at noise write: {:?}", e),
        }

        if noise.is_handshake_finished() {
            break;
        }
    }

    eprintln!("handshake finished successfully!");
    eprintln!("remote pubkey: {:x?}", noise.get_remote_static().unwrap());
    eprintln!("remote payload len: {}", remote_payload_len);
    let remote_nonce = decode_nonce_msg(&buf_rx[..remote_payload_len]).unwrap();
    eprintln!("remote nonce: {:x?}", remote_nonce);
    eprintln!("handshake hash len: {}", noise.get_handshake_hash().len());
    eprintln!("handshake hash: {:x?}", noise.get_handshake_hash());

    // The following is a basic example on how to send messages with transport
    // encryption. This will not work with a hypercore-protocol stream
    // because hypercore-protocol does not follow the NOISE spec for transport
    // encryption, it uses streaming XSalsa20 instead, where the keys are the
    // split parts from the noise handshake hash (?) and the nonces are the payloads.
    let mut noise_transport = noise.into_transport_mode().unwrap();
    let mut out_buf = vec![0u8; 200];
    if is_initiator == true {
        let msg = b"very secret";
        let len = noise_transport.write_message(msg, &mut out_buf).unwrap();
        send(&mut writer, &out_buf[..len]).await.unwrap();
        eprintln!("sent: {}", String::from_utf8_lossy(msg));
        eprintln!("sent enc len: {}", len);

        let msg = b"hello!";
        let len = noise_transport.write_message(msg, &mut out_buf).unwrap();
        send(&mut writer, &out_buf[..len]).await.unwrap();
        eprintln!("sent: {}", String::from_utf8_lossy(msg));
        eprintln!("sent enc len: {}", len);
    } else {
        let mut out_buf = vec![0u8; 200];
        let msg = recv(&mut reader).await.unwrap();
        let len = noise_transport.read_message(&msg, &mut out_buf).unwrap();
        eprintln!(
            "[read deciphered]: len {} '{}'",
            len,
            String::from_utf8_lossy(&out_buf[..len])
        );

        let msg = recv(&mut reader).await.unwrap();
        let len = noise_transport.read_message(&msg, &mut out_buf).unwrap();
        eprintln!(
            "[read deciphered]: len {} '{}'",
            len,
            String::from_utf8_lossy(&out_buf[..len])
        );
    };

    Ok(())
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
    let buf_delimited = with_delimiter(buf);
    writer.write_all(&buf_delimited).await?;
    writer.flush().await?;
    // eprintln!("send len {} final {}", buf.len(), buf_delimited.len());
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
