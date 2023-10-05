# Hypercore Protocol

<div align="center">
  <strong>
    Rust implementation of the <a href="https://github.com/mafintosh/hypercore-protocol">Hypercore</a> wire protocol
  </strong>
</div>

<br />

<div align="center">
  <!-- Crates version -->
  <a href="https://crates.io/crates/hypercore-protocol">
    <img src="https://img.shields.io/crates/v/hypercore-protocol.svg?style=flat-square"
    alt="Crates.io version" />
  </a>
  <!-- Downloads -->
  <a href="https://crates.io/crates/hypercore-protocol">
    <img src="https://img.shields.io/crates/d/hypercore-protocol.svg?style=flat-square"
      alt="Download" />
  </a>
  <!-- docs.rs docs -->
  <a href="https://docs.rs/hypercore-protocol">
    <img src="https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square"
      alt="docs.rs docs" />
  </a>
</div>

<div align="center">
  <h4>
    <a href="https://docs.rs/hypercore-protocol">
      API Docs
    </a>
    <span> | </span>
    <a href="#contributing">
      Contributing
    </a>
  </h4>
</div>

This crate provides a low-level streaming API to hypercore-protocol and exposes an interface that should make it easy to implement actual protocol logic on top. This crate targets Hypercore 9 (Dat 2) only.

It uses [async-std](https://async.rs) for async IO, and [snow](https://github.com/mcginty/snow) for the Noise handshake.

Current features are:

* Complete the Noise handshake and set up the transport encryption
* Open channels with a key
* Accept channels opened by the remote end if your end knows the key
* Create and verify capability hashes
* Send and receive all protocol messages
* Register and use protocol extensions

*We're actively looking for contributors to the datrust development! If you're interested, say hi in the `#rust` channel on the [Hypercore Protocol Discord](https://chat.hypercore-protocol.org/) :-)*

## Examples

These examples sync data between Rust and NodeJS hypercore-protocol implementations. To prepare, run
```
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

## Contributing

We're actively looking for contributors to the datrust development! 

If you're interested, the easiest is to say hi in the `#rust` channel on the [Hypercore Protocol Discord](https://chat.hypercore-protocol.org/).

Contributions include pull requests, issue reports, documentation, design
and other work that benefits this project.

This project is welcoming contributions from anyone who acts in good faith!
We do not tolerate toxic behavior or discriminations against other contributors.
People who engage with this project in bad faith or fail to reflect and change
harmful behavior may be excluded from contributing. Should you feel that someone
acted in such a way, please reach out to the authors of this project.

Open, diverse, and inclusive communities live and die on the basis of trust.
Contributors can disagree with one another so long as they trust that those
disagreements are in good faith and everyone is working towards a common
goal.
