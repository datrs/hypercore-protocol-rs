use super::HandshakeResult;
use crate::util::{stat_uint24_le, write_uint24_le, UINT_24_LENGTH};
use blake2_rfc::blake2b::Blake2b;
use crypto_secretstream::{Header, Key, PullStream, PushStream, Tag};
use rand::rngs::OsRng;
use std::convert::TryInto;
use std::io;

const STREAM_ID_LENGTH: usize = 32;
const KEY_LENGTH: usize = 32;
const HEADER_MSG_LEN: usize = UINT_24_LENGTH + STREAM_ID_LENGTH + Header::BYTES;

pub struct DecryptCipher {
    pull_stream: PullStream,
}

pub struct EncryptCipher {
    push_stream: PushStream,
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
    pub fn from_handshake_rx_and_init_msg(
        handshake_result: &HandshakeResult,
        init_msg: &[u8],
    ) -> io::Result<Self> {
        if init_msg.len() < 32 + 24 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "Received too short init message, {} < {}.",
                    init_msg.len(),
                    32 + 24
                ),
            ));
        }

        let key: [u8; KEY_LENGTH] = handshake_result.split_rx[..KEY_LENGTH]
            .try_into()
            .expect("split_rx with incorrect length");
        let key = Key::from(key);
        let handshake_hash = handshake_result.handshake_hash.clone();
        let is_initiator = handshake_result.is_initiator;

        // Read the received message from the other peer
        let mut expected_stream_id: [u8; 32] = [0; 32];
        write_stream_id(&handshake_hash, !is_initiator, &mut expected_stream_id);
        let remote_stream_id: [u8; 32] = init_msg[0..32]
            .try_into()
            .expect("stream id slice with incorrect length");
        if expected_stream_id != remote_stream_id {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("Received stream id does not match expected",),
            ));
        }

        let header: [u8; 24] = init_msg[32..]
            .try_into()
            .expect("header slice with incorrect length");
        let pull_stream = PullStream::init(Header::from(header), &key);
        Ok(Self { pull_stream })
    }

    pub fn decrypt(
        &mut self,
        buf: &mut [u8],
        header_len: usize,
        body_len: usize,
    ) -> io::Result<usize> {
        let (to_decrypt, _tag) =
            self.decrypt_buf(&buf[header_len..header_len + body_len as usize])?;
        let decrypted_len = to_decrypt.len();
        write_uint24_le(decrypted_len, buf);
        let decrypted_end = 3 + to_decrypt.len();
        buf[3..decrypted_end].copy_from_slice(to_decrypt.as_slice());
        // Set extra bytes in the buffer to 0
        let encrypted_end = header_len + body_len as usize;
        buf[decrypted_end..encrypted_end].fill(0x00);
        Ok(decrypted_end)
    }

    pub fn decrypt_buf(&mut self, buf: &[u8]) -> io::Result<(Vec<u8>, Tag)> {
        let mut to_decrypt = buf.to_vec();
        let tag = &self.pull_stream.pull(&mut to_decrypt, &[]).map_err(|err| {
            io::Error::new(io::ErrorKind::Other, format!("Decrypt failed, err {}", err))
        })?;
        Ok((to_decrypt, *tag))
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
        Ok((Self { push_stream }, msg))
    }

    /// Get the length needed for encryption, that includes padding.
    pub fn safe_encrypted_len(&self, plaintext_len: usize) -> usize {
        // ChaCha20-Poly1305 uses padding in two places, use two 15 bytes as a safe
        // extra room.
        // https://mailarchive.ietf.org/arch/msg/cfrg/u734TEOSDDWyQgE0pmhxjdncwvw/
        plaintext_len + 2 * 15
    }

    /// Encrypts message in the given buffer to the same buffer, returns number of bytes
    /// of total message.
    pub fn encrypt(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let stat = stat_uint24_le(buf);
        if let Some((header_len, body_len)) = stat {
            let mut to_encrypt = buf[header_len..header_len + body_len as usize].to_vec();
            self.push_stream
                .push(&mut to_encrypt, &[], Tag::Message)
                .map_err(|err| {
                    io::Error::new(io::ErrorKind::Other, format!("Encrypt failed, err {}", err))
                })?;
            let encrypted_len = to_encrypt.len();
            write_uint24_le(encrypted_len, buf);
            buf[header_len..header_len + encrypted_len].copy_from_slice(to_encrypt.as_slice());
            Ok(3 + encrypted_len)
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Could not encrypt invalid data, len: {}", buf.len()),
            ))
        }
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
