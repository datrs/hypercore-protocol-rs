# Changelog

All notable changes to this Rust implementation of hypercore-protocol will be documented here.

### unreleased

* Changed key and discovery key values to be `[u8; 32]` in place of `Vec<u8>`. To convert from a `Vec<u8>`, use `key.try_into().unwrap()` if you're sure that the key is a 32 byte long `u8` vector.
* Reworked internals to use manual poll functions and not an async function. The `Protocol` struct now directly implements `Stream`.

### 0.0.2

initial release
