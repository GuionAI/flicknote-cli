# FlickNote CLI

Local-first note management CLI with cloud sync via PowerSync and Supabase.

## Architecture

Rust workspace with 5 crates:

- **flicknote-cli** — CLI binary (`flicknote`): add, find, list, get, replace, append, remove, rename, insert, modify, upload, archive, unarchive, project, login, logout, sync, import, api, tui
- **flicknote-core** — Shared library (db, config, schema, types, session, errors)
- **flicknote-auth** — Supabase GoTrue authentication (OTP + OAuth2/PKCE)
- **flicknote-sync** — Background sync daemon (PowerSync ↔ Supabase)
- **flicktask-cli** — CLI binary (`flicktask`): tree-based task management via TaskChampion + PowerSync. Commands: add, get, done, delete, start, stop, edit, tag, untag, annotate, move, list, tree, plan, undo, import

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
- **reqwest** — HTTP client (auth)
- **serde/serde_json** — serialization

## Project Conventions

- Rust 2024 edition, resolver 3
- Guard clauses over deep nesting
- `thiserror` for error types
- Config via XDG dirs (`~/.config/flicknote/`) or env vars
- Data stored at `~/.local/share/flicknote/`

## Commit Style

```
feat(scope): description
fix(scope): description
refactor(scope): description
chore(scope): description
```

Scopes: `cli`, `core`, `auth`, `sync`, `task`, `ci`
