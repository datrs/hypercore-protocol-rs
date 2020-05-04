<h1 align="center">hypercore-protocol-rs</h1>

**A Rust implementation of the wire protocol of [Hypercore](https://github.com/mafintosh/hypercore-protocol)**

*Unstable and not yet maintained in any way. I started this in my spare time while playing with [datrs](https://github.com/datrs).* If someone wants to help to fill the gaps feel free to open issues or submit PRs. The best starting place is to say hi on IRC in #datrs on freenode.

This crate provides a low-level API to hypercore-protocol and exposes traits that should make it easy to implement actual protocol logic on top. This crate targets Hypercore 8 (Dat 2) only.

It uses [async-std](https://async.rs) for async IO, and [snow](https://github.com/mcginty/snow) for the Noise handshake (currently depending on unreleased changes on its master branch).

Current features are:

* Complete the Noise handshake (\*) and set up the transport encryption
* Open channels with a key
* Accept channels opened by the remote end if your end knows the key
* Create and verify capability hashes
* Send and receive all protocol messages

_\*: The Noise handshake is not working with the released version of Hypercore. See [this issue](https://github.com/mafintosh/hypercore-protocol/issues/51) for details._

## Examples

To get started:
```
cd examples-nodejs
npm install
```

Note that the rust impl works only against a patched version of hypercore that switches the NodeJS module [noise-protocol](https://github.com/emilbayes/noise-protocol) to a branch that changes to [Noise handshake DH calculation to the recommended standard](https://github.com/mafintosh/hypercore-protocol/issues/51). The package.json monkey-requires the patched `noise-protocol`. Other npm clients like yarn and pnpm won't make this work, but npm for me is OK with it.

### [hypercore.rs](examples/hypercore.rs)

`node examples-nodejs/run.js hypercore`

Runs the `hypercore.rs` example with a replication stream from NodeJS hypercore. The `hypercore.rs` example fetches all blocks of a Node.js hypercore and inserts them into a Rust in-memory hypercore. This currently depends on uncommitted pull requests on datrs/hypercore: [#110](https://github.com/datrs/hypercore/pull/110), [#103](https://github.com/datrs/hypercore/pull/103) (a patched datrs/hypercores dependency is included in `Cargo.toml`) and the patched Node.js hypercore (see above).

### [basic.rs](examples/basic.rs)

Accepts a hypercore-protocol stream and fetches all blocks of the first hypercore.

`node examples-nodejs/run.js basic`

Runs the `basic.rs` example with a replication stream from NodeJS hypercore. The `basic.rs` example fetches all blocks of a hypercore and prints them to STDOUT.

* Share a file over a hypercore on a local TCP server. Prints a hypercore key.
  `node examples-nodejs/replicate.js server 8000 ./README.md`

* Use this key to connect from Rust and pipe the file content to stdout:
  `cargo run --example basic -- server 8000 KEY`

