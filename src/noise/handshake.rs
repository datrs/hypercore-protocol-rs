// use async_std::io::{BufReader, BufWriter};
use crate::constants::CAP_NS_BUF;
use crate::schema::NoisePayload;
use blake2_rfc::blake2b::Blake2b;
#[cfg(feature = "v9")]
use prost::Message;
use rand::Rng;
pub use snow::Keypair;
use snow::{Builder, Error as SnowError, HandshakeState};
use std::io::{Error, ErrorKind, Result};

const CIPHERKEYLEN: usize = 32;
const HANDSHAKE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2b";

#[derive(Debug, Clone, Default)]
pub struct HandshakeResult {
    pub is_initiator: bool,
    pub local_pubkey: Vec<u8>,
    pub local_seckey: Vec<u8>,
    pub remote_pubkey: Vec<u8>,
    pub local_nonce: Vec<u8>,
    pub remote_nonce: Vec<u8>,
    pub split_tx: [u8; CIPHERKEYLEN],
    pub split_rx: [u8; CIPHERKEYLEN],
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
    let builder: Builder<'_> = Builder::new(HANDSHAKE_PATTERN.parse()?);
    let key_pair = builder.generate_keypair().unwrap();
    let builder = builder.local_private_key(&key_pair.private);
    // log::trace!("hs local pubkey: {:x?}", &key_pair.public);
    let handshake_state = if is_initiator {
        builder.build_initiator()?
    } else {
        builder.build_responder()?
    };
    Ok((handshake_state, key_pair))
}

pub struct Handshake {
    result: HandshakeResult,
    state: HandshakeState,
    payload: Vec<u8>,
    tx_buf: Vec<u8>,
    rx_buf: Vec<u8>,
    complete: bool,
    did_receive: bool,
}

impl Handshake {
    pub fn new(is_initiator: bool) -> Result<Self> {
        let (state, local_keypair) = build_handshake_state(is_initiator).map_err(map_err)?;

        let local_nonce = generate_nonce();
        let payload = encode_nonce(local_nonce.clone());

        let result = HandshakeResult {
            is_initiator,
            local_pubkey: local_keypair.public,
            local_seckey: local_keypair.private,
            // local_keypair,
            local_nonce,
            ..Default::default()
        };
        Ok(Self {
            state,
            result,
            payload,
            tx_buf: vec![0u8; 512],
            rx_buf: vec![0u8; 512],
            complete: false,
            did_receive: false,
        })
    }

    pub fn start(&mut self) -> Result<Option<&'_ [u8]>> {
        if self.is_initiator() {
            let tx_len = self.send()?;
            Ok(Some(&self.tx_buf[..tx_len]))
        } else {
            Ok(None)
        }
    }

    pub fn complete(&self) -> bool {
        self.complete
    }

    pub fn is_initiator(&self) -> bool {
        self.result.is_initiator
    }

    fn recv(&mut self, msg: &[u8]) -> Result<usize> {
        self.state
            .read_message(&msg, &mut self.rx_buf)
            .map_err(map_err)
    }
    fn send(&mut self) -> Result<usize> {
        self.state
            .write_message(&self.payload, &mut self.tx_buf)
            .map_err(map_err)
    }

    pub fn read(&mut self, msg: &[u8]) -> Result<Option<&'_ [u8]>> {
        // eprintln!("hs read len {}", msg.len());
        if self.complete() {
            return Err(Error::new(ErrorKind::Other, "Handshake read after finish"));
        }

        // eprintln!(
        //     "[{}] HANDSHAKE recv len {} {:?}",
        //     self.is_initiator(),
        //     msg.len(),
        //     msg
        // );
        let rx_len = self.recv(&msg)?;
        // eprintln!("[{}] HANDSHAKE recv post", self.is_initiator());

        if !self.is_initiator() && !self.did_receive {
            self.did_receive = true;
            let tx_len = self.send()?;
            return Ok(Some(&self.tx_buf[..tx_len]));
        }

        let tx_buf = if self.is_initiator() {
            let tx_len = self.send()?;
            Some(&self.tx_buf[..tx_len])
        } else {
            None
        };

        let split = self.state.dangerously_get_raw_split();
        if self.is_initiator() {
            self.result.split_tx = split.0;
            self.result.split_rx = split.1;
        } else {
            self.result.split_tx = split.1;
            self.result.split_rx = split.0;
        }
        self.result.remote_nonce = decode_nonce(&self.rx_buf[..rx_len])?;
        self.result.remote_pubkey = self.state.get_remote_static().unwrap().to_vec();
        self.complete = true;

        Ok(tx_buf)
    }

    pub fn into_result(self) -> Result<HandshakeResult> {
        if !self.complete() {
            Err(Error::new(ErrorKind::Other, "Handshake is not complete"))
        } else {
            Ok(self.result)
        }
    }
}

fn map_err(e: SnowError) -> Error {
    Error::new(
        ErrorKind::PermissionDenied,
        format!("handshake error: {}", e),
    )
}

#[inline]
fn generate_nonce() -> Vec<u8> {
    let random_bytes = rand::thread_rng().gen::<[u8; 24]>();
    random_bytes.to_vec()
}

#[inline]
#[cfg(feature = "v9")]
fn encode_nonce(nonce: Vec<u8>) -> Vec<u8> {
    let nonce_msg = NoisePayload { nonce };
    let mut buf = vec![0u8; 0];
    nonce_msg.encode(&mut buf).unwrap();
    buf
}

#[inline]
#[cfg(feature = "v10")]
fn encode_nonce(nonce: Vec<u8>) -> Vec<u8> {
    let mut buf: Vec<u8> = vec![0; 2];
    // Protobuf encoding, shortcut
    buf[0] = 10;
    buf[1] = nonce.len() as u8;
    buf.extend(nonce);
    buf
}

#[inline]
#[cfg(feature = "v9")]
fn decode_nonce(msg: &[u8]) -> Result<Vec<u8>> {
    let decoded = NoisePayload::decode(msg)?;
    Ok(decoded.nonce)
}

#[inline]
#[cfg(feature = "v10")]
fn decode_nonce(msg: &[u8]) -> Result<Vec<u8>> {
    // Assume this is always the shortcut protobuf
    if msg[0] != 10 && msg[1] as usize != msg.len() - 2 {
        Err(Error::new(
            ErrorKind::PermissionDenied,
            "Could not decode nonce",
        ))
    } else {
        let mut buf: Vec<u8> = vec![0; msg.len() - 2];
        buf.copy_from_slice(&msg[2..]);
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_nonce_encode_decode() {
        let nonce = generate_nonce();
        let payload = encode_nonce(nonce.clone());
        assert_eq!(payload.len(), 26);
        assert_eq!(payload[0], 10);
        assert_eq!(payload[1] as usize, payload.len() - 2);
        let nonce_ret = decode_nonce(&*payload).unwrap();
        assert_eq!(nonce_ret, nonce);
    }
}
