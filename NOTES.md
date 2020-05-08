# Development notes

## Performance

A simple throughput benchmark is included. Run with

`cargo bench --bench throughput`

To get a flamegraph for the the benchmark, install `flamegraph` with `cargo install flamegraph`, and the

```bash
cargo bench --bench throughput
# Get the hash from the output
flamegraph ./target/release/deps/throughput-f85af157940f16e6 --bench throughput --profile-time 10
firefox flamegraph.svg
```
