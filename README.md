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

# Install to ~/.cargo/bin
make install
```

Or directly with cargo:

```bash
cargo build --release
cargo install --path flicknote-cli
```

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

# Get a specific note (use --tree to see section IDs)
flicknote get <note-id>
flicknote get <note-id> --tree

# Edit note content
echo "updated content" | flicknote replace <note-id>
echo "updated content" | flicknote replace <note-id> --section <section-id>
echo "more content" | flicknote append <note-id>

# Archive
flicknote archive <note-id>

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

Rust workspace with 4 crates:

| Crate | Type | Purpose |
|-------|------|---------|
| `flicknote-cli` | binary | CLI commands |
| `flicknote-core` | library | Database, config, types, schema |
| `flicknote-auth` | library | Supabase auth (OTP + OAuth2/PKCE) |
| `flicknote-sync` | binary | Background sync daemon |

## License

MIT
