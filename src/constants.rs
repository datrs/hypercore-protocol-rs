/// Seed for the discovery key hash
pub const DISCOVERY_NS_BUF: &[u8] = b"hypercore";

/// Default timeout (in seconds)
pub const DEFAULT_TIMEOUT: u32 = 20;

/// Default keepalive interval (in seconds)
pub const DEFAULT_KEEPALIVE: u32 = 10;

// 16,78MB is the max encrypted wire message size (will be much smaller usually).
// This limitation stems from the 24bit header.
pub const MAX_MESSAGE_SIZE: u64 = 0xFFFFFF;

/// v10: Protocol name
pub const PROTOCOL_NAME: &str = "hypercore/alpha";
