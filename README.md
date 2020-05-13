<h1 align="center">hypercore-protocol-rs</h1>

**A Rust implementation of the wire protocol of [Hypercore](https://github.com/mafintosh/hypercore-protocol)**

*Unstable and not yet maintained in any way. I started this in my spare time while playing with [datrs](https://github.com/datrs).* If someone wants to help to fill the gaps feel free to open issues or submit PRs. The best starting place is to say hi on IRC in #datrs on freenode.

This crate provides a low-level API to hypercore-protocol and exposes traits that should make it easy to implement actual protocol logic on top. This crate targets Hypercore 9 (Dat 2) only.

It uses [async-std](https://async.rs) for async IO, and [snow](https://github.com/mcginty/snow) for the Noise handshake (currently depending on unreleased changes on its master branch).

Current features are:

* Complete the Noise handshake and set up the transport encryption
* Open channels with a key
* Accept channels opened by the remote end if your end knows the key
* Create and verify capability hashes
* Send and receive all protocol messages

*Note: To sync with NodeJS the minimum required version is hypercore `9` and hypercore-protocol `8`.*

## Examples

These examples sync data between Rust and NodeJS hypercore-protocol implementations. To prepare, run
```
cd examples-nodejs
npm install
```

### [hypercore.rs](examples/hypercore.rs)

`node examples-nodejs/run.js hypercore`

Runs the `hypercore.rs` example with a replication stream from NodeJS hypercore. The `hypercore.rs` example fetches all blocks of a Node.js hypercore and inserts them into a Rust in-memory hypercore.

### [basic.rs](examples/basic.rs)

Accepts a hypercore-protocol stream and fetches all blocks of the first hypercore.

`node examples-nodejs/run.js basic`

Runs the `basic.rs` example with a replication stream from NodeJS hypercore. The `basic.rs` example fetches all blocks of a hypercore and prints them to STDOUT.

* Share a file over a hypercore on a local TCP server. Prints a hypercore key.
  `node examples-nodejs/replicate.js server 8000 ./README.md`

* Use this key to connect from Rust and pipe the file content to stdout:
  `cargo run --example basic -- server 8000 KEY`

