.PHONY: build test check fmt clippy sqlx-prepare install install-rust install-tui reinstall reinstall-rust clean release release-plan cut-release setup install-hooks build-tui

build:
	cargo build

build-tui:
	cd flicknote-tui && go build -o ../target/flicknote-tui .

release:
	cargo build --release

release-plan:
	@test -n "$(VERSION)" || (echo "VERSION is required, e.g. make release-plan VERSION=0.1.8"; exit 1)
	cargo release $(VERSION)

cut-release:
	@test -n "$(VERSION)" || (echo "VERSION is required, e.g. make cut-release VERSION=0.1.8"; exit 1)
	cargo release $(VERSION) --execute

test:
	cargo test

check: fmt clippy test

fmt:
	cargo fmt -p flicknote-auth -p flicknote-cli -p flicknote-core -p flicknote-sync --check

clippy:
	cargo clippy -p flicknote-auth -p flicknote-cli -p flicknote-core -p flicknote-sync --all-targets -- -D warnings

sqlx-prepare:
	./scripts/sqlx-prepare.sh

install: install-rust install-tui

install-rust:
	cargo install --path flicknote-cli

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

clean:
	cargo clean

setup:
	lefthook install

install-hooks: setup
