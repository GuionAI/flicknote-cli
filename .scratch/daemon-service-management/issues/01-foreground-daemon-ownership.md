# 01 — Foreground daemon ownership and graceful shutdown

**What to build:** Make `flicknote daemon run` the reliable foreground daemon entry point, with exclusive ownership of its configured data directory and one bounded shutdown path for terminal and service signals.

**Blocked by:** None — can start immediately.

Status: ready-for-agent

- [ ] `flicknote daemon run` requires a valid login and runs synchronously without forking, detaching, creating a new session, or redirecting terminal output.
- [ ] The daemon obtains a non-blocking advisory exclusive lock for its configured data directory before touching its socket or PowerSync SQLite database, and holds the lock for its full lifetime.
- [ ] A second daemon for the same data directory fails promptly with actionable ownership diagnostics and does not unlink the active daemon's socket or open its database.
- [ ] Daemons using different isolated data directories can run concurrently.
- [ ] Lock ownership is released automatically after graceful exit, panic, or forced process termination without requiring lock-file deletion.
- [ ] Socket cleanup and binding occur only after lock acquisition, so stale endpoints can be cleaned without racing a live owner.
- [ ] SIGINT and SIGTERM enter the same graceful-shutdown coordinator instead of relying on default process termination.
- [ ] Shutdown stops new IPC work, bounds in-flight IPC, bounds PowerSync disconnect, attempts a bounded WAL truncate checkpoint, releases resources, and exits within an approximately eight-second total budget.
- [ ] A stalled disconnect or checkpoint is logged by stage but cannot prevent process exit.
- [ ] Explicit shutdown exits successfully; unexpected actor termination or panic remains distinguishable as process failure.
- [ ] Isolated real-process tests prove single ownership, stale socket handling, SIGINT/SIGTERM cleanup, bounded shutdown, forced-release recovery, and subsequent restart without touching user data or services.
