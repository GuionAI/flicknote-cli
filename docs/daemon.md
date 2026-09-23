# FlickNote daemon

FlickNote has one distributed executable. The foreground entry point and the
managed user service both run:

```bash
flicknote daemon run
```

The daemon owns the local PowerSync SQLite database and its Unix IPC socket.
Data commands and MCP use IPC only; they never start a service implicitly or
open the database directly.

## Search projection

PowerSync/SQLite remains the canonical note store. The daemon owns a disposable
Meilisearch projection for unfiltered keyword `find` queries up to 1,000 hits,
maintained from PowerSync's reactive note-table watcher. Its initial snapshot rebuilds the
index; later snapshots update changed notes and remove archived notes. Other
find shapes (project, archive, or extraction filters) retain the canonical
SQLite path. All Meilisearch hits are hydrated from SQLite before returning
the existing CLI/MCP `find` DTO, so clients do not depend on Meilisearch.

The daemon starts Meilisearch as a foreground child bound to `127.0.0.1:7702`.
`FLICKNOTE_MEILI_PORT` overrides that port; invalid values degrade search to
SQLite. The child uses private state below FlickNote's data directory, is
terminated and waited for on normal daemon shutdown, and has no separate
service installation. The generated Homebrew formula installs Meilisearch as
a runtime dependency; source installs without it continue to work with SQLite
search. A failed or rebuilding projection never invalidates canonical note
operations. `daemon status --verbose` reports search readiness, and daemon
logs explain projection failures and fallback. Transient projection failures
retry with bounded backoff and rebuild from the canonical snapshot.

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
