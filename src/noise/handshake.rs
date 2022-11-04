// use async_std::io::{BufReader, BufWriter};
#[cfg(feature = "v9")]
use crate::constants::CAP_NS_BUF;
#[cfg(feature = "v10")]
use crate::noise::curve::CurveResolver;
#[cfg(feature = "v9")]
use crate::schema::NoisePayload;
#[cfg(feature = "v10")]
use crate::util::wrap_uint24_le;
use blake2_rfc::blake2b::Blake2b;
#[cfg(feature = "v9")]
use prost::Message;
#[cfg(feature = "v9")]
use rand::Rng;
#[cfg(feature = "v10")]
use snow::resolvers::{DefaultResolver, FallbackResolver};
pub use snow::Keypair;
use snow::{Builder, Error as SnowError, HandshakeState};
use std::io::{Error, ErrorKind, Result};

const CIPHERKEYLEN: usize = 32;
#[cfg(feature = "v9")]
const HANDSHAKE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2b";
#[cfg(feature = "v10")]
const HANDSHAKE_PATTERN: &str = "Noise_XX_Ed25519_ChaChaPoly_BLAKE2b";

// These the output of, see `hash_namespace` test below for how they are produced
// https://github.com/hypercore-protocol/hypercore/blob/70b271643c4e4b1e5ecae5bb579966dfe6361ff3/lib/caps.js#L9
#[cfg(feature = "v10")]
const REPLICATE_INITIATOR: [u8; 32] = [
    0x51, 0x81, 0x2A, 0x2A, 0x35, 0x9B, 0x50, 0x36, 0x95, 0x36, 0x77, 0x5D, 0xF8, 0x9E, 0x18, 0xE4,
    0x77, 0x40, 0xF3, 0xDB, 0x72, 0xAC, 0xA, 0xE7, 0xB, 0x29, 0x59, 0x4C, 0x19, 0x4D, 0xC3, 0x16,
];
#[cfg(feature = "v10")]
const REPLICATE_RESPONDER: [u8; 32] = [
    0x4, 0x38, 0x49, 0x2D, 0x2, 0x97, 0xC, 0xC1, 0x35, 0x28, 0xAC, 0x2, 0x62, 0xBC, 0xA0, 0x7,
    0x4E, 0x9, 0x26, 0x26, 0x2, 0x56, 0x86, 0x5A, 0xCC, 0xC0, 0xBF, 0x15, 0xBD, 0x79, 0x12, 0x7D,
];

#[derive(Debug, Clone, Default)]
pub struct HandshakeResult {
    pub is_initiator: bool,
    pub local_pubkey: Vec<u8>,
    pub local_seckey: Vec<u8>,
    pub remote_pubkey: Vec<u8>,
    #[cfg(feature = "v9")]
    pub local_nonce: Vec<u8>,
    #[cfg(feature = "v9")]
    pub remote_nonce: Vec<u8>,
    #[cfg(feature = "v10")]
    pub handshake_hash: Vec<u8>,
    pub split_tx: [u8; CIPHERKEYLEN],
    pub split_rx: [u8; CIPHERKEYLEN],
}

impl HandshakeResult {
    #[cfg(feature = "v9")]
    pub fn capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        let mut context = Blake2b::with_key(32, &self.split_rx[..32]);
        context.update(CAP_NS_BUF);
        context.update(&self.split_tx[..32]);
        context.update(key);
        let hash = context.finalize();
        Some(hash.as_bytes().to_vec())
    }

    #[cfg(feature = "v10")]
    pub fn capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        Some(replicate_capability(
            self.is_initiator,
            key,
            &self.handshake_hash,
        ))
    }

    #[cfg(feature = "v9")]
    pub fn remote_capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        let mut context = Blake2b::with_key(32, &self.split_tx[..32]);
        context.update(CAP_NS_BUF);
        context.update(&self.split_rx[..32]);
        context.update(key);
        let hash = context.finalize();
        Some(hash.as_bytes().to_vec())
    }

    #[cfg(feature = "v10")]
    pub fn remote_capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        Some(replicate_capability(
            !self.is_initiator,
            key,
            &self.handshake_hash,
        ))
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

#[cfg(feature = "v9")]
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

#[cfg(feature = "v10")]
pub fn build_handshake_state(
    is_initiator: bool,
) -> std::result::Result<(HandshakeState, Keypair), SnowError> {
    use snow::params::{
        BaseChoice, CipherChoice, DHChoice, HandshakeChoice, HandshakeModifierList,
        HandshakePattern, HashChoice, NoiseParams,
    };
    // NB: HANDSHAKE_PATTERN.parse() doesn't work because the pattern has "Ed25519"
    // instead of "25519".
    let noise_params = NoiseParams::new(
        HANDSHAKE_PATTERN.to_string(),
        BaseChoice::Noise,
        HandshakeChoice {
            pattern: HandshakePattern::XX,
            modifiers: HandshakeModifierList { list: vec![] },
        },
        DHChoice::Curve25519,
        CipherChoice::ChaChaPoly,
        HashChoice::Blake2b,
    );
    let builder: Builder<'_> = Builder::with_resolver(
        noise_params,
        Box::new(FallbackResolver::new(
            Box::new(CurveResolver::default()),
            Box::new(DefaultResolver::default()),
        )),
    );
    let key_pair = builder.generate_keypair().unwrap();
    let builder = builder.local_private_key(&key_pair.private);
    let handshake_state = if is_initiator {
        log::info!("building initiator");
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
    #[cfg(feature = "v9")]
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

    #[cfg(feature = "v10")]
    pub fn new(is_initiator: bool) -> Result<Self> {
        let (state, local_keypair) = build_handshake_state(is_initiator).map_err(map_err)?;

        let local_pubkey = local_keypair.public;
        let local_seckey = local_keypair.private;
        let payload = vec![];
        let result = HandshakeResult {
            is_initiator,
            local_pubkey,
            local_seckey,
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

    #[cfg(feature = "v9")]
    pub fn start(&mut self) -> Result<Option<&'_ [u8]>> {
        if self.is_initiator() {
            let tx_len = self.send()?;
            Ok(Some(&self.tx_buf[..tx_len]))
        } else {
            Ok(None)
        }
    }

    #[cfg(feature = "v10")]
    pub fn start(&mut self) -> Result<Option<Vec<u8>>> {
        if self.is_initiator() {
            let tx_len = self.send()?;
            let wrapped = wrap_uint24_le(&self.tx_buf[..tx_len].to_vec());
            Ok(Some(wrapped))
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
        let result = self
            .state
            .write_message(&self.payload, &mut self.tx_buf)
            .map_err(map_err);
        result
    }

    #[cfg(feature = "v9")]
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

    #[cfg(feature = "v10")]
    pub fn read(&mut self, msg: &[u8]) -> Result<Option<Vec<u8>>> {
        // eprintln!("hs read len {}", msg.len());
        if self.complete() {
            return Err(Error::new(ErrorKind::Other, "Handshake read after finish"));
        }

        let _rx_len = self.recv(&msg)?;

        if !self.is_initiator() && !self.did_receive {
            self.did_receive = true;
            let tx_len = self.send()?;
            let wrapped = wrap_uint24_le(&self.tx_buf[..tx_len].to_vec());
            return Ok(Some(wrapped));
        }

        let tx_buf = if self.is_initiator() {
            let tx_len = self.send()?;
            let wrapped = wrap_uint24_le(&self.tx_buf[..tx_len].to_vec());
            Some(wrapped)
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
        self.result.remote_pubkey = self
            .state
            .get_remote_static()
            .expect("Could not read remote static key after handshake")
            .to_vec();
        self.result.handshake_hash = self.state.get_handshake_hash().to_vec();
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
#[cfg(feature = "v9")]
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
#[cfg(feature = "v9")]
fn decode_nonce(msg: &[u8]) -> Result<Vec<u8>> {
    let decoded = NoisePayload::decode(msg)?;
    Ok(decoded.nonce)
}

/// Create a hash used to indicate replication capability.
/// See https://github.com/hypercore-protocol/hypercore/blob/70b271643c4e4b1e5ecae5bb579966dfe6361ff3/lib/caps.js#L11
#[cfg(feature = "v10")]
pub fn replicate_capability(is_initiator: bool, key: &[u8], handshake_hash: &[u8]) -> Vec<u8> {
    let seed = if is_initiator {
        REPLICATE_INITIATOR
    } else {
        REPLICATE_RESPONDER
    };

    let mut hasher = Blake2b::with_key(32, handshake_hash);
    hasher.update(&seed);
    hasher.update(&key);
    let hash = hasher.finalize();
    let capability = hash.as_bytes().to_vec();
    capability
}

#[cfg(test)]
mod tests {
    use super::*;
}
