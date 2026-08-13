Status: ready-for-agent

## Problem Statement

FlickNote currently manages its sync daemon with hand-written PID-file logic, direct Unix signals, manually generated launchd property lists, direct `launchctl` commands, detached child processes, socket-file cleanup, and health polling. These mechanisms do not share a reliable source of truth.

A daemon can remain alive after its PID file is removed, and a second daemon can delete the first daemon's socket and open the same PowerSync SQLite database. This has produced simultaneous database owners and repeated SQLite `BUSY` failures. The daemon also listens only for terminal interrupt (`SIGINT`), while launchd and normal service-stop operations use termination (`SIGTERM`). Consequently, service stops bypass the daemon's PowerSync disconnect and WAL checkpoint path.

The current CLI exposes these operations under `flicknote sync`, even though they manage a long-running daemon rather than request an immediate synchronization. The implementation supports launchd only and would require another custom lifecycle implementation for systemd. Users should not need to understand platform-specific service commands or repair PID/socket inconsistencies.

## Solution

Replace FlickNote's custom process and launchd lifecycle management with the maintained `service-manager` abstraction, using user-level launchd services on macOS and user-level systemd services on Linux. The operating-system service manager is the source of truth for installation and process lifecycle; FlickNote's IPC health endpoint remains the source of truth for application readiness.

Replace the `flicknote sync` command family with `flicknote daemon`. Login automatically reconciles, starts, and verifies the user service. Logout stops and uninstalls the service before removing the session and local data. A foreground-only `flicknote daemon run` entry point supports development and diagnosis without detaching or creating a second lifecycle model.

Protect each FlickNote data directory with a kernel-managed advisory exclusive file lock held for the daemon's full lifetime. The lock, rather than a PID file or socket existence, enforces single ownership of the PowerSync SQLite database. Socket cleanup is permitted only after the process acquires that lock.

Handle `SIGINT` and `SIGTERM` as inputs to one bounded graceful-shutdown sequence. Stop accepting IPC work, bound in-flight work, disconnect PowerSync, attempt a bounded WAL checkpoint, release resources, and exit. Maintenance failures are recorded but do not leave the service stuck indefinitely.

Provide concise normal output, detailed and JSON status modes, and a cross-platform logs command so users do not need to know launchd or systemd details.

## User Stories

1. As a FlickNote user, I want login to start everything required for note commands, so that I do not need a separate daemon setup step.
2. As a FlickNote user, I want logout to stop and remove the daemon service, so that no authenticated background process remains after logout.
3. As a FlickNote user, I want daemon installation to work on macOS, so that FlickNote starts automatically through my launchd user session.
4. As a FlickNote user, I want daemon installation to work on Linux, so that FlickNote starts automatically through my systemd user session.
5. As a FlickNote user, I want the same daemon commands on macOS and Linux, so that I do not need platform-specific service knowledge.
6. As a FlickNote user, I want daemon management commands to be named `daemon`, so that their purpose is clear and is not confused with an immediate sync operation.
7. As a FlickNote user, I want `daemon install` to install, start, and verify the service, so that success means FlickNote is actually usable.
8. As a FlickNote user, I want repeated `daemon install` calls to reconcile the installed service, so that I can repair configuration and executable-path changes safely.
9. As a FlickNote user, I want `daemon start` to start an installed service without silently installing one, so that command side effects remain predictable.
10. As a FlickNote user, I want `daemon stop` to stop the service without uninstalling autostart configuration, so that I can pause FlickNote temporarily.
11. As a FlickNote user, I want `daemon restart` to restart an installed service and wait for readiness, so that I have a reliable recovery command.
12. As a FlickNote user, I want `daemon uninstall` to stop and remove the service, so that it will not return on my next login.
13. As a developer, I want `daemon run` to run synchronously in the foreground, so that I can observe logs and stop it with my terminal.
14. As a developer, I want foreground daemon execution to avoid forking, detaching, or creating a session, so that its lifetime remains attached to my shell.
15. As a developer, I want both Ctrl-C and normal service termination to use the same shutdown path, so that foreground and managed execution behave consistently.
16. As a FlickNote user, I want only one daemon to own a data directory, so that concurrent processes cannot corrupt or starve the SQLite workload.
17. As a FlickNote user, I want a crashed or forcibly killed daemon to release single-instance ownership automatically, so that stale metadata does not block recovery.
18. As a developer, I want separate XDG data directories to permit separate daemon instances, so that isolated development and testing environments remain possible.
19. As a FlickNote user, I want a clear error when another daemon owns my data directory, so that I know why startup was refused.
20. As a FlickNote user, I want lock-conflict errors to show safe diagnostic metadata when available, so that I can identify the owner without FlickNote automatically killing it.
21. As a FlickNote user, I want `daemon run` to tell me how to stop an installed daemon when ownership conflicts, so that I can switch to foreground diagnosis safely.
22. As a FlickNote user, I want daemon startup success to require a valid IPC health response, so that a merely running but unusable process is not reported as ready.
23. As a FlickNote user, I want network disconnection not to prevent daemon readiness, so that local-first operations remain available offline.
24. As a FlickNote user, I want protocol incompatibility between the CLI and daemon to produce a clear error, so that I do not unknowingly use an incompatible process.
25. As a FlickNote user, I want compatible CLI and daemon package versions to interoperate even when their version strings differ, so that protocol compatibility—not incidental version equality—governs operation.
26. As a FlickNote user, I want `daemon status` to provide a concise healthy summary, so that routine checks are easy to read.
27. As a FlickNote user, I want `daemon status --verbose` to distinguish service state, application readiness, version, protocol, and sync state, so that failures can be diagnosed.
28. As an automation author, I want `daemon status --json` to return a stable object, so that scripts can inspect daemon state without parsing human text.
29. As an automation author, I want status to return a nonzero exit status when the application is not ready, so that health checks can fail reliably.
30. As a FlickNote user, I want `daemon logs` to show recent daemon logs on both macOS and Linux, so that I do not need to learn launchd and journal commands.
31. As a FlickNote user, I want `daemon logs --follow` to stream logs, so that I can observe startup and synchronization failures in real time.
32. As a FlickNote user, I want to choose the number of recent log lines, so that diagnostics remain bounded.
33. As a FlickNote user, I want a failed data command to recommend `daemon status` and `daemon start`, so that recovery is obvious.
34. As a FlickNote user, I want data commands never to install or start services implicitly, so that read and mutation commands do not change OS service state.
35. As a FlickNote user, I want data commands and the MCP server never to fall back to opening SQLite directly, so that the daemon remains the sole database owner.
36. As a FlickNote user, I want successful login to report authentication and daemon readiness concisely, so that I know FlickNote is usable.
37. As a FlickNote user, I want authentication to remain valid if daemon installation fails, so that I can retry service setup without logging in again.
38. As a FlickNote user, I want login to return a failure when authentication succeeds but daemon readiness fails, so that partial setup is not presented as complete success.
39. As a FlickNote user, I want `login --force` to stop the old service before replacing my session, so that the daemon never continues with superseded credentials.
40. As a FlickNote user, I want successful forced login to reinstall and verify the service, so that reauthentication restores the complete system.
41. As a FlickNote user, I want a failed forced authentication to leave me clearly logged out, so that an old session is not silently resurrected.
42. As a FlickNote user, I want normal logout to preserve my session if service shutdown or uninstall fails, so that a running daemon is not left with credentials removed underneath it.
43. As a FlickNote user, I want `logout --force` to let me clear the session and local data despite service cleanup failure, so that I retain an explicit emergency escape hatch.
44. As a FlickNote user, I want daemon installation and foreground execution to require a valid login, so that an unauthenticated background process is not created.
45. As a FlickNote user, I want transient network errors to be retried inside the running daemon, so that the OS service manager does not churn the process during offline periods.
46. As a FlickNote user, I want daemon crashes and unexpected actor exits to trigger OS-managed restart, so that the service recovers from transient internal failures.
47. As a FlickNote user, I want permanent configuration and authentication errors not to cause an infinite restart loop, so that logs and system resources are not flooded.
48. As a FlickNote user, I want service restart attempts to use a reasonable delay, so that repeated crashes do not create a tight loop.
49. As a FlickNote user, I want graceful shutdown to have a hard upper bound, so that stop, logout, restart, and upgrade operations cannot hang indefinitely.
50. As a FlickNote user, I want PowerSync disconnect to be attempted during shutdown, so that synchronization actors can stop cleanly.
51. As a FlickNote user, I want a shutdown WAL checkpoint to be attempted but not required for exit, so that normal SQLite WAL durability does not become a shutdown deadlock.
52. As a FlickNote user, I want shutdown logs to identify the stage that timed out or failed, so that PowerSync and SQLite issues can be distinguished.
53. As a Homebrew user, I want the service to reference a stable executable entry point, so that package upgrades do not leave launchd pointing into a removed Cellar version.
54. As a FlickNote user, I want the managed service and foreground command to execute the same daemon entry point, so that version and behavior cannot drift between binaries.
55. As a FlickNote maintainer, I want one distributed executable rather than separate CLI and daemon executables, so that packaging and upgrades have a single source of truth.
56. As a FlickNote maintainer, I want OS service state and application readiness to remain separate concepts, so that diagnostics accurately describe partial failures.
57. As a FlickNote maintainer, I want PID information to be diagnostic only, so that PID reuse and stale files cannot control correctness or automatic signaling.
58. As a FlickNote maintainer, I want socket cleanup to occur only while holding the data-directory lock, so that one daemon cannot unlink another live daemon's endpoint.
59. As a FlickNote maintainer, I want the service-manager adapter to own platform differences, so that FlickNote does not hand-generate launchd or systemd configuration.
60. As a FlickNote maintainer, I want observable lifecycle behavior covered at a high integration seam, so that replacing internal libraries does not invalidate the tests.

## Implementation Decisions

- Replace direct launchd commands and hand-written service files with the maintained `service-manager` crate. Configure user-level launchd on macOS and user-level systemd on Linux. Windows, OpenRC, rc.d, and system-level services are not promised by this feature.
- The OS service manager is the source of truth for whether a service is installed, started, stopped, or uninstalled. FlickNote must not use a PID file to infer or control managed service state.
- IPC health is the source of truth for application readiness. A service can be running while the application is unavailable; status and error messages must preserve that distinction.
- Rename the public command namespace from `sync` to `daemon`. Do not retain a compatibility alias.
- The public command family is `install`, `uninstall`, `start`, `stop`, `restart`, `status`, `logs`, and `run`.
- `install` is an idempotent reconciliation operation. It installs a missing service, updates changed service configuration or executable paths, ensures the service is started, and waits for IPC readiness. Re-running it with an equivalent configuration is successful.
- `start`, `stop`, `restart`, and `uninstall` operate only through the service manager. `start` and `restart` do not silently install a missing service.
- `run` executes the daemon synchronously in the foreground. It does not fork, detach, invoke `setsid`, redirect terminal output, or create an alternate background mode.
- Login and daemon lifecycle are symmetric. A successful login reconciles and starts the user service and waits for readiness. Logout stops and uninstalls the service before deleting the session and local database files.
- If authentication succeeds but service installation or readiness fails, retain the valid session, print that authentication succeeded and daemon startup failed, provide the status recovery command, and return nonzero.
- Forced login stops and uninstalls the current service before removing the prior session. It then authenticates, reconciles the service, and waits for readiness. Failed authentication does not restore the old session or service.
- Normal logout aborts before deleting session or local data when service stop/uninstall cannot be confirmed. Add an explicit force option that allows cleanup to continue despite that failure and clearly reports the unresolved service state.
- Daemon installation and foreground execution require a valid session. There is no supported unauthenticated daemon state.
- Data commands and MCP operations continue to require daemon health. They never open SQLite directly and never implicitly install or start the service.
- Use a kernel-managed, non-blocking advisory exclusive file lock to protect each configured data directory. Select a maintained Rust lock crate after checking current documentation and types rather than implementing raw `flock`/`fcntl` handling.
- Store the lock file in the configured FlickNote data directory under daemon terminology. Hold the open lock guard from before any socket or database ownership is acquired until daemon teardown completes. Process exit, panic, or forced termination must release the kernel lock automatically.
- Lock-file contents may contain PID, version, and start time for diagnostics. That content is not authoritative, may be stale, and must never be used to choose or signal a process.
- After obtaining the data-directory lock, the daemon may remove a stale daemon socket and bind the shared endpoint. It must never unlink the endpoint before lock ownership is established.
- Standardize lifecycle terminology on daemon. Use a daemon service label, daemon lock name, and daemon socket name. Do not preserve sync-named lifecycle artifacts for compatibility.
- Use one public `flicknote` executable. The managed service runs the same `daemon run` entry point used for foreground execution. Remove the separately distributed daemon executable and its sibling-path discovery logic.
- Preserve a stable symlink path when installation is invoked through one, rather than canonicalizing it into a versioned package-manager location. Validate that the selected executable exists, is executable, and identifies as FlickNote before installing the service.
- The release and package configuration distributes only the unified executable.
- Handle terminal interrupt and Unix termination as equivalent shutdown triggers feeding one shutdown coordinator. Platform-specific signal registration remains internal to the daemon runtime.
- Graceful shutdown uses an approximately eight-second total budget and logs stage boundaries and durations. Stop accepting new IPC immediately, allow up to approximately two seconds for in-flight IPC, allow up to approximately four seconds for PowerSync disconnect, and allow up to approximately two seconds for a shutdown WAL truncate checkpoint. Remaining cleanup releases the socket and lock as guards drop.
- A PowerSync disconnect timeout or checkpoint timeout/failure is logged and does not prevent process exit. SQLite WAL already provides durability; successful truncation is maintenance, not a correctness precondition.
- Unexpected internal actor exit, panic, and transient internal crashes produce failure semantics suitable for OS-managed restart. Explicit shutdown exits successfully.
- Permanent startup errors such as missing authentication, invalid configuration, or incompatible schema are classified separately so they do not create an unbounded restart loop. Network unavailability is not a startup failure; PowerSync retains its internal reconnect/backoff behavior while the daemon remains ready for local work.
- Configure autostart and restart-on-failure with a reasonable platform-supported delay. Explicit service stop must not cause immediate restart.
- A successful readiness check requires a valid IPC server-info response after configuration validation, database opening, backend creation, and socket serving. It does not require remote PowerSync connection or completion of a sync cycle.
- CLI/daemon compatibility is governed by the IPC protocol contract, not exact package-version equality. Status includes both values. Incompatible protocol responses fail readiness and report the executable path, CLI version/protocol, and daemon version/protocol when available.
- Default status output is one concise line. Unhealthy states include an actionable recovery command automatically.
- Verbose status distinguishes service installation/running state, application readiness, package version, IPC protocol, PowerSync connection state, last observed error, and the platform-appropriate log location or command.
- JSON status has an object root with stable fields for service state, application state, version, protocol, sync state, error details, and log guidance. Use explicit string enums for state fields and nullable object/string fields for unavailable observations; do not encode unavailable states by omitting unrelated required fields unpredictably.
- Status emits its human or JSON report even when unhealthy, then returns nonzero unless the application is ready.
- Logs provides bounded recent output by default, accepts a line-count option, and supports follow mode. On macOS it reads/follows launchd-directed stdout/stderr in the FlickNote data directory. On Linux it queries/follows the systemd user journal. Platform adaptation is internal to the command.
- Foreground run logs to the attached terminal. It does not redirect output to the managed-service log destination.
- Login success prints `Authenticated` followed by a concise daemon-ready confirmation. Partial success states identify the failed stage without exposing launchd/systemd implementation details in the normal path.
- When foreground run cannot acquire ownership because the managed daemon is active, the error tells the user to stop the daemon and retry. It does not offer or perform an automatic stop/kill option.
- Service manager errors retain enough platform context for verbose diagnostics but normal errors use FlickNote concepts and actionable `flicknote daemon` commands.
- There is no runtime migration or compatibility layer for old PID files, sync commands, sync socket names, old service labels, or manually detached daemons. Before installing the release containing this feature, development/release documentation instructs existing installations to run the old version's uninstall command.
- The temporary PID/SIGKILL stopgap on the current development branch is superseded by this design; the final implementation must remove custom PID signaling rather than layering service-manager behavior on top of it.

## Testing Decisions

- Tests assert observable lifecycle contracts rather than source shape, command strings, selected crate internals, generated plist/unit text, or private helper structure.
- Prefer two high seams because they cover distinct trust boundaries without duplicating low-level tests.
- The primary seam is the CLI lifecycle orchestration boundary with an injected/fake service-manager adapter and fake health endpoint. Exercise daemon install/start/stop/restart/uninstall/status behavior and login/logout composition through the same command orchestration used by the CLI. Assert calls only where they are externally meaningful state transitions; assert user-visible output, exit outcome, retained/deleted session state, and readiness behavior.
- The second seam is an isolated daemon process integration test using temporary XDG config/data roots and the real daemon entry point. It verifies exclusive ownership, stale socket handling under lock, SIGINT shutdown, SIGTERM shutdown, bounded exit, resource release, and subsequent restart. It must never touch the user's real service, session, socket, database, or logs.
- Extend the existing CLI parser test style to mechanically confirm every `daemon` subcommand and option parses and the removed `sync` namespace does not parse. This is a public CLI contract rather than a source-text test.
- Add machine-consumed contract tests for status JSON: object root, stable required fields, allowed state enums, healthy and unhealthy examples, and protocol/version representation.
- Test status behavior across at least: not installed, installed/stopped, service running/application unavailable, ready/offline, ready/connected, protocol incompatible, and service-manager query failure.
- Test that default status is concise, verbose status distinguishes service and application state, unhealthy status still emits diagnostics, and unhealthy status exits nonzero.
- Test install reconciliation across missing, equivalent, and changed service configurations. The changed configuration case must prove the current stable executable entry point is applied and the service becomes ready.
- Test that start/restart fail clearly when the service is not installed and do not install it implicitly.
- Test that data commands and MCP startup report daemon recovery guidance without attempting service installation, service start, or direct database access.
- Test login orchestration for full success, authentication failure, authentication success plus install failure, and authentication success plus readiness failure. Verify valid sessions are retained in partial-success cases.
- Test forced login ordering and outcomes: old service cleanup precedes old-session removal; failed new authentication leaves no restored old session; successful authentication reconciles and verifies the new service.
- Test logout success, stop failure, uninstall failure, and forced cleanup. Normal failure preserves session/local data; forced cleanup removes them while reporting unresolved service cleanup.
- Test foreground run rejects missing authentication before acquiring database ownership.
- Test two foreground daemon processes against one temporary data directory. The second must fail promptly without deleting the first process's socket or opening its SQLite database. After the first exits or is forcibly killed, another process must acquire ownership successfully without manual lock-file deletion.
- Test that different temporary data directories can run concurrently.
- Test lock diagnostic metadata only for user-visible diagnostics; do not test its exact serialized layout unless that layout is explicitly exposed as a machine-consumed contract.
- Test SIGINT and SIGTERM against the real isolated process. Both must enter the same shutdown sequence, release the socket and lock, and exit within the configured total budget. Capture stage logs to distinguish entering shutdown from default signal termination.
- Test a controllably stalled PowerSync disconnect and checkpoint at the runtime orchestration seam. Each timeout must allow later cleanup and process exit. Do not require real network timing or real SQLite lock contention to make these deterministic.
- Test that remote network failure does not make local application readiness fail and does not terminate the daemon.
- Test service restart classification through adapter-visible outcomes: explicit shutdown is successful; unexpected actor failure is unsuccessful; permanent startup errors do not request an endless retry path.
- Add platform adapter tests only for behavior not already guaranteed by `service-manager`, such as choosing user-service level, stable executable input, logging destination/guidance, and translating status into FlickNote's status model. Do not duplicate the dependency's launchd/systemd command-generation tests.
- Add bounded macOS and Linux system tests where CI runners permit them: install a uniquely labeled temporary user service, start it, observe readiness, stop it, and uninstall it. These tests must clean up through guards even on failure and must not use the production label or production data directory.
- Follow existing project conventions: Rust unit/integration tests, temporary directories, explicit XDG isolation, targeted package tests during iteration, then workspace format, test, check, and Clippy with warnings denied.
- The current parser tests are prior art for public CLI syntax. Existing daemon health tests are prior art for IPC readiness. Existing PowerSync actor tests are prior art for real connect/disconnect behavior. Existing temporary-directory configuration tests are prior art for XDG isolation.

## Out of Scope

- Supporting Windows Services, WinSW, OpenRC, rc.d, or system-level/root services.
- Preserving the `flicknote sync` command as an alias.
- Migrating or automatically cleaning old PID files, old sync sockets, old service labels, old plist files, or detached legacy processes.
- Automatically scanning for or killing processes by name or diagnostic PID metadata.
- Maintaining a standalone background-detach mode outside launchd/systemd.
- Allowing multiple daemons to share one data directory or SQLite database.
- Replacing the Unix-domain IPC protocol or changing note/MCP business operations.
- Requiring remote PowerSync connectivity before the daemon is ready.
- Guaranteeing that a shutdown WAL truncate checkpoint succeeds.
- Making exact package-version equality an IPC compatibility requirement.
- Adding direct SQLite fallback paths to CLI or MCP clients.
- Deploying, releasing, or migrating existing installations as part of implementation; release documentation only records the required pre-upgrade uninstall step.

## Further Notes

- Production evidence showed two daemon processes simultaneously holding the same SQLite database and separate Unix socket objects associated with the same socket path. Repeated SQLite `BUSY` errors occurred during this period. This supports treating single database ownership as a correctness requirement.
- Investigation disproved the initial assumption that PowerSync shutdown was known to hang. The existing daemon listens only through Tokio's Ctrl-C API, which maps to `SIGINT` on Unix, while the controller sends `SIGTERM`. An isolated probe showed `SIGTERM` exited without entering FlickNote shutdown, whereas `SIGINT` entered shutdown and completed PowerSync disconnect quickly. The implementation should still retain bounded shutdown stages because external dependencies must not be allowed to hang service operations.
- The repository currently has no domain glossary, context document, or architecture decision record for this area. This specification uses the established project terms daemon, user service, application readiness, IPC health, PowerSync database, and local backend.
- The repository currently distributes both `flicknote` and `flicknote-sync`; this specification intentionally collapses them into one executable and requires release metadata and documentation to follow that decision.
- The implementation agent should verify the current `service-manager` and advisory-lock crate documentation and types before adding dependencies. The architectural contract is fixed; exact dependency API usage is not.
