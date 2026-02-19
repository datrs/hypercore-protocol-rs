//! Handshake result and capability verification for hypercore replication.
//!
//! This module handles capability verification using the handshake hash from
//! the underlying encrypted connection (e.g., from hyperswarm/hyperdht).

use blake2::{
    Blake2bMac,
    digest::{FixedOutput, Update, typenum::U32},
};
use std::io::{Error, ErrorKind, Result};

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

/// Result of a Noise handshake, used for capability verification.
///
/// When using hypercore-protocol with hyperswarm, the Noise handshake is performed
/// at the transport layer (hyperdht). This struct holds the information needed
/// for capability verification when opening channels.
#[derive(Debug, Clone, Default)]
pub struct HandshakeResult {
    pub(crate) is_initiator: bool,
    /// Local public key (32 bytes)
    pub local_pubkey: Vec<u8>,
    /// Remote public key (32 bytes)
    pub remote_pubkey: Vec<u8>,
    pub(crate) handshake_hash: Vec<u8>,
}

impl HandshakeResult {
    /// Create a HandshakeResult for a pre-encrypted connection.
    ///
    /// This is used when the Noise handshake was performed at a lower layer
    /// (e.g., hyperswarm/hyperdht) and we're reusing the encrypted channel.
    /// The handshake_hash is used for capability verification.
    ///
    /// # Arguments
    /// * `is_initiator` - Whether this peer initiated the connection
    /// * `local_pubkey` - This peer's Noise public key
    /// * `remote_pubkey` - The remote peer's Noise public key
    /// * `handshake_hash` - The 64-byte handshake hash from the Noise handshake
    pub fn from_pre_encrypted(
        is_initiator: bool,
        local_pubkey: [u8; 32],
        remote_pubkey: [u8; 32],
        handshake_hash: Vec<u8>,
    ) -> Self {
        Self {
            is_initiator,
            local_pubkey: local_pubkey.to_vec(),
            remote_pubkey: remote_pubkey.to_vec(),
            handshake_hash,
        }
    }

    /// Compute capability for opening a channel with the given key.
    pub(crate) fn capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        Some(replicate_capability(
            self.is_initiator,
            key,
            &self.handshake_hash,
        ))
    }

    /// Compute expected remote capability for the given key.
    pub(crate) fn remote_capability(&self, key: &[u8]) -> Option<Vec<u8>> {
        Some(replicate_capability(
            !self.is_initiator,
            key,
            &self.handshake_hash,
        ))
    }

    /// Verify a remote peer's capability for opening a channel.
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

/// Create a hash used to indicate replication capability.
/// See JavaScript [here](https://github.com/hypercore-protocol/hypercore/blob/70b271643c4e4b1e5ecae5bb579966dfe6361ff3/lib/caps.js#L11).
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

    hash.as_slice().to_vec()
}
