use blake2_rfc::blake2b::Blake2b;
// use pretty_hash::fmt as pretty_hash;

use crate::constants::DISCOVERY_NS_BUF;

pub fn discovery_key(key: &[u8]) -> Vec<u8> {
    let mut hasher = Blake2b::with_key(32, key);
    hasher.update(&DISCOVERY_NS_BUF);
    hasher.finalize().as_bytes().to_vec()
}

pub fn pretty_hash(key: &[u8]) -> String {
    pretty_hash::fmt(key).unwrap_or("<invalid>".into())
}
