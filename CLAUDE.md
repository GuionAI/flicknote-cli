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

## Key Dependencies

- **powersync** — local path dependency (SQLite sync engine)
- **rusqlite** — SQLite with bundled + load_extension
- **clap** — CLI framework (derive macros)
- **tokio** — async runtime
- **reqwest** — HTTP client (auth + PostgREST backend)
- **serde/serde_json** — serialization

## Project Conventions

- Rust 2024 edition, resolver 3
- Guard clauses over deep nesting
- `thiserror` for error types
- Config via XDG dirs (`~/.config/flicknote/`) or env vars
- Data stored at `~/.local/share/flicknote/`

## CI (Woodpecker)

- **Never use `set -eo pipefail`** in Woodpecker pipeline scripts — Woodpecker runs commands with `/bin/sh`, not bash. `pipefail` is a bash-only option. Use `set -e` only.
- **Use `$$` for shell variables and secrets** in Woodpecker commands — Woodpecker substitutes `${VAR}` before passing to shell. Use `$${VAR}` to pass `$VAR` literally to the shell. CI variables like `CI_COMMIT_SHA` don't need `$$` (Woodpecker substitutes them).
- Hardcode versions inline in curl URLs — don't use shell variables that might not interpolate in all shells
- Mirror fb's `.woodpecker/containers.yaml` patterns exactly when writing pipeline configs

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
