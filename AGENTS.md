# FlickNote CLI

Local-first note management CLI with cloud sync via PowerSync and Supabase.

## Agent Instruction Source

- `AGENTS.md` is the project instruction source of truth for agents.
- Do not add or update `CLAUDE.md`; this repo no longer uses it.
- Keep agent-facing workflow rules here when they affect future coding,
  release, verification, or review behavior.

## Architecture

Rust workspace with 4 crates:

- **flicknote-cli** — CLI package (`flicknote`, `flicknote-sync`): add, find, list, count, detail, content, replace, modify, append, delete, restore, rename, insert, project, prompt, keyterm, login, logout, sync, import, tui
- **flicknote-core** — Shared library (db, config, schema, types, session, errors)
- **flicknote-auth** — Supabase GoTrue authentication (OTP + OAuth2/PKCE)
- **flicknote-sync** — Background sync daemon library (PowerSync ↔ Supabase)

### modify vs replace

- `flicknote modify <id>` — edit-mode: exact-string replace via `===BEFORE===`/`===AFTER===` blocks, plus metadata
- `flicknote replace <id>` — overwrite: replaces entire note or section (including heading), plus metadata

## Build & Test

```bash
cargo build                # build all crates
cargo test                 # run all tests
cargo clippy               # lint
cargo fmt --check          # format check
```

Or use the Makefile: `make build`, `make test`, `make check`, `make install`

### SQLx metadata

After changing any `sqlx::query!`, `query_as!`, or `query_scalar!` macro, run
`make sqlx-prepare` and commit the `.sqlx` changes. Do not hand-edit `.sqlx`
files.

For pgwire metadata, `make sqlx-prepare` must run against a local Postgres
schema that already has the matching FlickNote backend migrations applied. If
prepare reports a missing column or relation, update the local prepare DB from
the backend migrations first, then rerun prepare. Keep
`scripts/sqlx-sqlite-schema.sql` in sync with SQLite macro-selected columns.

When the CLI depends on a fresh backend schema change, confirm the local prepare
database has that backend migration applied before trusting generated metadata.
For short-id work, this means the local database must include the backend
`add_user_short_ids` migration before regenerating pgwire metadata.

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
- **release.yml** — cargo-dist on version tags → GitHub Releases → GuionAI/homebrew-tap

Commit scope: `ci`

## Skills

The `skills/` directory contains command reference docs for AI agents:

- `skills/flicknote.md` — FlickNote CLI command reference

Agent quick reference is deployed via `ttal sync` to the runtime agent rules.

## Commit Style

```
feat(scope): description
fix(scope): description
refactor(scope): description
chore(scope): description
```

Scopes: `cli`, `core`, `auth`, `sync`, `ci`
