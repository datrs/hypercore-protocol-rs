use blake2_rfc::blake2b::Blake2b;

use crate::constants::DISCOVERY_NS_BUF;

pub fn discovery_key(key: &[u8]) -> Vec<u8> {
    let mut hasher = Blake2b::with_key(32, key);
    hasher.update(&DISCOVERY_NS_BUF);
    hasher.finalize().as_bytes().to_vec()
}
