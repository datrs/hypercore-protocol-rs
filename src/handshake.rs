// use async_std::io::{BufReader, BufWriter};
use blake2_rfc::blake2b::Blake2b;
use prost::Message;
use rand::Rng;
use snow;
pub use snow::Keypair;
use snow::{Builder, Error as SnowError, HandshakeState};
use std::io::{Error, ErrorKind, Result};

use crate::constants::CAP_NS_BUF;
use crate::schema::NoisePayload;

const CIPHERKEYLEN: usize = 32;

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
    static PATTERN: &str = "Noise_XX_25519_XChaChaPoly_BLAKE2b";
    let builder: Builder<'_> = Builder::new(PATTERN.parse()?);
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

        let rx_len = self.recv(&msg)?;

        if !self.is_initiator() && !self.did_receive {
            self.did_receive = true;
            let tx_len = self.send()?;
            return Ok(Some(&self.tx_buf[..tx_len]));
        }
        let mut tx_len = 0;
        if self.is_initiator() {
            tx_len = self.send()?;
        }

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

        if self.is_initiator() {
            Ok(Some(&self.tx_buf[..tx_len]))
        } else {
            Ok(None)
        }
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
fn encode_nonce(nonce: Vec<u8>) -> Vec<u8> {
    let nonce_msg = NoisePayload { nonce };
    let mut buf = vec![0u8; 0];
    nonce_msg.encode(&mut buf).unwrap();
    buf
}

#[inline]
fn decode_nonce(msg: &[u8]) -> Result<Vec<u8>> {
    let decoded = NoisePayload::decode(msg)?;
    Ok(decoded.nonce)
}
