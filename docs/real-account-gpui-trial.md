# Independent real-account GPUI trial

Spec #3279 adds an experimental macOS Today window over the production local
host, with English email-code login. It is separate from installed FlickNote
services and is not part of release distribution. The accepted Today appearance,
32-point rows, 520-point detail, composer and local keyboard behavior remain.

## Build and opt-in launch

Use Apple Silicon macOS and the repository toolchain:

```bash
cargo build --locked -p flicknote-gpui -p flicknote-spike -p flicknote-cli
# Choose a new absolute independent profile, never a normal FlickNote root:
PROFILE=/tmp/flicknote-real-account-trial
FLICKNOTE_ENV=prod target/debug/flicknote-gpui --profile "$PROFILE" --mcp-port 0
```

Launching this real mode and signing in are manual opt-in cloud operations.
Automated verification uses temporary sessions and local HTTP fakes. The Worker
has not sent real emails, logged into a cloud account, mutated cloud notes or
launched this artifact in a native OS window. Rendered GPUI tests and successful
builds do not establish native IME candidate-window behavior or cloud sync.
Linux builds/runtime and signed distribution remain deferred.

A versioned scratch app may be supplied with `SOURCE.json`, `SHA256SUMS` and
`RUN.md`; follow its exact source-bound profile instructions. Its wrapper sets
`FLICKNOTE_ENV=prod` and an independent profile with an allocated local MCP port.
It does not replace installed executables or register services/MCP. Preserve
prior trials unless the user separately authorizes their removal.

## Email login and ownership

When the profile has no usable session, the GUI displays an English Kit login
pane. Enter an email, choose Send code, then enter the received code and choose
Verify. Change email and Resend code are explicit actions. Invalid/expired code
or network failure retains the email and presents an error. Requests run in the
background, are bounded to 30 seconds, and disable duplicate submission while
pending. Closing the login window cancels its request; reopening provides a
fresh pane. Quit cancels startup and prevents a stale response from starting a
host. No OTP or token is printed.

Successful verification persists the selected profile session and opens Today
on the same production Application, PowerSync backend, creator, IPC and local
MCP host used by foreground daemon mode. A usable stored identity/access/refresh
session skips the pane; token refresh and sync run in the background. An active
owner rejects another GUI, headless host or authentication attempt before
session replacement or authentication effects. Authentication cannot switch the
identity underneath a running host. For an unrecoverable authentication error,
Quit first, authenticate again, then restart.

The retained CLI alternative uses the existing interactive email OTP flow:

```bash
FLICKNOTE_ENV=prod target/debug/flicknote --profile "$PROFILE" login --auth-only
# Existing browser providers are optional headless/operator alternatives:
FLICKNOTE_ENV=prod target/debug/flicknote --profile "$PROFILE" login --auth-only --provider google
```

Apple is also supported by the existing CLI provider flow. Auth-only login calls
no daemon service, including with `--force`; force is allowed only after the
owner has quit. Ordinary login/logout and daemon install/uninstall/start/stop/
restart/status/logs reject with `--profile` before config/session/auth/network/
service effects. Normal commands without `--profile` retain their normal service
behavior and must not be used to operate this trial.

## Profile and endpoints

The profile must be explicit, absolute and independent. Normal/default or current
XDG FlickNote roots, their ancestors/descendants, path traversal, symlink escapes
and nonempty unmarked directories are rejected. No normal configuration is
implicitly loaded and no live session/database is copied. The profile contains:

- `REAL_ACCOUNT_TRIAL`, identifying this mode.
- `config/flicknote/`, including its own `session.json` and temporary OTP PKCE state.
- `data/flicknote/`, including `flicknote.db`, `daemon.sock` and `daemon.lock`.

Port 0 allocates an available **loopback** MCP listener; an explicit other port
may be used, except the normal daemon port 37789. Startup reports its actual
`IPC=…/daemon.sock` and `MCP=http://127.0.0.1:PORT/mcp`. CLI data commands select
that socket through the same profile; no MCP registration is required:

```bash
FLICKNOTE_ENV=prod target/debug/flicknote --profile "$PROFILE" list
FLICKNOTE_ENV=prod target/debug/flicknote --profile "$PROFILE" content NOTE_ID
```

GUI mode exposes local Unix IPC and local loopback MCP only. The independent
`flicknote private-mcp` headless PostgreSQL route is unchanged: its remote tools,
full-grant verifier, identity RLS, auth configuration and Host/Origin contract
remain as documented in [private-mcp.md](private-mcp.md). The GUI does not host
that remote service or route it to the local database. Local MCP still validates
Host/Origin and retains machine provenance, lifecycle and strict output schemas.

## Today, sync and creation

Cached database/Application/IPC/MCP readiness precedes the first network download.
Connection, refresh and reconnect work does not block foreground input. Quiet
loading/auth/sync-error messages follow existing sync events; an empty cache
before the first download is not a definitive empty day. Today watches current
account notes from local 04:00 to next-day 04:00, including DST transitions,
ordered by confirmed numeric ID descending, capped at 10,000. Canonical metadata
and current-account project context come from the bounded watch; project rail
navigation remains inactive. No fixture user/projects/creator are injected.

GUI create/read/copy/archive use production Application operations. Existing
local reads and mutations follow production sync semantics during an outage;
remote-backed creation still requires the network. There is no added offline
creation queue or automatic mutation retry. Definite rejection preserves text
recovery without replacing new typing. An unknown/partial create preserves its
structured identity, known canonical detail and **do not submit again** guidance;
check the identified note after sync/recovery before considering another create.
Pending/watch acknowledgement reconciles by persisted ID in either order.

## Close, reopen and Quit

Closing Today cancels only its watch/input work; IPC, local MCP and sync continue.
Command-1 reopens a fresh watch over the same host, including changes made while
closed. Command-Q explicitly quits: owned operations and actors cancel, servers
stop, sync disconnects, the database checkpoints and the socket/lock ownership
releases with bounded shutdown. A diagnostic lock file may remain after release.
Startup cancellation and a required runtime actor failure also clean up and
report truthfully. A competing owner never deletes the original owner's socket.

After Quit, the same profile can run headlessly:

```bash
FLICKNOTE_ENV=prod target/debug/flicknote --profile "$PROFILE" daemon run --mcp-port 0
```

Ctrl-C/SIGTERM performs the same bounded shutdown. Run one owner at a time;
there is no service installer or automatic GUI/headless handoff for profiles.
Synthetic mode remains explicit `--root` with fixture-owned storage and is
covered separately in [embedded-gpui-spike.md](embedded-gpui-spike.md).

## Verification and pinned dependencies

```bash
cargo test --locked -p flicknote-sync --all-features runtime::local_host_tests -- --nocapture
cargo test --locked -p flicknote-gpui
cargo test --locked -p flicknote-cli
bash scripts/check-routine.sh
```

Tests own their temporary paths/listeners and fake GoTrue/PostgREST/PowerSync
transport; rendered tests cover production creation, email login, cancellation,
close/reopen and retained input/IME/layout behavior. No live state or service is
a test fixture. Existing PostgreSQL integration tests remain ignored without
provisioning; its unchanged boundary requires no new PG harness.

Kit/base/component/assets 0.7.0, GPUI-pre 0.3.7 and the exact maintenance/license
exceptions in the synthetic guide were rechecked for this experimental trial.
Versions and exception lists are unchanged; CLI/client/headless normal graphs
exclude them, and the pure client remains backend/GPUI-free. `cargo deny check`
passes without a new vulnerability/soundness waiver, broad ignore or fork. The
source-bound implementation report records graph paths, license hashes, tests,
builds and unperformed native/cloud evidence. No new manual matrix is required.
