# Changelog

All notable changes to this Rust implementation of hypercore-protocol will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Added

### Changed

### Removed



## [0.7.2] - 2026-08-03

### Added

### Changed

* Fix `Open`/`Close` messages corrupting multi-message batches.

### Removed



## [0.7.1] - 2026-06-04

* Rewrite serveral async methods to return owned futures and fix a busy loop ([PR 148](https://github.com/datrs/hypercore-protocol-rs/pull/24)).

## [0.7.0] - 2026-02-19

BIG CHANGES:
* Encryption and framing of streams has been moved out of this crate into `hypercore_handshake` and `uint24le_framing` respectively. This had big impacts on the public API. Now `Protocol::new` just takes a `impl CipherTrait` argument.
* Remove dependence on `hypercore` instead we use `hypercore_schema` (so hypercore related features have been removed).
* Bumped to edition 2024.
* Dropped support for async-std (and its feature flag)

## [0.6.1] - 2024-10-28

* Implement Clone on Channel, remove unnecessary mut requirements, and improve Protocol debugging.

## [0.6.0] - 2024-10-25

#### API breaking changes

Support hypercore v0.14.0 and change tokio as the default async runtime.

## [0.5.0] - 2024-07-05

#### API breaking changes

Support hypercore v0.13.0 which [removed generic parameters from Hypercore and Storage][https://github.com/datrs/hypercore/pull/139].

## [0.4.1] - 2023-10-26

* Implement close() and signal_local() in CommandTx

## [0.4.0] - 2023-10-12

#### API breaking changes

* Support for [Hypercore LTS v10](https://github.com/holepunchto/hypercore) which is backward incompatible with v9 and thus v0.3.0 of this crate. This resulted in many changes throughout the API.

## 0.3.0

#### API breaking changes

* Changed the generic argument of the Protocol struct to be a single IO handle (AsyncRead + AsyncWrite) in place of a R: AsyncRead and W: AsyncWrite. This is a change in the public API. If you have seperate reader and writer structs, a Duplex handle is provided that combines the reader and writer. If you have a struct that is both AsyncRead and AsyncWrite, the Protocol is now generic just over that struct.
* Reworked internals to use manual poll functions and not an async function. The `Protocol` struct now directly implements `Stream`, the `ProtocolStream` wrapper is removed.
* Made the `Event` enum non-exhaustive.
* Removed the `destroy` method, errors are emitted on the protocol stream.
* Changed key and discovery key values to be `[u8; 32]` in place of `Vec<u8>`
  > . To convert from a `Vec<u8>`, use `key.try_into().unwrap()` if you're sure that the key is a 32 byte long `u8` vector.

## 0.0.2

initial release

<!-- next-url -->
[Unreleased]: https://github.com/datrs/hypercore-protocol-rs/compare/v0.7.2...HEAD
[0.7.2]: https://github.com/datrs/hypercore-protocol-rs/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/datrs/hypercore-protocol-rs/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/datrs/hypercore-protocol-rs/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/datrs/hypercore-protocol-rs/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/datrs/hypercore-protocol-rs/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/datrs/hypercore-protocol-rs/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/datrs/hypercore-protocol-rs/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/datrs/hypercore-protocol-rs/releases/tag/v0.4.0
