# 06 — Contract legacy lifecycle and unify distribution

**What to build:** Finish the daemon-management replacement by removing every legacy lifecycle path, shipping one executable and one vocabulary, documenting the clean pre-upgrade boundary, and verifying the complete behavior in one pull request.

**Blocked by:** 01 — Foreground daemon ownership and graceful shutdown; 02 — Cross-platform user service lifecycle; 03 — Daemon status and logs experience; 04 — Authentication-owned daemon lifecycle; 05 — Daemon recovery guidance for CLI and MCP clients.

Status: ready-for-agent

- [ ] The `sync` command namespace is removed without a compatibility alias, and parser tests accept every agreed `daemon` command and option while rejecting the removed namespace.
- [ ] Custom PID-file lifecycle decisions, PID signaling, process-name scanning, SIGKILL stopgaps, detached background startup, direct launchctl handling, and hand-generated service files are removed.
- [ ] Lifecycle artifacts consistently use daemon terminology for service label, lock, and socket; old sync-named artifacts are not retained as runtime compatibility paths.
- [ ] The separately distributed daemon executable and sibling-binary discovery are removed; release metadata ships only the unified `flicknote` executable.
- [ ] No CLI or MCP code path opens SQLite directly, and the daemon remains the sole local backend and PowerSync database owner.
- [ ] User documentation covers login/logout symmetry, every `daemon` command, foreground diagnosis, status/logs usage, macOS and Linux user services, and recovery guidance.
- [ ] Upgrade documentation instructs installations using the old lifecycle to run the old version's uninstall command before installing this release; no automatic migration is added.
- [ ] Agent-facing workflow/reference documentation uses the new daemon vocabulary and commands where future implementation or verification depends on them.
- [ ] The final diff contains no temporary debug instrumentation or obsolete lifecycle tests, and replacement tests assert runtime/public contracts rather than deleted source shape.
- [ ] Workspace formatting, tests, checks, Clippy with warnings denied, strict MCP schema contracts, and the smallest available macOS/Linux service behavioral probes all pass.
- [ ] The staged final diff is reviewed as one coherent feature intended for a single pull request.
