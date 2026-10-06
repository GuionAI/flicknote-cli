# FlickNote CLI

Local-first note management CLI with cloud sync via PowerSync and Supabase.

## Agent Instruction Source

- `AGENTS.md` is the project instruction source of truth for agents.
- Do not add or update `CLAUDE.md`; this repo no longer uses it.
- Keep agent-facing workflow rules here when they affect future coding,
  release, verification, or review behavior.

## Architecture

Rust workspace with 5 production crates and 1 experimental package:

- **flicknote-cli** — unified `flicknote` executable: thin CLI client, foreground daemon, and explicit private remote MCP entrypoint; ordinary data commands never open SQLite or Postgres
- **flicknote-client** — Canonical pure application DTOs, wire protocol, transport errors, and async Unix socket client with an explicit endpoint
- **flicknote-core** — Database, config, schema, storage types, session, services, internal errors, and storage/Markdown-to-client DTO conversions
- **flicknote-auth** — Supabase GoTrue authentication (OTP + OAuth2/PKCE)
- **flicknote-sync** — Daemon application host, local and private remote MCP HTTP servers, IPC server, backend ownership, and PowerSync ↔ Supabase sync
- **flicknote-gpui** — macOS experimental native Today UI over embedded host/watch

Use `flicknote-client` imports for shared DTOs and protocol types. Keep backend
conversions and business algorithms in core/sync. The standalone client graph
must remain free of core/sync/auth, databases, PowerSync, MCP, and HTTP servers.
Callers resolve and pass the socket path; client construction manages no config,
credentials, directories, or services. See [client usage](flicknote-client/README.md)
for the compiled async example and failure semantics.

### MCP interface

FlickNote MCP is the formal model interface for note operations. The daemon
serves it over loopback Streamable HTTP with Origin and Host validation. MCP
handlers and CLI IPC requests share the daemon `Application` and operation
DTOs. The CLI remains for human and operational
workflows; structured section mutations remain MCP operations.

The explicit `flicknote private-mcp` foreground entrypoint uses a shared
PostgreSQL pool, a per-request full-grant verifier, transaction-local Supabase
identity, RLS, and PGroonga search. It reuses the application handlers and
DTOs, advertises only the supported remote tool subset, and never loads the
local config/session/PowerSync database. Do not route ordinary CLI commands
through PostgreSQL. See `docs/private-mcp.md` for the verifier, role, schema,
index, and operator contract. Real remote integration tests run through
`scripts/test-private-pg.sh` against an isolated PGroonga fixture provisioned
from every unchanged fb migration via dbmate. The default resolves current fb main once and logs the
commit; use `FLICKNOTE_TEST_FB_REVISION` to replay it.
Before changing database columns, SQL adapters, or PostgreSQL test setup, read
the schema ownership and verification workflow in `docs/private-mcp.md`.
Use fb's dbmate migrations as the PostgreSQL DDL authority and cross-check its
Drizzle mapping; shared development databases are read-only reference targets
for tests, not disposable fixtures.

### Machine note operation contract

Machine note operations are orthogonal: content mutation, metadata mutation,
and lifecycle mutation are separate contracts. Ordinary content and metadata
edits must preserve lifecycle status and never implicitly trigger AI processing.
`submit` is the explicit draft lifecycle transition; do not add a generic
public status setter or expose raw internal status fields through machine DTOs.
Local and remote MCP `note_add` accept content and optional project only; text
enters `ai_queued` and recognized URLs enter `source_queued`. Supplied `draft`
arguments are invalid input. Human CLI draft creation and shared application/IPC
draft support remain available. Machine reads return stored note content. The
synthesized editable document is reserved for the human `flicknote edit` workflow.

MCP creation sets `metadata.created_by_ai` to JSON boolean `true` regardless of
client or session. Human CLI/GUI creation omits it. Human-only list, count and
find exclude only boolean `true`; missing and `false` are human creation.
Recall returns human-created candidates. The marker describes the creation
channel, not content authorship; later edits, AI processing and lifecycle
changes preserve it. `note_get` exposes stored metadata; `note_add` has no
public marker parameter. Historical `created_by` strings have no runtime
meaning and receive no automatic conversion.

Every MCP structured result must have an object root, and each advertised output
schema must be precise and derived from its boundary DTO's serialized JSON
contract: fields, requiredness, JSON types, value and structural constraints,
and references. `format` annotations are intentionally omitted: client support
is nonportable, so they do not establish a client-facing validation or UI
contract; server-side validation is authoritative. Arbitrary JSON schema terms
must use object form rather than bare boolean terms. Every MCP change must pass
the repository-wide strict-client output-schema contract test.

`note_recall` is a read-only, host-triggered tool for Codex's synchronous
`UserPromptSubmit` hook. It offers bounded historical candidates by numeric
short ID; it does not read note bodies or write notes. Human and operator recall
uses `flicknote recall QUERY`; Codex command hooks use
`flicknote recall --hook`, which reads one `UserPromptSubmit` event JSON object
from stdin and emits the same bounded hook JSON contract. The CLI uses the
explicit `--project` argument; host event metadata never selects the project.
Human recall has a five-second complete daemon-call budget;
command-hook and MCP `note_recall` recall have three seconds. Empty recall,
daemon-unavailable recall, and timed-out recall supply no context. Treat a
timeout as a slow response; recommend daemon status/start only for an actually
unavailable daemon.

Install the command hook with `flicknote hook install codex [--local|--global]`.
Installation does not require an MCP registration, daemon access, or trust
changes. It writes a static shell-quoted absolute CLI command with a three-second
synchronous timeout, preserves unrelated hook configuration, and replaces or
coalesces only recognizable `command` handlers for `flicknote recall --hook` in
the selected hooks file. Old MCP `mcp_tool` hooks are ignored and preserved
unchanged; they never block command-hook installation. If an old MCP hook is
still enabled, remove it manually to avoid duplicate recall. An active command
recall entry in another scope or inline source is reported instead of
duplicated. Review and trust the result in Codex with `/hooks` (and trust the
project for a local hook). Reinstall an existing command hook explicitly after
upgrading to receive the three-second generated timeout. Hook failures are
nonblocking and must not fabricate context; use `note_get` to inspect a
candidate and verify it before any separately authorized edit. The recall query
optimization is daemon-side and requires the updated daemon to be running.


## Build & Test

`rust-toolchain.toml` is the single source for the exact Rust channel, components
and targets. Local Cargo commands, routine CI and release compilation select it
through rustup; Nix also reads it. Do not override it with floating stable or a
second version in CI/dist configuration. For a compiler upgrade, edit the file,
run the full routine suite, and regenerate/check the release workflow with the
documented generator. `rustup show active-toolchain` installs/verifies the
repository selection; an absent toolchain may require a download.

```bash
cargo build                # build all crates
cargo test                 # run all tests
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check    # format check
```

Or use the justfile: `just build`, `just test`, `just check`, `just install`

## GUI host and synthetic verification

Read `docs/normal-gui-host.md` before normal GUI packaging, launch or manual
cutover. Normal GUI reuses `Config::load()`, the existing session/data/socket,
LocalHost ownership and default MCP37789 with `FLICKNOTE_MCP_PORT` overrides.
Shared saved configuration and backend environment defaults remain unchanged.
Acquire directory ownership before GUI authentication/session effects; an
incumbent rejects startup without takeover. Window close retains the host;
explicit Quit shuts down and releases it. Remote private MCP remains separate.

Build clean committed source with `cargo build --locked -p flicknote-cli -p
flicknote-gpui` on Apple Silicon macOS. Package GUI only with
`scripts/package-gpui.py` in a NEW versioned ignored `.scratch` output; verify
source/tree/hash evidence. Normal packaging requires an explicit Apple Development
certificate SHA1 (`--signing-identity`); use the operator contract in
`docs/normal-gui-host.md`. Sign the complete new scratch bundle before manifest
hashes, verify certificate/team/identifier/designated requirement and strict
signatures, and record unsigned input separately from final signed hashes.
Reuse verified cached builds; preserve historical originals. Tests use only fake
signers/owned fixtures. Authorized artifact signing uses the confirmed identity
only on new scratch bundles; a missing/locked identity or human authentication
need must be reported without prompt automation, unlock or ACL/trust changes.
Preserve existing apps/profiles/reports. Packaging
performs no launch, installation, service changes or real-account operations.
Manual normal cutover requires separate user direction; shared CLI `--profile`
auth-only/headless support remains available and unchanged.

Tests own temporary config/data/session roots, local fake HTTP and port0
listeners. Never use normal live state, bind37789, stop an incumbent daemon,
change installed binaries/services/MCP registrations or contact cloud accounts
for verification. Read `docs/embedded-gpui-spike.md` for explicit synthetic
`--root` mode; it never loads normal config/session and requires an owned port0.
The injected creator remains synthetic; normal creation is remote-backed.

Run `cargo test --locked -p flicknote-sync --all-features
runtime::local_host_tests -- --nocapture`, `cargo test --locked -p flicknote-gpui`
and CLI tests for relevant changes. Complete the full routine and locked Mac
CLI/GUI build on final clean committed source. Native input/IME/pixels and live
cloud/cutover evidence remain separate from rendered tests/builds. Linux GUI,
login-item registration, public distribution/updater and automatic handoff are deferred.
For visual changes follow the native synthetic screenshot workflow in the spike
guide; missing native pixels leave visual acceptance unverified. #3374 requires
owned automated host/login/lifecycle/package evidence, no native launch or new
manual matrix. Send IMPL_COMPLETE with final SHA/PR/absolute scratch report;
Orc starts independent review and governs merge without another user review gate.
Accepted merged normal GUI updates are installed by Orc to
`/Applications/FlickNote.app` under standing user authorization, after independent
review/merge and source/hash/signature verification; preserve a rollback package.
Install the already signed whole bundle and reverify its seal/hashes; never sign
or patch the installed app in place. Native launch/dyld/Keychain acceptance is
a user handoff and remains unverified by static packaging checks. Worker
verification/package actions do not install. Installation never automatically
quits/restarts the running app or clears drafts. Follow `docs/normal-gui-host.md`
for the operator boundary.
See `docs/shortcuts-audit.md` before extending desktop shortcuts.
Read `docs/automatic-organization.md` before changing GUI project editors,
organization eligibility/retries, cutoff preferences or credential handling.
Organization starts only in the normal GUI host; tests inject fake secrets/provider
HTTP and own preference roots. Never exercise the real Keychain adapter in tests.

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

## CI and releases

GitHub `.github/workflows/checks.yml` runs the routine suite daily at 03:17 UTC,
on manual dispatch, and as the exact-source release gate. The check job selects
latest Python 3.14 patch through setup-python before Python/pip or routine steps;
use `python3 -m pip` with that interpreter. PRs and main pushes
have no remote routine quality checks. Run `bash scripts/check-routine.sh` for
the same formatting, locked all-feature workspace tests, locked all-target
Clippy, dependency policy and release-script fixtures locally. Existing Git
hooks remain in place. Ordinary tests skip ignored PostgreSQL integration tests.

Release checks must pass on `github.sha` before cargo-dist's plan (including
`dist host --steps=create`), public upload/release and Homebrew publication.
Regenerate with cargo-dist 0.31.0 on PATH and
`python3 scripts/generate-release-workflow.py`; verify with `--check`.
The supported `plan-jobs` hook calls the reusable suite. The small generator
adds strict success dependencies because cargo-dist's stock host condition
accepts skipped builds; `allow-dirty = ["ci"]` preserves these owned changes.
Plain `dist init`/`dist generate` is not the regeneration procedure. Check
workflow semantics after changing release dependencies or upgrading cargo-dist.

CI never runs the PG harness and requires no private fb token or network.
For columns, SQL adapters, PG setup, RLS, search, or shared database/MCP behavior
that affects PostgreSQL, read `docs/private-mcp.md`, run the isolated
migration-backed harness locally, and report its pinned fb revision and outcome
in implementation and PR evidence. Configuration alone does not establish live
GitHub scheduling, mirror freshness, or branch protection. Tests and merge do
not deploy; verify release/deployment separately when requested.

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
