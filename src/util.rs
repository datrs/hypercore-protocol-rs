use blake2_rfc::blake2b::Blake2b;
use std::convert::TryInto;
use std::io::{Error, ErrorKind};

use crate::constants::DISCOVERY_NS_BUF;
use crate::DiscoveryKey;

/// Calculate the discovery key of a key.
///
/// The discovery key is a 32 byte namespaced hash of the key.
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

#[cfg(feature = "v10")]
pub const UINT_24_LENGTH: usize = 3;

#[inline]
#[cfg(feature = "v10")]
pub fn wrap_uint24_le(data: &Vec<u8>) -> Vec<u8> {
    let mut buf: Vec<u8> = vec![0; 3];
    let n = data.len();
    write_uint24_le(n, &mut buf);
    buf.extend(data);
    buf
}

#[inline]
#[cfg(feature = "v10")]
pub fn write_uint24_le(n: usize, buf: &mut [u8]) {
    buf[0] = (n & 255) as u8;
    buf[1] = ((n >> 8) & 255) as u8;
    buf[2] = ((n >> 16) & 255) as u8;
}

#[inline]
#[cfg(feature = "v10")]
pub fn stat_uint24_le(buffer: &[u8]) -> Option<(usize, u64)> {
    // FIXME: when to return None!
    let len =
        (((buffer[0] as u32) << 0) | ((buffer[1] as u32) << 8) | ((buffer[2] as u32) << 16)) as u64;
    Some((UINT_24_LENGTH, len))
}
