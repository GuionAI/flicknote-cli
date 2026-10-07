# flicknote-cli

Daemon-backed note management CLI with local-first sync and an explicit private remote MCP server. The CLI uses typed Unix-socket IPC; the daemon owns SQLite, PowerSync, and the local MCP HTTP endpoint.

## Features

- **Add & capture notes** — text, URLs (auto-detected as links), files
- **List & search notes** — filter by type, project, or keyword (`find`)
- **Get note details** — retrieve by numeric short ID; view heading structure with `--tree`
- **Edit notes** — human editor plus explicit machine append, write, metadata, and draft-submit workflows; structured section mutations are provided by MCP
- **MCP server** — typed local note, source, and project tools over Streamable HTTP
- **Private remote MCP** — verified full-grant HTTP access to each user's notes through PostgreSQL RLS and PGroonga
- **Codex recall** — human-readable `recall QUERY` results and a read-only `UserPromptSubmit` hook with bounded historical note candidates
- **Archive notes** — archive and unarchive
- **Authentication** — email OTP or OAuth (Google/Apple) via Supabase
- **User daemon service** — foreground daemon managed by launchd (macOS) or systemd (Linux)

## Build

Requires rustup and [`just`](https://github.com/casey/just). The exact Rust
version, components and targets are declared in `rust-toolchain.toml`; local
Cargo commands, routine CI and release compilation use that file. Run
`rustup show active-toolchain` from the checkout to install or verify it.

To upgrade Rust, change the channel in `rust-toolchain.toml`, run the full
routine suite and regenerate/check the release workflow using the procedure
below. Do not add a separate compiler version to CI or dist configuration.
A pin change can download the selected toolchain if it is not installed.

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

Private PostgreSQL/MCP verification uses every unchanged fb migration in an
isolated PGroonga database. Run `scripts/test-private-pg.sh` with Podman,
dbmate, Python 3 and og read access to fb. It logs the resolved mainline commit
and image; use `FLICKNOTE_TEST_FB_REVISION=COMMIT` for replay or
`FLICKNOTE_TEST_FB_CHECKOUT=/path/to/fb` for a read-only checkout override.
See [migration-backed verification](docs/private-mcp.md#migration-backed-cli-verification)
for prerequisites, failure checks and limitations. This local harness is mandatory
for columns, SQL adapters, PG setup, RLS, search, and shared database/MCP behavior
that affects PostgreSQL. Record the pinned fb revision and harness outcome in
implementation and PR evidence; CI excludes these ignored integration tests.

Or directly with cargo:

```bash
cargo build --release
cargo install --path flicknote-cli
```

The Linux musl distribution statically builds OpenSSL through its supported
`vendored` feature. Its builder needs a musl C compiler, Perl, make, and
clang/libclang for PowerSync bindings (on Debian/Ubuntu: `musl-tools`, `perl`,
`make`, `clang`, `libclang-dev`). Build the unified binary with
`cargo build --locked --profile dist --target x86_64-unknown-linux-musl -p flicknote-cli`.
Other targets retain system OpenSSL discovery. See the
[private MCP contract](docs/private-mcp.md) for runtime CA configuration.

## Routine verification

Run `bash scripts/check-routine.sh` locally for formatting, locked workspace
all-feature tests, locked workspace/all-target/all-feature Clippy with warnings
denied, `cargo deny check`, and the release-script fixtures. Install `cargo-deny`
to run the full suite. Git hooks remain unchanged.

GitHub runs this suite daily at **03:17 UTC** on its default branch and through
**Actions → Routine checks → Run workflow** on the selected ref. There are no
remote PR/main-push quality checks or daily binary snapshots. Scheduling requires
the workflow on the GitHub default branch and enabled Actions; checked-in YAML
and local checks do not establish a live scheduled run or that the GitHub mirror
matches Forgejo main. No private fb CI token, network access, or runner setup is
needed by the routine suite.

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

If preparation was interrupted before changing the version and `main` later
advanced, rerunning the same command restarts preparation from the new HEAD
when the working tree is clean and `Cargo.toml`/`Cargo.lock` are unchanged.
Version-changing commits without a tag or uncommitted preparation changes stay
pending for inspection; the script does not discard them.

Each release runs the same reusable routine checks on the tag event's exact
`github.sha`, independently of previous daily results. Successful checks precede
even cargo-dist's `host --steps=create` planning step. A failed, skipped or
cancelled check blocks public GitHub assets/releases and Homebrew updates.

The release configuration uses cargo-dist's supported reusable `plan-jobs`
hook. Version 0.31.0 runs that hook alongside plan and permits skipped build
jobs at host, so a small version-bound generator adds strict success dependencies
and pins source checkouts to the event SHA. `allow-dirty = ["ci"]` prevents dist
from overwriting that customization. With cargo-dist 0.31.0 on PATH:

```bash
python3 scripts/generate-release-workflow.py
python3 scripts/generate-release-workflow.py --check
```

Use this procedure when editing dist configuration or upgrading cargo-dist;
plain `dist init`/`dist generate` does not recreate the gate. The check generates
in temporary state and fails on template drift or a different checked-in result.
See [cargo-dist customization](https://axodotdev.github.io/cargo-dist/book/ci/customizing.html).

The release uses cargo-dist 0.31.0 for `x86_64-unknown-linux-musl` and
`aarch64-apple-darwin`. Verify the published Linux archive and its `.sha256`
asset before handing the version to fse, which packages public GitHub Release
binaries only. A failed public release tag stays immutable; publish a new
patch after the correction is reviewed and merged.

## Usage

```bash
# Authenticate
flicknote login --email user@example.com

# Add notes
flicknote add "Meeting notes about API redesign"
flicknote add https://example.com          # URL auto-detected as link note
echo "long content" | flicknote add --project myproject
echo "draft content" | flicknote add --draft --json

# List and search
flicknote list
flicknote list --type link --limit 10
flicknote list --limit 10 --cursor 123 # continue after note 123
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
flicknote modify <note-id> --title "Updated title"
flicknote modify <note-id> --clear-summary --clear-project

# Machine content writes preserve lifecycle state. Section mutations use MCP.

# Append
echo "more content" | flicknote append <note-id>
cat replacement.md | flicknote write <note-id> --json

# Submit a draft explicitly; ordinary edits never requeue AI processing.
flicknote submit <note-id>

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

The daemon serves MCP over Streamable HTTP at `http://127.0.0.1:37789/mcp`.
For Codex, replace the old `command = "flicknote"` MCP registration with:

```toml
[mcp_servers.flicknote]
url = "http://127.0.0.1:37789/mcp"
```

Start the daemon before connecting Codex. The HTTP endpoint binds only to the
loopback address and checks the request origin and host.

The MCP server exposes typed note, discovery,
note-source, project, and read-only recall tools. `note_get` returns actual stored
note content; the synthesized editable document is used only by the human editor.
Machine content, metadata, and lifecycle operations are orthogonal: ordinary
content or metadata mutations preserve status and never start AI processing.
Local and remote MCP `note_add` create ordinary notes: text enters `ai_queued`
and recognized URLs enter `source_queued`. The tool accepts content and optional
project; a supplied `draft` argument is rejected before insertion. Use the human
CLI `flicknote add --draft` to create drafts. Existing drafts remain readable and
editable through MCP. `note_submit` is the explicit draft transition, while
`note_write` replaces stored content without changing metadata or lifecycle. Note content and exact `before`/`after` edits
are structured JSON fields, so callers do not need shell heredocs. Note tools
accept numeric short IDs and do not expose internal UUIDs; project tools use
project names. `note_source` reads stored source data. Every data tool uses the running daemon; the MCP process
never opens SQLite. The server does not start the daemon automatically.

MCP creation sets `metadata.created_by_ai` to JSON boolean `true` regardless of
client or session. Human CLI/GUI creation omits it. Human-only list, count and
find exclude only boolean `true`; missing and `false` are human creation.
Recall returns human-created candidates. The marker describes the creation
channel, not content authorship; later edits, AI processing and lifecycle
changes preserve it. `note_get` exposes stored metadata; `note_add` has no
public marker parameter. Historical `created_by` strings have no runtime
meaning and receive no automatic conversion.

### Private remote MCP

`flicknote private-mcp` runs a separate foreground server against PostgreSQL.
It requires an explicit database URL environment variable, listen address,
canonical public resource, OAuth issuer and verifier, and exact Host/Origin
allowlists. It does not read the local login, configuration, or PowerSync data.
The remote endpoint exposes the supported note, project, topic, and entity tool
subset and verifies a resource-bound `flicknote:full` grant on every request.
The fb OAuth Worker grant verifier and db-init PGroonga indexes are prerequisites
for production use; this repository does not provide those fb changes.

See [the private MCP operator contract](docs/private-mcp.md) for the command,
grant response, database role/RLS requirements, supported tools, and the
test-owned PGroonga verification command.

### Codex recall hook

`flicknote recall QUERY` is the human entrance for recall. It sends the text to
the daemon and prints up to five matching active-note candidates with their
numeric short IDs, titles, available summaries, and modification times. An
empty or unmatched query prints an empty-result message; it never lists every
note. Human recall gives the complete daemon call five seconds, including IPC
connection and response work. Use `--project NAME` to filter by project.

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

Rust workspace with 5 production crates and 1 experimental package:

| Crate | Type | Purpose |
|-------|------|---------|
| `flicknote-cli` | binary | Unified CLI, foreground daemon, and private remote MCP executable |
| `flicknote-client` | library | Pure application DTOs, wire protocol, errors, and async Unix socket client |
| `flicknote-core` | library | Database, config, shared services, storage types, schema, and DTO conversions |
| `flicknote-auth` | library | Supabase auth (OTP + OAuth2/PKCE) |
| `flicknote-sync` | library | Application RPC host, backend ownership, and PowerSync implementation |
| `flicknote-gpui` | macOS binary | macOS GUI host with watched Today and native input |

Rust clients can call the daemon directly with an explicit socket path through
[`flicknote-client`](flicknote-client/README.md), without linking database, sync,
auth, or MCP server implementations. The unified executable still hosts the
daemon and therefore retains those dependencies. This extraction preserves the
existing user/operator commands, configuration, IPC wire version, and CLI/MCP
outputs. The separate experimental desktop package is described below.

## Experimental embedded GPUI spike

Build on Apple Silicon macOS with `cargo build --locked -p flicknote-gpui
-p flicknote-cli`. Launch with an explicit independent absolute directory:

```bash
FLICKNOTE_ENV=dev target/debug/flicknote-gpui --root /tmp/flicknote-synthetic-spike --mcp-port 0
```

Synthetic mode is available through the macOS GUI. Normal headless operation
uses `flicknote daemon run`; it does not use synthetic fixtures.

This is a synthetic fixture experiment. It embeds real PowerSync, shared
application operations and the existing local MCP/IPC servers. It loads no login
or cloud session. The GUI presents a continuous workbench with a compact rail, full-width note
rows, a docked create/append composer and an optional right readonly Markdown reading pane.
Use the FlickNote application menu for System, Light or Dark appearance with
independently authored Light/Dark workbench roles adapted through existing Kit. The
minimum window size is 760×560 points. Home and active projects are working
destinations; search, Shared, Archive browsing and Charts remain inactive rail landmarks with aligned packaged Lucide icons
and project dots. Detail fills a separate 272–420-point reading pane without covering the list or composer.
Option-J/K select next/previous confirmed notes without wrapping and preserve
editor focus; empty Return opens selected detail, and Option-A archives with
the existing input guard. Unsubmitted/marked input blocks note-selection shortcuts.
Markdown editing, formula/Mermaid rendering, unsupported navigation shortcuts, global trigger,
voice and full Swift desktop parity remain deferred. Home watches current Today;
projects watch All active project notes across dates, capped at 10,000. Cmd1 selects
Home; Cmd2..9 select the first eight active projects in displayed rail order,
retaining draft text/caret unless marked. Empty/unmarked Option-Up/Down traverses
Home and projects without wrap, skipping unavailable groups. Closing keeps the
host alive; ordinary reopen keeps the last available destination and draft/caret.
Capture results arriving while closed retain recovery or canonical identity and
do-not-submit-again guidance. Cmd1 opens Home, and CmdQ stops the host.
With detail closed, capture creates an unassigned new note. With a confirmed detail
open, Return appends exact submitted text to that note and immediately clears input.
The short-ID placeholder shows the accepted target for the next submission.
Home/project-All previews match the Swift desktop: <=512 raw UTF-8 bytes show
folded content; longer notes show nullable title or `Untitled note`. Empty titles
stay empty. Folding trims line edges and omits blank lines while retaining
internal spacing. Pending capture shows folded content until reconciliation;
32-point rows and canonical detail/copy remain unchanged.
The operator output gives the isolated socket and loopback MCP endpoint.
The installed daemon and desktop remain separate.

See [the spike guide](docs/embedded-gpui-spike.md) for safety, CLI connection,
fixture limits, build checks and the distinction between automated and native
validation. This package is not included in release distribution.

## Normal macOS GUI host

The GUI shares the existing daemon's normal config, session, data directory,
Unix socket and MCP endpoint (`http://127.0.0.1:37789/mcp`). Start without flags:

```bash
cargo build --locked -p flicknote-gpui -p flicknote-cli
# From clean committed source, package into a NEW output without launching:
python3 scripts/package-gpui.py --output .scratch/normal-gui-host/normal-v1-3374 \
  --signing-identity 3C696934260FDAFA30B7A1959AD9CC0D964C5BB7
```

The package directly runs the GUI binary and needs no CLI companion or wrapper.
`SOURCE.json` and `SHA256SUMS` identify exact source/tree and app hashes; `RUN.md`
gives manual launch/cutover instructions. It preserves normal config resolution:
explicit endpoint environment values override saved config, saved nonempty fields
remain, and missing fields use shared `FLICKNOTE_ENV` defaults (dev when unset).
`FLICKNOTE_MCP_PORT` remains the normal port override. No GUI environment selector
is added. A usable stored session skips email login; ownership is acquired before
authentication or session effects. A competing host rejects startup clearly.

After acceptance, manually quit the old GUI trial and stop/uninstall the managed
daemon before launching the normal app. See [the operator guide](docs/normal-gui-host.md)
for commands and rollback. Packaging changes no services or installed binaries,
launches nothing and leaves historical apps/profiles/reports intact.

Readonly detail shows short ID, wrapping real title and known project, including
archived assignments omitted from the rail. Missing metadata stays hidden.
Metadata and Markdown share the reading scroll below a fixed toolbar. Toolbar
Copy preserves exact canonical Markdown; selected-text Cmd-C copies exact plain
rendered text, retaining code indentation and trailing whitespace. Each code block
has a top-right Copy button for its whole payload, excluding fences and language.
Explicit http/https links can open the browser; images never load
network, local files or data URLs. See [the reader contract](docs/embedded-gpui-spike.md#markdown-detail-reader-3348).

Home/day and project-All/Week use the production local host's watched local
cache with Unix IPC and loopback MCP ready before the first download. Project
archival/removal falls back Home while retaining the composer. First-sync progress
uses a visible Kit bar: notes download maps to 0–90%, then holds at 90% while
other required default streams finish, and hides on applied completion. It measures SDK operations,
not remaining unique notes. Unknown totals stay indeterminate; offline/errors
keep honest cached-data status. Cached first-sync completion skips the indicator. Completed sync has no routine
connecting/downloading status or transient layout space; offline/errors remain visible.
The empty, unmarked composer offers Option-J/K to workspace bindings before an
active Chinese IME; draft/marked input retains Kit IME routing and native editing.
Text highlighting keeps Kit’s dedicated input-selection role; workbench
row selection is separate. Closed-detail capture remains global/unassigned even inside a project. Open-detail
append keeps its accepted UUID across navigation and close/reopen, with one outstanding
append per note. Optimistic Markdown overlays watched content; toolbar Copy retains
canonical stored Markdown. Failures retain submitted text without replacing new typing;
a possible-after-write error requires checking the target before resubmitting. No offline
queue or automatic mutation retry is added. Close keeps the owner alive; Quit releases it.

The experimental macOS GPUI workspace supports historical Home days and per-project
All/Week navigation. Home uses local 04:00; weeks begin Monday at local midnight.
Option-H/Left and Option-L/Right navigate with an empty, unmarked composer;
header controls offer Today/This week return and never advance into the future.
Choices remain in process memory across close/reopen. See the
[normal GUI calendar contract](docs/normal-gui-host.md#historical-days-and-project-weeks-3422)
for capture, input and verification boundaries.

Add projects with the Plus icon beside Projects in the rail and edit their summaries above the project All list.
The application menu's **Automatic organization…** control stores an account-scoped
OpenRouter key in a new GUI Keychain service. The background GUI host routes
post-first-start, unassigned ready notes from compact project/note summaries,
with bounded content fallback, manual precedence and finite retries. Closing the
window retains routing; Quit cancels it. Configuration, privacy/provider cost,
cutoff and recovery are documented in [automatic organization](docs/automatic-organization.md).
**Catch up** there offers the recent three or seven local 04:00 workdays,
including today, with a free local eligible-count preview and an explicit Start.
It freezes the end at Start, preserves the cutoff and drains without a total cap.
Regular work shares one coordinator and takes priority; shared provider batch
starts are at least ten seconds apart during Catch up. Stop and finite failure
recovery preserve successful decisions; window close continues, Quit stops without
automatic resume. Progress stays in the control. Reader drag selection freezes
on pointer release while exact plain, canonical and code-block Copy remain distinct.
**Only mine** in the right workspace header defaults off and applies to Home/Today
and project-All/Week. It excludes only `metadata.created_by_ai` JSON boolean `true`
before the watched 10,000-note limit; missing, false and other JSON values remain.
The account-scoped choice persists across destination/window changes and restart.
It describes creation channel, not authorship. Explicit-ID CLI/MCP/IPC access and
background Jev eligibility remain independent. Charts/search and headless organization remain deferred.

See [the normal GUI operator guide](docs/normal-gui-host.md) for login,
config/endpoints and recovery, and [the shortcuts audit](docs/shortcuts-audit.md)
for implemented, partial and deferred desktop mappings. Private remote PostgreSQL
MCP stays separate. Native OS input/pixels and real cloud acceptance are
unperformed; rendered tests and builds establish separate evidence. Linux and
signed distribution remain deferred. Accepted merged GUI updates are installed by
Orc to `/Applications/FlickNote.app` with verified source/hashes and rollback
retained, under standing user authorization. Installation never automatically
quits/restarts the running app or clears drafts.

## License

MIT
