.PHONY: build test check fmt clippy install reinstall clean release

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
	cargo install --path flicknote-sync

reinstall:
	cargo install --path flicknote-cli --force
	cargo install --path flicknote-sync --force
	@for label in $$(launchctl list 2>/dev/null | awk '/io\.guion\.flicknote/ {print $$3}'); do \
		echo "Restarting $$label..."; \
		launchctl kickstart -k "gui/$$(id -u)/$$label"; \
		echo "✓ $$label restarted"; \
	done

clean:
	cargo clean
