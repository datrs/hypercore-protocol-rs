/// Seed for the discovery key hash
pub(crate) const DISCOVERY_NS_BUF: &[u8] = b"hypercore";

/// Default timeout (in seconds)
pub(crate) const DEFAULT_TIMEOUT: u32 = 20;

/// Default keepalive interval (in seconds)
pub(crate) const DEFAULT_KEEPALIVE: u32 = 10;

// 16,78MB is the max encrypted wire message size (will be much smaller usually).
// This limitation stems from the 24bit header.
pub(crate) const MAX_MESSAGE_SIZE: u64 = 0xFFFFFF;

/// v10: Protocol name
pub(crate) const PROTOCOL_NAME: &str = "hypercore/alpha";
