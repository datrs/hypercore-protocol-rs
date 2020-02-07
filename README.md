<h1 align="center">hypercore-protocol-rs</h1>

**A Rust implementation of the wire protocol of [Hypercore](https://github.com/mafintosh/hypercore-protocol)**

*Unstable and not yet maintained in any way. I started this in my spare time while playing with [datrs](https://github.com/datrs).* If someone wants to help to fill the gaps feel free to open issues or submit PRs. The best starting place is to say hi on IRC in #datrs on freenode.

This crate provides a low-level API to hypercore-protocol and exposes traits that should make it easy to implement actual protocol logic on top. This crate targets Hypercore 8 (Dat 2) only.

It uses [async-std](https://async.rs) for async IO, and [snow](https://github.com/mcginty/snow) for the Noise handshake (currently with two patches: [(1)](https://github.com/mcginty/snow/pull/73), [(2)](https://github.com/mcginty/snow/pull/78)).

Current features are:

* Complete the Noise handshake (\*) and set up the transport encryption
* Open channels with a key
* Accept channels opened by the remote end if your end knows the key
* Create and verify capability hashes
* Send and receive all protocol messages

_\*: The Noise handshake is not working with the released version of Hypercore. See [this issue](https://github.com/mafintosh/hypercore-protocol/issues/51) for details._

## Examples

### [basic.rs](examples/basic.rs)

Accepts a hypercore-protocol stream and fetches the first data block of the first hypercore. Also my current test case, read the code in there if you want to see how you'd work with hypercore-protocol using this crate.

* Share a file over a hypercore on a local TCP server. Prints a hypercore key.
  `node examples-nodejs/replicate.js server 8000 ./README.md`

* Use this key to connect from Rust and pipe the file content to stdout:
  `cargo run --example basic -- server 8000 KEY`

You can swap client and server between the two commands.

Note that the rust impl works only against a patched version of hypercore that switches the NodeJS module [noise-protocol](https://github.com/emilbayes/noise-protocol) to a branch that changes to [Noise handshake DH calculation to the recommended standard](https://github.com/mafintosh/hypercore-protocol/issues/51).

### noise.rs

```
cargo run --example noise -- server 8000
cargo run --example noise -- client 8000
```

This performs a NOISE handshake in the `Noise_XX_25519_XChaChaPoly_BLAKE2` mode and prints debug output. It works between a client and server as above (each command from a different terminal).

This crates has a dependency on a patched version of the [snow](https://docs.rs/snow/0.5.2/snow/) crate that [adds support for the XChaCha20 cipher](https://github.com/mcginty/snow/pull/73) used by hypercore-protocol.

I added a basic NodeJS hypercore-protocol stream in [examples-nodejs/handshake.js](examples-nodejs/basic-protocol). First, run `npm install` in `./examples-nodejs`. Then, `handshake.js` takes the same arguments as the Rust example above:

```
node examples-nodejs/handshake.js server 8000
node examples-nodejs/handshake.js client 8000
```

Running both from different terminals should print debug output and complete the handshake.

There's also an [issue in the datrs/hypercore repo](https://github.com/datrs/hypercore/issues/92) where I documented my findings in the process up to here.


