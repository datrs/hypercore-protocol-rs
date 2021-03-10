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
