use super::curve::CurveResolver;
use crate::util::wrap_uint24_le;
use blake2::{
    digest::{typenum::U32, FixedOutput, Update},
    Blake2bMac,
};
use snow::resolvers::{DefaultResolver, FallbackResolver};
use snow::{Builder, Error as SnowError, HandshakeState};
use std::io::{Error, ErrorKind, Result};

const CIPHERKEYLEN: usize = 32;
const HANDSHAKE_PATTERN: &str = "Noise_XX_Ed25519_ChaChaPoly_BLAKE2b";

// These the output of, see `hash_namespace` test below for how they are produced
// https://github.com/hypercore-protocol/hypercore/blob/70b271643c4e4b1e5ecae5bb579966dfe6361ff3/lib/caps.js#L9
const REPLICATE_INITIATOR: [u8; 32] = [
    0x51, 0x81, 0x2A, 0x2A, 0x35, 0x9B, 0x50, 0x36, 0x95, 0x36, 0x77, 0x5D, 0xF8, 0x9E, 0x18, 0xE4,
    0x77, 0x40, 0xF3, 0xDB, 0x72, 0xAC, 0xA, 0xE7, 0xB, 0x29, 0x59, 0x4C, 0x19, 0x4D, 0xC3, 0x16,
];
const REPLICATE_RESPONDER: [u8; 32] = [
    0x4, 0x38, 0x49, 0x2D, 0x2, 0x97, 0xC, 0xC1, 0x35, 0x28, 0xAC, 0x2, 0x62, 0xBC, 0xA0, 0x7,
    0x4E, 0x9, 0x26, 0x26, 0x2, 0x56, 0x86, 0x5A, 0xCC, 0xC0, 0xBF, 0x15, 0xBD, 0x79, 0x12, 0x7D,
];

#[derive(Debug, Clone, Default)]
pub(crate) struct HandshakeResult {
    pub(crate) is_initiator: bool,
    pub(crate) local_pubkey: Vec<u8>,
    pub(crate) remote_pubkey: Vec<u8>,
    pub(crate) handshake_hash: Vec<u8>,
    pub(crate) split_tx: [u8; CIPHERKEYLEN],
    pub(crate) split_rx: [u8; CIPHERKEYLEN],
}

impl HandshakeResult {
    pub(crate) fn capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        Some(replicate_capability(
            self.is_initiator,
            key,
            &self.handshake_hash,
        ))
    }

    pub(crate) fn remote_capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        Some(replicate_capability(
            !self.is_initiator,
            key,
            &self.handshake_hash,
        ))
    }

    pub(crate) fn verify_remote_capability(
        &self,
        capability: Option<Vec<u8>>,
        key: &[u8],
    ) -> Result<()> {
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

pub(crate) struct Handshake {
    result: HandshakeResult,
    state: HandshakeState,
    payload: Vec<u8>,
    tx_buf: Vec<u8>,
    rx_buf: Vec<u8>,
    complete: bool,
    did_receive: bool,
}

impl Handshake {
    pub(crate) fn new(is_initiator: bool) -> Result<Self> {
        let (state, local_pubkey) = build_handshake_state(is_initiator).map_err(map_err)?;

        let payload = vec![];
        let result = HandshakeResult {
            is_initiator,
            local_pubkey,
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

    pub(crate) fn start(&mut self) -> Result<Option<Vec<u8>>> {
        if self.is_initiator() {
            let tx_len = self.send()?;
            let wrapped = wrap_uint24_le(&self.tx_buf[..tx_len].to_vec());
            Ok(Some(wrapped))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn complete(&self) -> bool {
        self.complete
    }

    pub(crate) fn is_initiator(&self) -> bool {
        self.result.is_initiator
    }

    fn recv(&mut self, msg: &[u8]) -> Result<usize> {
        self.state
            .read_message(msg, &mut self.rx_buf)
            .map_err(map_err)
    }
    fn send(&mut self) -> Result<usize> {
        self.state
            .write_message(&self.payload, &mut self.tx_buf)
            .map_err(map_err)
    }

    pub(crate) fn read(&mut self, msg: &[u8]) -> Result<Option<Vec<u8>>> {
        // eprintln!("hs read len {}", msg.len());
        if self.complete() {
            return Err(Error::new(ErrorKind::Other, "Handshake read after finish"));
        }

        let _rx_len = self.recv(msg)?;

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

    pub(crate) fn into_result(self) -> Result<HandshakeResult> {
        if !self.complete() {
            Err(Error::new(ErrorKind::Other, "Handshake is not complete"))
        } else {
            Ok(self.result)
        }
    }
}

fn build_handshake_state(
    is_initiator: bool,
) -> std::result::Result<(HandshakeState, Vec<u8>), SnowError> {
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
            Box::<CurveResolver>::default(),
            Box::<DefaultResolver>::default(),
        )),
    );
    let key_pair = builder.generate_keypair().unwrap();
    let builder = builder.local_private_key(&key_pair.private);
    let handshake_state = if is_initiator {
        tracing::debug!("building initiator");
        builder.build_initiator()?
    } else {
        tracing::debug!("building responder");
        builder.build_responder()?
    };
    Ok((handshake_state, key_pair.public))
}

fn map_err(e: SnowError) -> Error {
    Error::new(ErrorKind::PermissionDenied, format!("Handshake error: {e}"))
}

/// Create a hash used to indicate replication capability.
/// See https://github.com/hypercore-protocol/hypercore/blob/70b271643c4e4b1e5ecae5bb579966dfe6361ff3/lib/caps.js#L11
fn replicate_capability(is_initiator: bool, key: &[u8], handshake_hash: &[u8]) -> Vec<u8> {
    let seed = if is_initiator {
        REPLICATE_INITIATOR
    } else {
        REPLICATE_RESPONDER
    };

    let mut hasher =
        Blake2bMac::<U32>::new_with_salt_and_personal(handshake_hash, &[], &[]).unwrap();
    hasher.update(&seed);
    hasher.update(key);
    let hash = hasher.finalize_fixed();
    let capability = hash.as_slice().to_vec();
    capability
}
