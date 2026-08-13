# 04 — Authentication-owned daemon lifecycle

**What to build:** Make login establish a ready authenticated daemon and make logout remove it before credentials and local data are cleared, with explicit behavior for partial failures.

**Blocked by:** 02 — Cross-platform user service lifecycle.

Status: ready-for-agent

- [ ] Successful login authenticates, reconciles the user service, starts it, waits for readiness, and prints concise authentication and daemon-ready confirmations.
- [ ] If authentication succeeds but service installation or readiness fails, the valid session is retained, the command identifies the failed daemon stage, recommends `daemon status --verbose`, and exits nonzero.
- [ ] `login --force` stops and uninstalls the current service before removing the prior session and beginning new authentication.
- [ ] Successful forced login reconciles and verifies a service using the new session.
- [ ] Failed forced authentication does not restore the old session or old service and leaves a clear logged-out state.
- [ ] Normal logout stops and uninstalls the service and confirms it is stopped before deleting the session and local database files.
- [ ] Normal logout preserves the session and local data if service stop or uninstall cannot be confirmed.
- [ ] `logout --force` explicitly permits session and local-data cleanup after service cleanup failure and reports the unresolved service state.
- [ ] Daemon installation and foreground execution reject missing authentication rather than creating an unauthenticated persistent process.
- [ ] Lifecycle orchestration tests cover login success, authentication failure, post-auth install/readiness failure, forced-login ordering and outcomes, logout success, cleanup failures, and forced logout.
