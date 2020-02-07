// use async_std::io::{BufReader, BufWriter};
use blake2_rfc::blake2b::Blake2b;
use futures::io::{AsyncRead, AsyncWrite};
use prost::Message;
use rand::Rng;
use snow;
use snow::{Builder, Error as SnowError, HandshakeState, Keypair};
use std::io::{Error, ErrorKind, Result};

use crate::constants::CAP_NS_BUF;
use crate::prefixed::{read_prefixed as recv, write_prefixed as send};
use crate::schema::NoisePayload;

#[derive(Debug, Clone)]
pub struct HandshakeResult {
    pub is_initiator: bool,
    pub local_pubkey: Vec<u8>,
    pub local_seckey: Vec<u8>,
    pub remote_pubkey: Vec<u8>,
    pub local_nonce: Vec<u8>,
    pub remote_nonce: Vec<u8>,
    pub split_tx: Vec<u8>,
    pub split_rx: Vec<u8>,
}

impl HandshakeResult {
    pub fn capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        let mut context = Blake2b::with_key(32, &self.split_rx[..32]);
        context.update(CAP_NS_BUF);
        context.update(&self.split_tx[..32]);
        context.update(key);
        let hash = context.finalize();
        Some(hash.as_bytes().to_vec())
    }

    pub fn remote_capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        let mut context = Blake2b::with_key(32, &self.split_tx[..32]);
        context.update(CAP_NS_BUF);
        context.update(&self.split_rx[..32]);
        context.update(key);
        let hash = context.finalize();
        Some(hash.as_bytes().to_vec())
    }

    pub fn verify_remote_capability(&self, capability: Option<Vec<u8>>, key: &[u8]) -> Result<()> {
        let expected_capability = self.remote_capability(key);
        match (capability, expected_capability) {
            (Some(c1), Some(c2)) if c1 == c2 => Ok(()),
            (None, None) => Err(Error::new(
                ErrorKind::PermissionDenied,
                "Missing capabilities for verification",
            )),
            _ => Err(Error::new(
                ErrorKind::PermissionDenied,
                "Invalid remote channel capability",
            )),
        }
    }
}

pub fn build_handshake_state(
    is_initiator: bool,
) -> std::result::Result<(HandshakeState, Keypair), SnowError> {
    static PATTERN: &'static str = "Noise_XX_25519_XChaChaPoly_BLAKE2b";
    let builder: Builder<'_> = Builder::new(PATTERN.parse()?);
    let key_pair = builder.generate_keypair().unwrap();
    // log::trace!("hs local pubkey: {:x?}", &key_pair.public);
    let handshake_state = if is_initiator {
        builder
            .local_private_key(&key_pair.private)
            .build_initiator()
    } else {
        builder
            .local_private_key(&key_pair.private)
            .build_responder()
    };
    Ok((handshake_state?, key_pair))
}

pub async fn handshake<R, W>(
    mut reader: &mut R,
    mut writer: &mut W,
    is_initiator: bool,
) -> std::result::Result<HandshakeResult, Error>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    log::trace!(
        "handshake start, role: {}",
        if is_initiator {
            "initiator"
        } else {
            "responder"
        }
    );

    let map_err = |e| {
        Error::new(
            ErrorKind::PermissionDenied,
            format!("handshake error: {}", e),
        )
    };

    let (mut noise, local_keypair) = build_handshake_state(is_initiator).map_err(map_err)?;

    let local_nonce = generate_nonce();
    let payload = encode_nonce(local_nonce.clone());

    let mut tx_buf = vec![0u8; 512];
    let mut rx_buf = vec![0u8; 512];
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

    let remote_nonce = decode_nonce(&rx_buf[..rx_len])?;
    let remote_pubkey = noise.get_remote_static().unwrap().to_vec();

    let split = noise.split_raw();
    let split_tx;
    let split_rx;
    if is_initiator {
        split_tx = split.0;
        split_rx = split.1;
    } else {
        split_tx = split.1;
        split_rx = split.0;
    }

    log::trace!("handshake complete");
    // log::trace!("local pubkey {:x?}", &local_keypair.public);
    // log::trace!("remot pubkey {:x?}", &remote_pubkey));
    // log::trace!("split rx: {:x?}", &split_rx);
    // log::trace!("split tx: {:x?}", &split_tx);
    // log::trace!("remot nonce: {:x?}", &local_nonce);
    // log::trace!("local nonce: {:x?}", &remote_nonce));

    let result = HandshakeResult {
        is_initiator,
        local_nonce,
        remote_nonce,
        local_pubkey: local_keypair.public,
        local_seckey: local_keypair.private,
        remote_pubkey,
        split_tx,
        split_rx,
    };

    Ok(result)
}

fn generate_nonce() -> Vec<u8> {
    let random_bytes = rand::thread_rng().gen::<[u8; 24]>();
    random_bytes.to_vec()
}

fn encode_nonce(nonce: Vec<u8>) -> Vec<u8> {
    let nonce_msg = NoisePayload { nonce };
    let mut buf = vec![0u8; 0];
    nonce_msg.encode(&mut buf).unwrap();
    buf
}

fn decode_nonce(msg: &[u8]) -> Result<Vec<u8>> {
    let decoded = NoisePayload::decode(msg)?;
    Ok(decoded.nonce)
}
