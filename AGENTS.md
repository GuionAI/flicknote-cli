# FlickNote CLI

Local-first note management CLI with cloud sync via PowerSync and Supabase.

## Agent Instruction Source

- `AGENTS.md` is the project instruction source of truth for agents.
- Do not add or update `CLAUDE.md`; this repo no longer uses it.
- Keep agent-facing workflow rules here when they affect future coding,
  release, verification, or review behavior.

## Architecture

Rust workspace with 5 crates:

- **flicknote-cli** — unified `flicknote` executable: thin CLI client, foreground daemon, and explicit private remote MCP entrypoint; ordinary data commands never open SQLite or Postgres
- **flicknote-client** — Canonical pure application DTOs, wire protocol, transport errors, and async Unix socket client with an explicit endpoint
- **flicknote-core** — Database, config, schema, storage types, session, services, internal errors, and storage/Markdown-to-client DTO conversions
- **flicknote-auth** — Supabase GoTrue authentication (OTP + OAuth2/PKCE)
- **flicknote-sync** — Daemon application host, local and private remote MCP HTTP servers, IPC server, backend ownership, and PowerSync ↔ Supabase sync

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

## CI and releases

GitHub `.github/workflows/checks.yml` runs the routine suite daily at 03:17 UTC,
on manual dispatch, and as the exact-source release gate. PRs and main pushes
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
