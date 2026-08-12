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

### MCP interface

FlickNote MCP is the formal model interface for note operations. The CLI remains
for human and operational workflows; content and section mutations are not CLI
commands.

Every MCP structured result must have an object root, and each advertised output
schema must be precise and derived from its boundary DTO. Arbitrary JSON schema
terms must use object form rather than bare boolean terms. Every MCP change must
pass the repository-wide strict-client output-schema contract test.


## Build & Test

```bash
cargo build                # build all crates
cargo test                 # run all tests
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check    # format check
```

Or use the justfile: `just build`, `just test`, `just check`, `just install`

## Git Hooks (lefthook)

This repo uses lefthook for git hooks. Install once with `lefthook install` (or `just setup`).

- **pre-commit** runs `cargo fmt --all --check` — validates formatting (does NOT auto-fix). If it fails, run `cargo fmt --all` then re-commit.
- **pre-push** runs the workspace/all-target/all-feature check, clippy with warnings denied, and cargo deny. Requires `cargo install cargo-deny`.

Manual usage:

```bash
lefthook run pre-commit  # run pre-commit hooks
lefthook run pre-push    # run pre-push hooks
```

## Key Dependencies

- **powersync** — Guion fork of the SQLite sync engine
- **rusqlite** — SQLite with bundled + load_extension
- **clap** — CLI framework (derive macros)
- **tokio** — async runtime
- **reqwest** — HTTP client (auth + PostgREST backend)
- **serde/serde_json** — serialization

## Project Conventions

- Rust 2024 edition, resolver 3
- Guard clauses over deep nesting
- Workspace Clippy keeps `too_many_lines`, `cognitive_complexity`, `large_futures`, and `future_not_send` enabled; CI denies all warnings across every target and feature
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

- `skills/flicknote.md` — concise MCP-first FlickNote guidance

The bundled skill is installed with `flicknote skill install`.

## Commit Style

```
feat(scope): description
fix(scope): description
refactor(scope): description
chore(scope): description
```

Scopes: `cli`, `core`, `auth`, `sync`, `ci`
