# FlickNote CLI

Local-first note management CLI with cloud sync via PowerSync and Supabase.

## Architecture

Rust workspace with 5 crates:

- **flicknote-cli** — CLI binary (`flicknote`): add, find, list, get, replace, append, remove, rename, insert, modify, upload, archive, unarchive, project, login, logout, sync, import, api, tui
- **flicknote-core** — Shared library (db, config, schema, types, session, errors)
- **flicknote-auth** — Supabase GoTrue authentication (OTP + OAuth2/PKCE)
- **flicknote-sync** — Background sync daemon (PowerSync ↔ Supabase)
- **flicktask-cli** — CLI binary (`flicktask`): tree-based task management via TaskChampion + PowerSync. Commands: add, get, done, delete, start, stop, edit, tag, untag, annotate, move, list, tree, plan, undo, import, export

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

## CI (Woodpecker)

- **Never use `set -eo pipefail`** in Woodpecker pipeline scripts — Woodpecker runs commands with `/bin/sh`, not bash. `pipefail` is a bash-only option. Use `set -e` only.
- **Use `$$` for shell variables and secrets** in Woodpecker commands — Woodpecker substitutes `${VAR}` before passing to shell. Use `$${VAR}` to pass `$VAR` literally to the shell. CI variables like `CI_COMMIT_SHA` don't need `$$` (Woodpecker substitutes them).
- Hardcode versions inline in curl URLs — don't use shell variables that might not interpolate in all shells
- Mirror fb's `.woodpecker/containers.yaml` patterns exactly when writing pipeline configs

## Skills

The `skills/` directory contains command reference docs for AI agents:

- `skills/flicknote.md` — FlickNote CLI command reference
- `skills/flicktask.md` — FlickTask CLI command reference

## Commit Style

```
feat(scope): description
fix(scope): description
refactor(scope): description
chore(scope): description
```

Scopes: `cli`, `core`, `auth`, `sync`, `task`, `ci`

## Hook Protocol

flicktask implements the taskwarrior-compatible hook protocol.

- **Hooks dir:** `~/.config/flicktask/hooks/`
- **on-add-\*** — triggered by `flicktask add`
- **on-modify-\*** — triggered by `edit`, `done`, `delete`, `start`, `stop`, `tag`, `untag`, `annotate`, `move`
- Hooks run in alphabetical order. Non-zero exit aborts the operation.
- Same stdin/stdout JSON protocol as taskwarrior — ttal's hook shims work unchanged.
- Install ttal hooks with: `ttal doctor --fix` (once ttal is updated to target flicktask hooks dir too)
