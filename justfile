set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments

# List available recipes.
default:
    @just --list

# Build all Rust crates.
build:
    cargo build

# Build optimized Rust binaries.
build-release:
    cargo build --release

# Run all Rust tests.
test:
    cargo test

# Run formatting, lint, and test checks.
check: fmt clippy test

# Check Rust formatting.
fmt:
    cargo fmt --all --check

# Run Clippy with warnings denied.
clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Install the unified Rust CLI and daemon.
install: install-rust

# Install the Rust CLI.
install-rust:
    cargo install --path flicknote-cli

# Restart the installed FlickNote user daemon service.
restart:
    flicknote daemon restart

# Reinstall the unified executable and restart the FlickNote user daemon service.
reinstall: reinstall-rust restart

# Force-reinstall the Rust CLI.
reinstall-rust:
    cargo install --path flicknote-cli --force

# Remove Cargo build artifacts.
clean:
    cargo clean

# Install repository git hooks.
setup:
    lefthook install

# Alias for setup.
install-hooks: setup

# Cut and push a major, minor, or patch release.
release level:
    ./scripts/release.sh "$1"
