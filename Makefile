.PHONY: build test check fmt clippy install clean release

build:
	cargo build

release:
	cargo build --release

test:
	cargo test

check: fmt clippy test

fmt:
	cargo fmt --check

clippy:
	cargo clippy --all-targets -- -D warnings

install:
	cargo install --path flicknote-cli

clean:
	cargo clean
