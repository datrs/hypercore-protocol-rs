use blake2::{
    Blake2bMac,
    digest::{FixedOutput, Update, typenum::U32},
};
use std::{
    convert::TryInto,
    io::{Error, ErrorKind},
};

use crate::{DiscoveryKey, constants::DISCOVERY_NS_BUF};

/// Calculate the discovery key of a key.
///
/// The discovery key is a 32 byte namespaced hash of the key.
pub fn discovery_key(key: &[u8]) -> DiscoveryKey {
    let mut hasher = Blake2bMac::<U32>::new_with_salt_and_personal(key, &[], &[]).unwrap();
    hasher.update(DISCOVERY_NS_BUF);
    hasher.finalize_fixed().as_slice().try_into().unwrap()
}

pub(crate) fn pretty_hash(key: &[u8]) -> String {
    pretty_hash::fmt(key).unwrap_or_else(|_| "<invalid>".into())
}

pub(crate) fn map_channel_err<T>(err: async_channel::SendError<T>) -> Error {
    Error::new(
        ErrorKind::BrokenPipe,
        format!("Cannot forward on channel: {err}"),
    )
}
