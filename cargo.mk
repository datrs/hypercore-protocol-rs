##
##make cargo-*
cargo-help:### 	cargo-help
	@awk 'BEGIN {FS = ":.*?###"} /^[a-zA-Z_-]+:.*?###/ {printf "\033[36m%-15s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)
cargo-b:cargo-build### 	cargo b
cargo-build:### 	cargo build
## 	make cargo-build q=true
	@RUST_BACKTRACE=all cargo b $(QUIET)
cargo-i:cargo-install
cargo-install:### 	cargo install --path .
	@cargo install --path $(PWD)
cargo-br:cargo-build-release### 	cargo-br
## 	make cargo-br q=true
cargo-b-release:cargo-build-release### 	cargo-build-release
cargo-build-release:### 	cargo-build-release
## 	make cargo-build-release q=true
	@cargo b --release $(QUIET)
cargo-check:### 	cargo-check
	@cargo c
cargo-bench:### 	cargo-bench
	@cargo bench
cargo-test:### 	cargo-test
	@cargo test
cargo-report:### 	cargo-report
	cargo report future-incompatibilities --id 1

cargo-sort:## 	cargo-sort
	@[ -x cargo-sort ] || cargo install cargo-sort
	cargo-sort
#cargo-deny-check-bans:## 	cargo-deny-check-bans
#	@[ -x cargo-deny ] || cargo install cargo-deny
#	cargo deny check bans
#
# vim: set noexpandtab:
# vim: set setfiletype make
