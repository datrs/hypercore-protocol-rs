use crate::message::EncodeError;
use crate::noise::HandshakeResult;
use crate::util::{stat_uint24_le, write_uint24_le, UINT_24_LENGTH};
use blake2_rfc::blake2b::Blake2b;
use crypto_secretstream::{Header, Key, PullStream, PushStream, Tag};
use rand::rngs::OsRng;
use std::convert::TryInto;

const STREAM_ID_LENGTH: usize = 32;
const KEY_LENGTH: usize = 32;
const HEADER_MSG_LEN: usize = UINT_24_LENGTH + STREAM_ID_LENGTH + Header::BYTES;

pub struct DecryptCipher {
    is_initiator: bool,
    handshake_hash: Vec<u8>,
    pull_stream_bridge_key: Option<Key>,
    pull_stream: Option<PullStream>,
}

pub struct EncryptCipher {
    is_initiator: bool,
    handshake_hash: Vec<u8>,
    push_stream: PushStream,
    header_message: Option<[u8; HEADER_MSG_LEN]>,
}

impl std::fmt::Debug for DecryptCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DecryptCipher(crypto_secretstream)")
    }
}

impl std::fmt::Debug for EncryptCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EncryptCipher(crypto_secretstream)")
    }
}

impl DecryptCipher {
    pub fn from_handshake_rx(handshake_result: &HandshakeResult) -> std::io::Result<Self> {
        let key: [u8; KEY_LENGTH] = handshake_result.split_rx[..KEY_LENGTH]
            .try_into()
            .expect("split_rx with incorrect length");
        let key = Key::from(key);
        Ok(Self {
            is_initiator: handshake_result.is_initiator,
            handshake_hash: handshake_result.handshake_hash.clone(),
            pull_stream_bridge_key: Some(key),
            pull_stream: None,
        })
    }

    pub fn decrypt(&mut self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let mut decrypt_end = buf.len();
        let new_pull_stream = match self.pull_stream.as_mut() {
            Some(pull_stream) => {
                let stat = stat_uint24_le(buf);
                if let Some((header_len, body_len)) = stat {
                    let mut to_decrypt = buf[header_len..header_len + body_len as usize].to_vec();
                    let tag = pull_stream
                        .pull(&mut to_decrypt, &[])
                        .map_err(|err| println!("pull_stream err, {}", err.to_string()))
                        .unwrap();
                    let decryptd_len = to_decrypt.len();
                    println!(
                        "DecryptCipher::decrypt: {} bytes decrypted into tag {:?} and message({}) {:02X?}",
                        buf.len(),
                        tag, decryptd_len, to_decrypt
                    );
                    write_uint24_le(decryptd_len, buf);
                    decrypt_end = 3 + to_decrypt.len();
                    buf[3..decrypt_end].copy_from_slice(to_decrypt.as_slice());
                } else {
                    return Err(EncodeError::new(buf.len()));
                };
                None
            }
            None => {
                let stat = stat_uint24_le(buf);
                let pull_stream = if let Some((header_len, body_len)) = stat {
                    let mut expected_stream_id: [u8; 32] = [0; 32];
                    write_stream_id(
                        &self.handshake_hash,
                        !&self.is_initiator,
                        &mut expected_stream_id,
                    );
                    let remote_stream_id: [u8; 32] = buf[header_len..header_len + 32]
                        .try_into()
                        .expect("stream id slice with incorrect length");
                    if expected_stream_id != remote_stream_id {
                        return Err(EncodeError::new(buf.len()));
                    }

                    let header: [u8; 24] = buf[header_len + 32..header_len + body_len as usize]
                        .try_into()
                        .expect("header slice with incorrect length");
                    PullStream::init(
                        Header::from(header),
                        &self.pull_stream_bridge_key.as_ref().unwrap(),
                    )
                } else {
                    return Err(EncodeError::new(buf.len()));
                };

                Some(pull_stream)
            }
        };
        if let Some(new_pull_stream) = new_pull_stream {
            self.pull_stream = Some(new_pull_stream);
            self.pull_stream_bridge_key = None;
        }
        Ok(decrypt_end)
    }
}

impl EncryptCipher {
    pub fn from_handshake_tx(
        handshake_result: &HandshakeResult,
    ) -> std::io::Result<(Self, Vec<u8>)> {
        let key: [u8; KEY_LENGTH] = handshake_result.split_tx[..KEY_LENGTH]
            .try_into()
            .expect("split_tx with incorrect length");
        let key = Key::from(key);

        let mut header_message: [u8; HEADER_MSG_LEN] = [0; HEADER_MSG_LEN];
        write_uint24_le(STREAM_ID_LENGTH + Header::BYTES, &mut header_message);
        write_stream_id(
            &handshake_result.handshake_hash,
            handshake_result.is_initiator,
            &mut header_message[UINT_24_LENGTH..UINT_24_LENGTH + STREAM_ID_LENGTH],
        );

        let (header, push_stream) = PushStream::init(&mut OsRng, &key);
        let header = header.as_ref();
        header_message[UINT_24_LENGTH + STREAM_ID_LENGTH..].copy_from_slice(header);
        let msg = header_message.to_vec();
        Ok((
            Self {
                is_initiator: handshake_result.is_initiator,
                handshake_hash: handshake_result.handshake_hash.clone(),
                push_stream,
                header_message: Some(header_message),
            },
            msg,
        ))
    }

    /// Get the length needed for encryption, that includes padding.
    pub fn safe_encrypted_len(&self, plaintext_len: usize) -> usize {
        if self.header_message.is_some() {
            plaintext_len
        } else {
            // ChaCha20-Poly1305 uses padding in two places, use two 15 bytes as a safe
            // extra room.
            // https://mailarchive.ietf.org/arch/msg/cfrg/u734TEOSDDWyQgE0pmhxjdncwvw/
            plaintext_len + 2 * 15
        }
    }

    /// Encrypts message in the given buffer to the same buffer, returns number of bytes
    /// of total message.
    pub fn encrypt(&mut self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let is_header_message: bool = if let Some(header_message) = self.header_message {
            if header_message == buf {
                true
            } else {
                return Err(EncodeError::new(buf.len()));
            }
        } else {
            false
        };
        let len = if is_header_message {
            self.header_message = None;
            buf.len()
        } else {
            let stat = stat_uint24_le(buf);
            println!("EncryptCipher::encrypt: encrypting");
            if let Some((header_len, body_len)) = stat {
                let mut to_encrypt = buf[header_len..header_len + body_len as usize].to_vec();
                self.push_stream
                    .push(&mut to_encrypt, &[], Tag::Message)
                    .map_err(|err| println!("push_stream err, {}", err.to_string()))
                    .unwrap();
                let encrypted_len = to_encrypt.len();
                println!(
                    "EncryptCipher::encrypt: buf={}, header_len={}, body_len={}, encrypted={} => {:02X?}",
                    buf.len(),
                    header_len,
                    body_len,
                    encrypted_len,
                    to_encrypt
                );
                write_uint24_le(encrypted_len, buf);
                buf[3..3 + encrypted_len].copy_from_slice(to_encrypt.as_slice());
                3 + encrypted_len
            } else {
                return Err(EncodeError::new(buf.len()));
            }
        };
        Ok(len)
    }
}

// NB: These values come from Javascript-side
//
// const [NS_INITIATOR, NS_RESPONDER] = crypto.namespace('hyperswarm/secret-stream', 2)
//
// at https://github.com/hyperswarm/secret-stream/blob/master/index.js
const NS_INITIATOR: [u8; 32] = [
    0xa9, 0x31, 0xa0, 0x15, 0x5b, 0x5c, 0x09, 0xe6, 0xd2, 0x86, 0x28, 0x23, 0x6a, 0xf8, 0x3c, 0x4b,
    0x8a, 0x6a, 0xf9, 0xaf, 0x60, 0x98, 0x6e, 0xde, 0xed, 0xe9, 0xdc, 0x5d, 0x63, 0x19, 0x2b, 0xf7,
];
const NS_RESPONDER: [u8; 32] = [
    0x74, 0x2c, 0x9d, 0x83, 0x3d, 0x43, 0x0a, 0xf4, 0xc4, 0x8a, 0x87, 0x05, 0xe9, 0x16, 0x31, 0xee,
    0xcf, 0x29, 0x54, 0x42, 0xbb, 0xca, 0x18, 0x99, 0x6e, 0x59, 0x70, 0x97, 0x72, 0x3b, 0x10, 0x61,
];

fn write_stream_id(handshake_hash: &[u8], is_initiator: bool, out: &mut [u8]) {
    let mut hasher = Blake2b::with_key(32, handshake_hash);
    if is_initiator {
        hasher.update(&NS_INITIATOR);
    } else {
        hasher.update(&NS_RESPONDER);
    }
    let result = hasher.finalize();
    let result = result.as_bytes();
    out.copy_from_slice(result);
}
