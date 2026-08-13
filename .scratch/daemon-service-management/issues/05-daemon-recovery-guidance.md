# 05 — Daemon recovery guidance for CLI and MCP clients

**What to build:** Make every daemon-dependent CLI and MCP entry point fail consistently and safely when the application is unavailable, while preserving local-first operation when only remote sync is offline.

**Blocked by:** 03 — Daemon status and logs experience.

Status: ready-for-agent

- [ ] Daemon-dependent CLI commands report that the daemon is unavailable and recommend `flicknote daemon status` and `flicknote daemon start` as appropriate.
- [ ] MCP startup and daemon-dependent MCP operations expose consistent actionable unavailability diagnostics without leaking platform-specific service details.
- [ ] Data commands and MCP never install, start, restart, or otherwise mutate OS service state implicitly.
- [ ] Data commands and MCP never fall back to opening the PowerSync SQLite database directly.
- [ ] A ready local application remains usable and reports ready when PowerSync is disconnected or the network is unavailable.
- [ ] Transient remote connectivity failures stay inside PowerSync's reconnect/backoff behavior and do not churn the OS service process.
- [ ] Behavioral tests prove recovery guidance, absence of implicit service mutations, absence of direct database fallback, and local readiness during remote outage.
