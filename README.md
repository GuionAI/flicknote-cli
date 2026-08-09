# flicknote-cli

Daemon-backed note management CLI with local-first sync. The CLI and MCP server use a typed Unix-socket API; the daemon owns SQLite/PowerSync or the configured managed Postgres backend.

## Features

- **Add & capture notes** — text, URLs (auto-detected as links), files
- **List & search notes** — filter by type, project, or keyword (`find`)
- **Get note details** — retrieve by numeric short ID; view heading structure with `--tree`
- **Edit notes** — modify exact text, or replace, append, insert, remove, and rename sections by ID
- **MCP server** — typed local note, source, and project tools over stdio
- **Archive notes** — archive and unarchive
- **Authentication** — email OTP or OAuth (Google/Apple) via Supabase
- **Background sync** — daemon process with launchd integration (macOS)

## Build

Requires Rust 2024 edition (nightly or recent stable with edition support) and
[`just`](https://github.com/casey/just).

```bash
# Build all crates
just build

# Run tests
just test

# Lint + format check
just check

# Refresh sqlx offline metadata after SQL macro changes
just sqlx-prepare

# Install to ~/.cargo/bin
just install
```

CI sets `SQLX_OFFLINE=true`. After adding or changing `sqlx::query!`,
`query_as!`, or `query_scalar!` macros, run `just sqlx-prepare` and commit
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
just release patch
```

Use `major`, `minor`, or `patch`. `cargo-release` updates the shared workspace
version, commits it, and creates the `vX.Y.Z` tag. The recipe pushes the commit
and tag through `og`, which uses the daemon's project-scoped credentials. The
tag triggers cargo-dist.

Use `just --dry-run release patch` to print the commands without running them.
If a push fails, keep `main` at the release commit and rerun the same command to
resume the pending tag.

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

# Note IDs are numeric short IDs from list/detail. Full UUIDs are also accepted
# for compatibility.

# Get a specific note (use --tree to see section IDs)
flicknote detail <note-id>
flicknote detail <note-id> --tree
flicknote share <note-id>
flicknote unshare <note-id>
flicknote project share <project-id>
flicknote project unshare <project-id>

# Edit note content
# Precision edit (exact-string replace)
cat <<'EDIT' | flicknote modify <note-id>
===BEFORE===
typo here
===AFTER===
fixed here
EDIT

# Replace one section, including its heading and child sections
echo "## Heading
body" | flicknote replace <note-id> --section <section-id>

# For a whole-note rewrite, archive the old note and create a new note.

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

## MCP server

`flicknote mcp` runs a local MCP server over stdio. Configure an MCP client to
start it as a subprocess:

```json
{
  "mcpServers": {
    "flicknote": {
      "command": "flicknote",
      "args": ["mcp"]
    }
  }
}
```

The MCP server exposes typed note, note-source, and project tools. Note content
and exact `before`/`after` edits are JSON fields, so callers do not need shell
heredocs. Note tools accept numeric short IDs and do not expose internal UUIDs;
project tools use project names. `note_source` reads stored source data, while
`note_get` reads editable note content. Every data tool uses the running daemon;
the MCP process never opens SQLite or connects to Postgres. The daemon chooses
one backend at startup: local PowerSync by default, or managed Postgres when
`DATABASE_URL` is set in the daemon environment. The server does not start the
daemon automatically.

## Configuration

Config file: `~/.config/flicknote/config.json`

Environment variables:
- `FLICKNOTE_SUPABASE_URL`
- `FLICKNOTE_SUPABASE_KEY`
- `FLICKNOTE_POWERSYNC_URL`
- `DATABASE_URL` (daemon-only managed backend selection)

Data directory: `~/.local/share/flicknote/`

## Architecture

Rust workspace with 4 crates:

| Crate | Type | Purpose |
|-------|------|---------|
| `flicknote-cli` | binary | Thin CLI/MCP clients and installable daemon binary |
| `flicknote-core` | library | Database, config, shared services, DTOs, types, schema |
| `flicknote-auth` | library | Supabase auth (OTP + OAuth2/PKCE) |
| `flicknote-sync` | library | Application RPC host, backend ownership, and PowerSync implementation |

## License

MIT
