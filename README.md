# Hypercore Protocol
[![crates.io version][1]][2] [![build status][3]][4]
[![downloads][5]][6] [![docs.rs docs][7]][8]

Hypercore protocol is a streaming, message based protocol. This is a Rust port of
the wire protocol implementation in
[the original Javascript version](https://github.com/holepunchto/hypercore). This
crate targets the Hypercore LTS version 10.

This crate provides a low-level streaming API to hypercore-protocol and exposes an
interface that should make it easy to implement actual protocol logic on top.

This crate uses either [async-std](https://async.rs) or [tokio](https://tokio.rs/)
for async IO, [snow](https://github.com/mcginty/snow) for the Noise handshake and
[RustCrypto's crypto_secretsteram](https://github.com/RustCrypto/nacl-compat/tree/master/crypto_secretstream)
for encryption.

## Features

- [x] Complete the Noise handshake
- [x] Establish libsodium's `crypto_secretstream`.
- [x] Open channels with a key
- [x] Accept channels opened by the remote end if your end knows the key
- [x] Create and verify capability hashes
- [x] Send and receive all protocol messages
- [x] Support `async-std` or `tokio` runtimes
- [x] Support WASM
- [x] Test Javascript interoperability
- [ ] Support the new [manifest](https://github.com/holepunchto/hypercore/blob/main/lib/manifest.js) in the wire protocol to remain compatible with upcoming v11
- [ ] Finalize documentation and release v1.0.0

## Installation

```bash
cargo add hypercore-protocol
```

## Examples

These examples sync data between Rust and NodeJS hypercore-protocol implementations. To prepare, run
```bash
cd examples-nodejs && npm install && cd ..
```

### [replication.rs](examples/replication.rs)

Runs the `replication.rs` example by replicating a hypercore between Rust and Node hypercores and printing the result.

* Node Server / Rust Client

```bash
node examples-nodejs/run.js nodeServer
```

* Rust Server / Node Client

```bash
node examples-nodejs/run.js rustServer
```

* Rust Server / Rust Client

```bash
node examples-nodejs/run.js rust
```

* Node Server / Node Client

```bash
node examples-nodejs/run.js node
```

## Development

To test interoperability with Javascript, enable the `js_tests` feature:

```bash
cargo test --features js_tests
```

Run benches with:

```bash
cargo bench
```

## Contributing

We're actively looking for contributors to the datrust development! If you're interested, the
easiest is to say hi in the `#rust` channel on the
[Hypercore Protocol Discord](https://chat.hypercore-protocol.org/).

Want to help with Hypercore Protocol? Check out our
["Contributing" guide](https://github.com/datrs/hypercore-protocol-rs/blob/master/.github/CONTRIBUTING.md)
and take a look at the open [issues](https://github.com/datrs/hypercore-protocol-rs/issues).

## License

[MIT](./LICENSE-MIT) OR [Apache-2.0](./LICENSE-APACHE)

[1]: https://img.shields.io/crates/v/hypercore-protocol.svg?style=flat-square
[2]: https://crates.io/crates/hypercore-protocol
[3]: https://github.com/datrs/hypercore-protocol-rs/actions/workflows/ci.yml/badge.svg
[4]: https://github.com/datrs/hypercore-protocol-rs/actions
[5]: https://img.shields.io/crates/d/hypercore-protocol.svg?style=flat-square
[6]: https://crates.io/crates/hypercore-protocol
[7]: https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square
[8]: https://docs.rs/hypercore-protocol
