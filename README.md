# flicknote-cli

Daemon-backed note management CLI with local-first sync. The CLI and MCP server use a typed Unix-socket API; the daemon owns SQLite and PowerSync.

## Features

- **Add & capture notes** — text, URLs (auto-detected as links), files
- **List & search notes** — filter by type, project, or keyword (`find`)
- **Get note details** — retrieve by numeric short ID; view heading structure with `--tree`
- **Edit notes** — human editor, append, content, and metadata workflows; structured content and section mutations are provided by MCP
- **MCP server** — typed local note, source, and project tools over stdio
- **Codex recall** — human-readable `recall QUERY` results and a read-only `UserPromptSubmit` hook with bounded historical note candidates
- **Archive notes** — archive and unarchive
- **Authentication** — email OTP or OAuth (Google/Apple) via Supabase
- **User daemon service** — foreground daemon managed by launchd (macOS) or systemd (Linux)

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

# Install to ~/.cargo/bin
just install
```

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

Installs the unified `flicknote` executable.

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
flicknote recall "Memory Systems"        # show matching historical candidates
flicknote recall ""                       # an explicit empty query returns no candidates

# Note IDs are numeric short IDs from list/detail. Full UUIDs are also accepted
# for compatibility.

# Get a specific note (use --tree to see section IDs)
flicknote detail <note-id>
flicknote detail <note-id> --tree
flicknote share <note-id>
flicknote unshare <note-id>
flicknote project share <project-id>
flicknote project unshare <project-id>

# Edit note metadata
flicknote modify <note-id> --project myproject
flicknote modify <note-id> --project myproject --flagged
flicknote modify <note-id> --unflagged

# Content and section mutations use the structured MCP interface. The MCP
# schemas carry exact before/after fields and section-scoped operations.

# Append
echo "more content" | flicknote append <note-id>

# Delete
flicknote delete <note-id>

# Manage the user daemon service
flicknote daemon install
flicknote daemon status
flicknote daemon logs --lines 100
flicknote daemon stop

# Foreground diagnosis (runs synchronously and keeps terminal output)
flicknote daemon run

# Reconcile/start the service after an upgrade
flicknote daemon restart

# Install the Codex recall hook (choose interactively, or pass a scope)
flicknote hook install codex
flicknote hook install codex --global
```

## Daemon lifecycle

`flicknote login` authenticates and then installs, starts, and verifies the user daemon.
`flicknote logout` stops and uninstalls it before clearing the session and local database.
After upgrading an existing dev installation for the cnsupa authentication cutover,
run `flicknote login --force` once. This stops and uninstalls the existing daemon,
replaces the old session, and installs, starts, and verifies the daemon again. It
does not delete the local database. `flicknote logout --force` is reserved for
explicit recovery when service cleanup cannot be confirmed:

```bash
flicknote login --force
flicknote logout --force
```

The public lifecycle commands are `daemon install`, `uninstall`, `start`, `stop`,
`restart`, `status`, `logs`, and `run`. `status --verbose` separates service
state, application readiness, IPC protocol/version, PowerSync connectivity, and
log guidance. `status --json` emits a stable object for automation. Data commands
and MCP never start services or open SQLite directly; if the daemon is unavailable,
run `flicknote daemon status` and `flicknote daemon start`.

See [docs/daemon.md](docs/daemon.md) for macOS/Linux service details and the
pre-upgrade uninstall boundary for installations using the old lifecycle.

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

The MCP server requires the local daemon. It exposes typed note, discovery,
note-source, project, and read-only recall tools. Note content and exact `before`/`after` edits
are structured JSON fields, so callers do not need shell heredocs. Note tools
accept numeric short IDs and do not expose internal UUIDs; project tools use
project names. `note_source` reads stored source data, while `note_get` reads
editable note content. Every data tool uses the running daemon; the MCP process
never opens SQLite. The server does not start the daemon automatically.

### Codex recall hook

`flicknote recall QUERY` is the human entrance for recall. It sends the text to
the daemon and prints up to five matching active-note candidates with their
numeric short IDs, titles, available summaries, and modification times. An
empty or unmatched query prints an empty-result message; it never lists every
note. Human recall gives the complete daemon call five seconds, including IPC
connection and response work. Use `--project NAME` or `FLICKNOTE_PROJECT` with
the same precedence as the other note queries.

The Codex entrance is a synchronous command hook. Install it without an MCP
registration or a running daemon:

```bash
flicknote hook install codex
```

The installer asks whether to enable the hook for the current project or your
user account. Use `--local` or `--global` to choose directly. It preserves
unrelated configuration, does not contact the daemon, and does not grant hook
trust. Repeating the installation replaces and coalesces recognizable command
recall entries in the selected hooks file. Old MCP `mcp_tool` recall hooks are
left untouched and do not block installation; remove an old MCP hook manually
if it remains enabled, otherwise recall may run twice. If an active command
recall entry is already in the other scope or an inline configuration source,
the installer reports its location instead of creating another entry.

The installed handler runs `flicknote recall --hook`. Codex supplies one
`UserPromptSubmit` event as JSON on stdin; the command validates the event and
string `prompt`, then emits the existing `hookSpecificOutput` JSON contract.
The command is static: prompt text is delivered through stdin and is never
interpolated into shell code. It uses the same five-candidate and 6000-byte
context bounds as the MCP `note_recall` tool. Hook and MCP recall allow three
seconds for the complete daemon call, and the installed command hook has a
three-second synchronous host timeout. That host timeout also bounds an input
stream that never reaches EOF. The hook needs the FlickNote daemon when a
prompt arrives; malformed input, an unavailable daemon, or a response timeout
emits diagnostics on stderr and no context on stdout, with a non-blocking
failure. A response timeout is distinct from an unavailable daemon: only the
latter calls for `flicknote daemon status` and `flicknote daemon start`.

Existing installed hooks keep their generated host timeout until explicitly
reinstalled. After upgrading, run `flicknote hook install codex --local` or
`flicknote hook install codex --global` for the selected scope; reinstalling
updates only the recognizable command-hook entry. The recall query improvement
is in the daemon, so an updated daemon must be running for it to take effect.
Synthetic measurements compare query variants and do not establish a universal
200–300 ms SLA.

In Codex, open `/hooks` to review and trust the installed hook. Project-local
hooks also require a trusted project.

When you send a message, the hook supplies up to five historical note candidates
for Codex to consider. Codex can read a relevant candidate with `note_get` and
check it against the current task. Recall is read-only: it does not modify your
notes. Messages with no matches receive no extra context; if recall is
unavailable, the conversation continues.

If the hook is not working, check `flicknote daemon status` and the hook's
enabled and trusted state in `/hooks`. The command hook does not depend on the
FlickNote MCP connection. The MCP `note_recall` tool remains available for MCP
clients that use it directly. See the
[Codex hooks documentation](https://developers.openai.com/codex/hooks) for host
setup and trust requirements.

The Gateway CLI command remains available for internal development and
maintenance requests; it is not the formal agent interface.

## Configuration

Config file: `~/.config/flicknote/config.json`

Environment variables:

- `FLICKNOTE_SUPABASE_URL`
- `FLICKNOTE_SUPABASE_KEY`
- `FLICKNOTE_POWERSYNC_URL`
- `FLICKNOTE_API_URL` — API Worker base URL for share links
- `FLICKNOTE_GATEWAY_URL` — Gateway origin for attachment operations and `gateway request`

For the default `dev` environment, the built-in `FLICKNOTE_SUPABASE_KEY` value
in the [runtime configuration](flicknote-core/src/config.rs) is an opaque cnsupa
publishable key, not the retired JWT-shaped anon key. The value is sent through
Supabase's existing `apikey` header. Existing dev users must upgrade and run
`flicknote login --force` once to replace the old session before normal sync;
explicit config-file and environment key overrides continue to work for custom
environments.

`apiUrl` and `gatewayUrl` can also be set in `config.json`. After changing either
value, restart the daemon with `flicknote daemon restart`. Configure the two
endpoint values together; setting only one is rejected.

Data directory: `~/.local/share/flicknote/`

## Architecture

Rust workspace with 4 crates:

| Crate | Type | Purpose |
|-------|------|---------|
| `flicknote-cli` | binary | Unified CLI/MCP client and foreground daemon executable |
| `flicknote-core` | library | Database, config, shared services, DTOs, types, schema |
| `flicknote-auth` | library | Supabase auth (OTP + OAuth2/PKCE) |
| `flicknote-sync` | library | Application RPC host, backend ownership, and PowerSync implementation |

## License

MIT
