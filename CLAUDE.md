# FlickNote CLI

Local-first note management CLI with cloud sync via PowerSync and Supabase.

## Architecture

Rust workspace with 5 crates:

- **flicknote-cli** — CLI binary (`flicknote`): add, find, list, count, detail, content, modify, append, delete, restore, rename, insert, upload, project, prompt, keyterm, login, logout, sync, import, api, tui
- **flicknote-core** — Shared library (db, config, schema, types, session, errors)
- **flicknote-auth** — Supabase GoTrue authentication (OTP + OAuth2/PKCE)
- **flicknote-sync** — Background sync daemon (PowerSync ↔ Supabase)

## Build & Test

```bash
cargo build                # build all crates
cargo test                 # run all tests
cargo clippy               # lint
cargo fmt --check          # format check
```

Or use the Makefile: `make build`, `make test`, `make check`, `make install`

## Git Hooks (lefthook)

This repo uses lefthook for git hooks. Install once with `lefthook install` (or `make setup`).

- **pre-commit** runs `cargo fmt --check` — validates formatting (does NOT auto-fix). If it fails, run `cargo fmt` then re-commit.
- **pre-push** runs clippy, cargo deny, go vet, and golangci-lint. Requires `cargo install cargo-deny` and `go install github.com/golangci/golangci-lint/v2/cmd/golangci-lint@latest`

Manual usage:

```bash
lefthook run pre-commit  # run pre-commit hooks
lefthook run pre-push    # run pre-push hooks
```

## Key Dependencies

- **powersync** — local path dependency (SQLite sync engine)
- **rusqlite** — SQLite with bundled + load_extension
- **clap** — CLI framework (derive macros)
- **tokio** — async runtime
- **reqwest** — HTTP client (auth + PostgREST backend)
- **serde/serde_json** — serialization
- **postgres** — sync Postgres client for pgwire backend
- **sea-query** — SQL query builder (1.0.0-rc.32 + sea-query-postgres 0.6.0-rc.3 for pgwire)

## Project Conventions

- Rust 2024 edition, resolver 3
- Guard clauses over deep nesting
- `thiserror` for error types
- Config via XDG dirs (`~/.config/flicknote/`) or env vars
- Data stored at `~/.local/share/flicknote/`

## CI (GitHub Actions)

This repo uses GitHub Actions for CI/CD (no Woodpecker, no moon).

- **pr.yaml** — Rust check (fmt/clippy/test/deny/build), Go TUI (vet/build)
- **ci.yaml** — two parallel jobs: build (cargo test + build), lint (cargo fmt/clippy)
- **release.yml** — cargo-dist on version tags → GitHub Releases → guionai/homebrew-tap + guionai/homebrew-flicknote

Commit scope: `ci`

## Skills

The `skills/` directory contains command reference docs for AI agents:

- `skills/flicknote.md` — FlickNote CLI command reference

## Commit Style

```
feat(scope): description
fix(scope): description
refactor(scope): description
chore(scope): description
```

Scopes: `cli`, `core`, `auth`, `sync`, `ci`
