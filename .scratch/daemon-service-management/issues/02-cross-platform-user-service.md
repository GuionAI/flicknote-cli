# 02 — Cross-platform user service lifecycle

**What to build:** Let users install and control the foreground daemon as a user-level launchd service on macOS or systemd service on Linux through one `flicknote daemon` interface.

**Blocked by:** 01 — Foreground daemon ownership and graceful shutdown.

Status: ready-for-agent

- [ ] A maintained `service-manager` adapter replaces direct platform command construction for user-level launchd and systemd lifecycle operations.
- [ ] `daemon install` installs a missing service, starts it, and reports success only after compatible IPC readiness is observed.
- [ ] Re-running `daemon install` reconciles changed service configuration or executable location, ensures the service is started, and succeeds without creating duplicate services.
- [ ] Installed services execute the same `flicknote daemon run` entry point used for foreground diagnosis.
- [ ] Installation preserves a stable package-manager symlink entry point when invoked through one rather than canonicalizing it into a versioned package directory.
- [ ] The selected executable is validated as existing, executable, and FlickNote-owned before service installation.
- [ ] `daemon start` starts only an installed service and waits for readiness; it does not install a missing service.
- [ ] `daemon stop` stops without uninstalling autostart configuration and waits until application readiness is gone.
- [ ] `daemon restart` restarts only an installed service and waits for readiness.
- [ ] `daemon uninstall` stops and removes the installed service.
- [ ] Readiness requires a compatible IPC health response after local application initialization but does not require remote PowerSync connectivity or a completed sync cycle.
- [ ] Protocol incompatibility reports CLI and daemon version/protocol details when available; exact package-version equality is not required.
- [ ] Services autostart and restart unexpected failures with reasonable platform-supported delay, while explicit shutdown and permanent startup errors do not create a tight restart loop.
- [ ] Adapter-level tests cover user-service selection, lifecycle state transitions, reconciliation, stable executable selection, readiness, and error translation without testing dependency internals.
- [ ] Bounded isolated launchd/systemd user-service system tests run where CI or the host supports them, use unique labels and data roots, and always clean up.
