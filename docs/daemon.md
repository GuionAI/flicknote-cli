# FlickNote daemon

FlickNote has one distributed executable. The foreground entry point and the
managed user service both run:

```bash
flicknote daemon run
```

The daemon owns the local PowerSync SQLite database, its Unix IPC socket, and
the Streamable HTTP MCP endpoint at `http://127.0.0.1:37789/mcp`. CLI data
commands use IPC; MCP handlers call the same daemon application directly.
Neither entry point starts a service implicitly or opens the database outside
the daemon.
`FLICKNOTE_MCP_PORT` changes only the MCP loopback port when multiple local
daemon instances must run; Codex's configured URL must use that port.

## Rust client ownership

`flicknote-client` owns the canonical pure application DTOs, versioned wire
envelopes, typed result extraction, and async Unix socket transport. The CLI
resolves its configured data directory and passes `data_dir/daemon.sock` to
`DaemonClient::new`. Other Rust callers supply their own explicit socket path;
the client neither loads configuration/credentials nor creates directories or
starts a daemon. See the [async usage example](../flicknote-client/README.md),
which is compiled as a Rust documentation test.

`flicknote-sync::ipc` owns the socket server and daemon runtime diagnostics.
IPC and MCP still dispatch through the same `Application`; core/sync own all
database access, sync, parsing, business rules, and internal-to-wire conversions.
The standalone client has no database, PowerSync, auth, MCP, or HTTP server
dependencies. User/operator service commands and the existing wire/JSON
contracts remain unchanged; this refactor adds no desktop application.

## Search index

PowerSync/SQLite remains the canonical note store. Before accepting IPC
requests, the daemon registers better-trigram and prepares an FTS5 index on
PowerSync's pinned `ps_data__notes` backing table. SQLite triggers update the
index in the same transaction as local writes and remote downloads. Existing
indexes are reused on restart; missing or invalid indexes are rebuilt from
canonical rows. Search startup errors prevent the daemon from advertising
readiness.

Lexical `find` queries use FTS5 candidate retrieval, title/summary/content
coverage ranking (3/2/1), and one set-based snippet query. Lexical hits contain
a segmented snippet; structured-only hits may have empty snippet segments.
There is no per-hit note lookup. The final Latin term uses prefix matching from
two characters; a single Latin character alone returns no results. CJK
characters use indexed tokens. Project filtering stays in this path.
Extraction-only queries use canonical SQLite filtering; lexical terms cannot
be combined with extraction or archived filters. The CLI
and MCP return the same dedicated search result shape. Daemon logs identify
`fts` or `structured` routing without recording the query or note content.
`note_count` stays on canonical SQLite and accepts only project, type, and
archived filters; it does not count lexical matches.

The FTS index follows the local database's ownership and does not apply a
separate `user_id` filter. Reusing one database across accounts is outside this
search contract. `flicknote logout` removes the local database; `login --force`
retains it. Lexical search also does not combine with extraction filters or
search archived notes; use extraction-only search for archived filtering.

The local FTS projection carries an explicit `FTS_SCHEMA_VERSION`. Startup
discards and rebuilds it when the version differs or a required maintenance
trigger is missing, even if the index and source have equal row counts.

## Authentication symmetry

Login establishes a usable local installation:

```bash
flicknote login --email you@example.com
```

After authentication, login reconciles the user service, starts it, and waits
for a compatible IPC health response. If service setup fails, the valid session
is retained. Inspect and retry with:

```bash
flicknote daemon status --verbose
flicknote daemon install
```

Logout removes the service before deleting credentials and local database files:

```bash
flicknote logout
```

If cleanup cannot be confirmed, normal logout preserves the session and local
data. `flicknote logout --force` is the explicit emergency option; it clears
local state while reporting the unresolved service cleanup.

`flicknote login --force` stops and uninstalls the existing service before
removing the old session. A failed forced authentication does not restore the
old session or service.

After upgrading an existing dev installation for the cnsupa cutover, run
`flicknote login --force` once before normal sync. The forced login then
authenticates with the opaque publishable key and installs, starts, and verifies
the daemon through the existing lifecycle. It does not automatically delete the
local PowerSync database or perform a release or deployment.

## Service commands

The same commands select a user-level launchd service on macOS and a user-level
systemd service on Linux:

```text
flicknote daemon install    # reconcile, start, and verify
flicknote daemon start      # start an installed service only
flicknote daemon stop       # stop without uninstalling
flicknote daemon restart    # restart an installed service only
flicknote daemon uninstall  # stop and remove the service
flicknote daemon status
flicknote daemon logs --lines 100
flicknote daemon logs --follow
flicknote daemon run        # attached foreground diagnosis
```

Installation validates the invoked FlickNote executable and preserves its
package-manager entry-point path. Unexpected process failures are left to the
OS service manager's restart policy; explicit stops and permanent startup
errors are not treated as successful restart events.

## Diagnosis and recovery

Routine status is one line. Use verbose output to distinguish the service from
the local application and remote sync:

```bash
flicknote daemon status --verbose
flicknote daemon status --json
flicknote daemon logs
```

A ready local application can report `offline` PowerSync state. Remote network
failure does not make local IPC unavailable. If a data command or MCP startup
reports an unavailable daemon, use `flicknote daemon status` and then
`flicknote daemon start`; no data command or MCP operation changes service state.

`daemon run` remains attached to the terminal, writes logs to the terminal, and
handles both Ctrl-C (`SIGINT`) and service termination (`SIGTERM`) through the
same bounded shutdown coordinator. Only one daemon can own a configured data
directory at a time. The kernel lock is released automatically after a crash or
forced process termination, so no lock-file deletion is required.

On macOS managed logs are stored in the FlickNote data directory. On Linux
managed logs are available through the systemd user journal. `daemon logs`
hides that platform difference.

## Upgrade boundary

This release does not migrate old lifecycle artifacts. Before installing a
release containing the unified daemon lifecycle, run the old version's cleanup
command while it is still installed:

```bash
# Run with the old FlickNote executable:
flicknote sync uninstall
```

Then install the new release and run `flicknote daemon install` (or log in).
There is no compatibility alias for `flicknote sync`, no automatic PID/socket
migration, and no cleanup of detached legacy processes.
