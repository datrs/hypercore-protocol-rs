use blake2_rfc::blake2b::Blake2b;
use std::convert::TryInto;
use std::io::{Error, ErrorKind};

use crate::constants::DISCOVERY_NS_BUF;
use crate::DiscoveryKey;

pub fn discovery_key(key: &[u8]) -> DiscoveryKey {
    let mut hasher = Blake2b::with_key(32, key);
    hasher.update(&DISCOVERY_NS_BUF);
    hasher.finalize().as_bytes().try_into().unwrap()
}

pub fn pretty_hash(key: &[u8]) -> String {
    pretty_hash::fmt(key).unwrap_or_else(|_| "<invalid>".into())
}

pub fn map_channel_err<T>(err: async_channel::SendError<T>) -> Error {
    Error::new(
        ErrorKind::BrokenPipe,
        format!("Cannot forward on channel: {}", err),
    )
}

pub fn length_prefix(buf: &[u8]) -> Vec<u8> {
    let len = buf.len();
    let prefix_len = varinteger::length(len as u64);
    let mut prefix_buf = vec![0u8; prefix_len];
    varinteger::encode(len as u64, &mut prefix_buf[..prefix_len]);
    prefix_buf
}
