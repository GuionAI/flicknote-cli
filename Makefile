.PHONY: build test check fmt clippy install install-rust install-tui reinstall reinstall-rust clean release setup build-tui

build:
	cargo build

build-tui:
	cd flicknote-tui && go build -o ../target/flicknote-tui .

release:
	cargo build --release

test:
	cargo test

check: fmt clippy test

fmt:
	cargo fmt --check

clippy:
	cargo clippy --all-targets -- -D warnings

install: install-rust install-tui

install-rust:
	cargo install --path flicknote-cli
	cargo install --path flicknote-sync
	cargo install --path flicktask-cli

install-tui:
	cd flicknote-tui && go install .

reinstall: reinstall-rust install-tui
	@for label in $$(launchctl list 2>/dev/null | awk '/io\.guion\.flicknote/ {print $$3}'); do \
		echo "Restarting $$label..."; \
		launchctl kickstart -k "gui/$$(id -u)/$$label"; \
		echo "✓ $$label restarted"; \
	done

reinstall-rust:
	cargo install --path flicknote-cli --force
	cargo install --path flicknote-sync --force
	cargo install --path flicktask-cli --force

clean:
	cargo clean

setup:
	qlty githooks install
