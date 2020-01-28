# hypercore-protocol in Rust

*First experiments. I started this in my spare time, might be abandoned or later become a crate and moved into [datrs](https://github.com/datrs).* If someone wants to help to fill the gaps feel free to open issues or submit PRs. The best starting place is to say hi on IRC in #datrs on freenode.

## Examples

### basic.rs

`cargo run --example basic -- server 8000`

Accepts a hypercore-protocol stream and fetches the first data block of the first hypercore. Works only *with NOISE and the capability system disabled!*

See [this gist](https://gist.github.com/Frando/e123c29160d0d995ef2149e8e96a6717) for an NodeJS example. Note that to disable NOISE, [experimental pull requests](https://github.com/mafintosh/hypercore/pull/244) to [hypercore](https://github.com/hypercore) and its dependencies are needed.

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

Running a Rust client against a NodeJS server or vice-versa **does not work**. If anyone has ideas why that is the case please help out :-)

There's also an [issue in the datrs/hypercore repo](https://github.com/datrs/hypercore/issues/92) where I documented my findings in the process up to here.


