# flicknote-cli

Local-first note management CLI with cloud sync. Captures, queries, and manages notes stored in a local SQLite database synced to the cloud via PowerSync and Supabase.

## Features

- **Add & capture notes** — text, URLs (auto-detected as links), files
- **List & search notes** — filter by type, project, or keyword (`find`)
- **Get note details** — retrieve by full or partial UUID; view heading structure with `--tree`
- **Edit notes** — replace, append, insert, remove, rename sections by ID
- **Archive notes** — archive and unarchive
- **Authentication** — email OTP or OAuth (Google/Apple) via Supabase
- **Background sync** — daemon process with launchd integration (macOS)

## Build

Requires Rust 2024 edition (nightly or recent stable with edition support).

```bash
# Build all crates
make build

# Run tests
make test

# Lint + format check
make check

# Refresh sqlx offline metadata after SQL macro changes
make sqlx-prepare

# Install to ~/.cargo/bin
make install
```

CI sets `SQLX_OFFLINE=true`. After adding or changing `sqlx::query!`,
`query_as!`, or `query_scalar!` macros, run `make sqlx-prepare` and commit
the generated `.sqlx` metadata. The prepare script checks SQLite against a
local fixture DB and pgwire against the local Supabase Postgres used by
`flicknote-services` sqlc (`localhost:30432/supabase` by default), then merges
both metadata sets.

Runtime-built `sqlx::query` calls are checked at build time for Rust types, but
sqlx does not emit offline metadata for them.

Or directly with cargo:

```bash
cargo build --release
cargo install --path flicknote-cli
```

## Install

### Homebrew (macOS + Linux)

```bash
brew install GuionAI/tap/flicknote
```

Installs both `flicknote` and `flicknote-sync`.

## Release

```bash
cargo install cargo-release --locked
make release-plan VERSION=0.1.8
make cut-release VERSION=0.1.8
```

`cargo-release` updates the shared workspace version, commits it, creates the `vX.Y.Z` tag, and pushes the tag that triggers cargo-dist.

## Usage

```bash
# Authenticate
flicknote login --email user@example.com

# Add notes
flicknote add "Meeting notes about API redesign"
flicknote add https://example.com          # URL auto-detected as link note
echo "long content" | flicknote add --project myproject

# List and search
flicknote list
flicknote list --type link --limit 10
flicknote find rust
flicknote find rust effect                 # OR match across multiple keywords

# Note IDs are numeric short IDs from list/detail. A full UUID is also accepted
# for notes before short ID sync completes; UUID prefixes are not supported.

# Get a specific note (use --tree to see section IDs)
flicknote detail <note-id>
flicknote detail <note-id> --tree

# Edit note content
# Precision edit (exact-string replace)
cat <<'EDIT' | flicknote modify <note-id>
===BEFORE===
typo here
===AFTER===
fixed here
EDIT

# Overwrite (full replacement)
echo "new content" | flicknote replace <note-id>
echo "## Heading
body" | flicknote replace <note-id> --section <section-id>

# Append
echo "more content" | flicknote append <note-id>

# Delete
flicknote delete <note-id>

# Manage sync daemon
flicknote sync start
flicknote sync status
flicknote sync stop

# Install as launchd service (macOS)
flicknote sync install
```

## Configuration

Config file: `~/.config/flicknote/config.json`

Environment variables:
- `FLICKNOTE_SUPABASE_URL`
- `FLICKNOTE_SUPABASE_KEY`
- `FLICKNOTE_POWERSYNC_URL`

Data directory: `~/.local/share/flicknote/`

## Architecture

Rust workspace with 4 crates + 1 Go binary:

| Crate | Type | Purpose |
|-------|------|---------|
| `flicknote-cli` | binary | CLI commands and installable sync daemon binary |
| `flicknote-core` | library | Database, config, types, schema |
| `flicknote-auth` | library | Supabase auth (OTP + OAuth2/PKCE) |
| `flicknote-sync` | library | Background sync daemon implementation |
| `flicknote-tui` | binary (Go) | Terminal UI (`flicknote tui`) |

## License

MIT
