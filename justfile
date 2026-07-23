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

# Build the TUI.
build-tui:
    cd flicknote-tui && go build -o ../target/flicknote-tui .

# Run all Rust tests.
test:
    cargo test

# Run formatting, lint, and test checks.
check: fmt clippy test

# Check Rust formatting.
fmt:
    cargo fmt -p flicknote-auth -p flicknote-cli -p flicknote-core -p flicknote-sync --check

# Run Clippy with warnings denied.
clippy:
    cargo clippy -p flicknote-auth -p flicknote-cli -p flicknote-core -p flicknote-sync --all-targets -- -D warnings

# Refresh SQLx offline metadata.
sqlx-prepare:
    ./scripts/sqlx-prepare.sh

# Install both the Rust CLI and TUI.
install: install-rust install-tui

# Install the Rust CLI.
install-rust:
    cargo install --path flicknote-cli

# Install the TUI.
install-tui:
    cd flicknote-tui && go install .

# Reinstall both binaries and restart FlickNote launchd services.
reinstall: reinstall-rust install-tui
    @for label in $(launchctl list 2>/dev/null | awk '/io\.guion\.flicknote/ {print $3}'); do \
        echo "Restarting $label..."; \
        launchctl kickstart -k "gui/$(id -u)/$label"; \
        echo "✓ $label restarted"; \
    done

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
