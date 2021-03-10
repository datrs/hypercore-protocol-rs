use crate::noise::HandshakeResult;
use salsa20::stream_cipher::{NewStreamCipher, SyncStreamCipher};
use salsa20::XSalsa20;
use std::io::{Error, ErrorKind, Result};

// TODO: Don't define here but use the values from the XSalsa20 impl.
const KEY_SIZE: usize = 32;
const NONCE_SIZE: usize = 24;

pub struct Cipher(XSalsa20);

impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cipher(XSalsa20)")
    }
}

impl Cipher {
    pub fn from_handshake_rx(handshake: &HandshakeResult) -> Result<Self> {
        let cipher = XSalsa20::new_var(
            &handshake.split_rx[..KEY_SIZE],
            &handshake.remote_nonce[..NONCE_SIZE],
        )
        .map_err(|e| {
            Error::new(
                ErrorKind::PermissionDenied,
                format!("Cannot initialize cipher: {}", e),
            )
        })?;
        Ok(Self(cipher))
    }

    pub fn from_handshake_tx(handshake: &HandshakeResult) -> Result<Self> {
        let cipher = XSalsa20::new_var(
            &handshake.split_tx[..KEY_SIZE],
            &handshake.local_nonce[..NONCE_SIZE],
        )
        .map_err(|e| {
            Error::new(
                ErrorKind::PermissionDenied,
                format!("Cannot initialize cipher: {}", e),
            )
        })?;
        Ok(Self(cipher))
    }

    pub fn apply(&mut self, buffer: &mut [u8]) {
        self.0.apply_keystream(buffer);
    }
}
