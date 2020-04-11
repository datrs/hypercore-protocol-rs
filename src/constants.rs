/// Seed for the capability hash
pub const CAP_NS_BUF: &[u8] = b"hypercore capability";

/// Seed for the discovery key hash
pub const DISCOVERY_NS_BUF: &[u8] = b"hypercore";

/// Default timeout (in seconds)
pub const DEFAULT_TIMEOUT: u32 = 20;

/// Default keepalive interval (in seconds)
pub const DEFAULT_KEEPALIVE: u32 = 10;

// 4MB is the max wire message size (will be much smaller usually).
pub const MAX_MESSAGE_SIZE: u64 = 1024 * 1024 * 4;
