# FlickNote CLI

Local-first note management CLI with cloud sync via PowerSync and Supabase.

## Agent Instruction Source

- `AGENTS.md` is the project instruction source of truth for agents.
- Do not add or update `CLAUDE.md`; this repo no longer uses it.
- Keep agent-facing workflow rules here when they affect future coding,
  release, verification, or review behavior.

## Architecture

Rust workspace with 4 crates:

- **flicknote-cli** — CLI package (`flicknote`, `flicknote-sync`): thin CLI/MCP clients and daemon entrypoint; data commands never open SQLite or Postgres
- **flicknote-core** — Shared library (db, config, schema, types, session, services, DTOs, errors)
- **flicknote-auth** — Supabase GoTrue authentication (OTP + OAuth2/PKCE)
- **flicknote-sync** — Daemon application host, typed RPC boundary, backend ownership, and PowerSync ↔ Supabase sync

### modify vs replace

- `flicknote modify <id>` — edit-mode: exact-string replace via `===BEFORE===`/`===AFTER===` blocks, plus metadata
- `flicknote replace <id> --section <section-id>` — replaces one complete section subtree, including its heading; it does not change note metadata

## Build & Test

```bash
cargo build                # build all crates
cargo test                 # run all tests
cargo clippy               # lint
cargo fmt --check          # format check
```

Or use the justfile: `just build`, `just test`, `just check`, `just install`

### SQLx metadata

After changing any `sqlx::query!`, `query_as!`, or `query_scalar!` macro, run
`just sqlx-prepare` and commit the `.sqlx` changes. Do not hand-edit `.sqlx`
files.

`just sqlx-prepare` validates SQLite macros against the local fixture schema.
Keep `scripts/sqlx-sqlite-schema.sql` in sync with SQLite macro-selected
columns.

## Git Hooks (lefthook)

This repo uses lefthook for git hooks. Install once with `lefthook install` (or `just setup`).

- **pre-commit** runs `cargo fmt --check` — validates formatting (does NOT auto-fix). If it fails, run `cargo fmt` then re-commit.
- **pre-push** runs the SQLx offline check, clippy, and cargo deny. Requires `cargo install cargo-deny`.

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
- **sqlx** — typed local SQLite access pending migration to the shared PowerSync pool

## Project Conventions

- Rust 2024 edition, resolver 3
- Guard clauses over deep nesting
- `thiserror` for error types
- Config via XDG dirs (`~/.config/flicknote/`) or env vars
- Data stored at `~/.local/share/flicknote/`

## CI (GitHub Actions)

This repo uses GitHub Actions for CI/CD (no Woodpecker, no moon).

- **pr.yaml** — Rust check (fmt/clippy/test/deny/build)
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
