# 03 — Daemon status and logs experience

**What to build:** Give users concise routine status, detailed and machine-readable diagnosis, and one logs interface that hides launchd/systemd differences.

**Blocked by:** 02 — Cross-platform user service lifecycle.

Status: ready-for-agent

- [ ] Default `daemon status` output is one concise line for a ready daemon and includes an actionable recovery command automatically when unhealthy.
- [ ] `daemon status --verbose` separately reports service installation/running state, application readiness, FlickNote version, IPC protocol, PowerSync connection state, last observed error, and log guidance.
- [ ] `daemon status --json` always emits an object-root result with stable required fields and explicit enums for service, application, and sync states.
- [ ] JSON status represents unavailable observations predictably rather than silently changing the result shape.
- [ ] Status distinguishes at least not installed, installed/stopped, service running/application unavailable, ready/offline, ready/connected, protocol incompatible, and service-manager query failure.
- [ ] Unhealthy status still prints the requested human or JSON diagnosis and then exits nonzero; ready status exits successfully.
- [ ] `daemon logs` shows bounded recent managed-daemon logs on macOS and Linux without requiring users to know launchd or journal commands.
- [ ] `daemon logs --lines` controls the bounded history and `daemon logs --follow` streams new output.
- [ ] Managed macOS logs use the FlickNote data-directory log destination; managed Linux logs use the systemd user journal; foreground run continues writing to its terminal.
- [ ] Contract tests cover status JSON schema and enums, while behavioral tests cover human output, exit outcomes, service/application disagreement, and both logging backends.
