# Independent dev-account GPUI trial

Specs #3279 and #3290 provide an experimental macOS Home/project-All window over
the production local host, with English email-code login. It is separate from installed FlickNote
services and is not part of release distribution. Spec #3326 replaces the earlier floating appearance with a continuous workbench:
196-point rail, full-width 32-point rows, a center capture dock and optional
272–420-point right reading pane. Local keyboard and host behavior remain. See
[the design rationale and roles](embedded-gpui-spike.md#continuous-workbench-presentation-3326).

All local trial/spike/test packages use **dev**. **Prod is reserved for formal
release artifacts.** Preserve historical prod apps/profiles, their sessions and
running services; never repoint, migrate or copy them into a dev trial.

## Build and opt-in launch

Use Apple Silicon macOS and the repository toolchain:

```bash
cargo build --locked -p flicknote-gpui -p flicknote-cli
# From clean committed source, choose a NEW absent short independent profile:
python3 scripts/package-gpui-dev.py \
  --output .scratch/gpui-workbench-redesign/dev-v1-3326 \
  --profile /private/tmp/fn-dev-3326-v1
```

Packaging creates no profile and launches nothing. Choose another unique version
and profile if either example exists. For a user-authorized #3326 visual rebuild,
retain the original DEV-v1 profile without resyncing by supplying its original
manifest explicitly:

```bash
python3 scripts/package-gpui-dev.py \
  --output .scratch/gpui-workbench-redesign/dev-v2-3326 \
  --profile /private/tmp/fn-dev-3326-v1 \
  --reuse-profile-from .scratch/gpui-workbench-redesign/dev-v1-3326/SOURCE.json
```

Reuse validates the original #3326 dev manifest and matching profile argument;
it does not inspect, copy, reset or migrate profile data. The default still
requires a fresh profile. **Quit the old app before launching the new build**;
existing profile ownership rejects concurrent instances. Preserve both artifacts.

The generated `RUN.md` gives the separate
opt-in launch; its wrapper sets `FLICKNOTE_ENV=dev` and pins all effective dev
endpoints even with inherited overrides. `SOURCE.json` and `SHA256SUMS` bind the
source commit/tree, bundle ID, profile, CLI companion and app files. Packaging
binds a test-owned Unix socket of the planned socket path's exact byte length.

Launching this real mode and signing in are manual opt-in cloud operations.
Automated verification uses temporary sessions and local HTTP fakes. The Worker
has not sent real emails, logged into a cloud account, mutated cloud notes or
launched this artifact in a native OS window. Rendered GPUI tests and successful
builds do not establish native IME candidate-window behavior or cloud sync.
Linux builds/runtime and signed distribution remain deferred.

The package includes `FlickNote Dev.app/Contents/MacOS/flicknote` as its matching
CLI companion. Substitute that absolute path for `target/debug/flicknote` in the
operator examples when using a supplied package. It replaces no installed binary
and registers no service/MCP. Preserve prior trials unless separately authorized.

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

The retained CLI alternative uses the existing interactive email OTP flow.
Use the package's matching companion and exact profile. Direct commands below
assume all inherited `FLICKNOTE_*` endpoint/key overrides have been cleared;
the new profile's defaults then resolve to dev. Never use an old prod profile:

```bash
PROFILE=/private/tmp/fn-dev-3326-v1
```

```bash
FLICKNOTE_ENV=dev target/debug/flicknote --profile "$PROFILE" login --auth-only
# Existing browser providers are optional headless/operator alternatives:
FLICKNOTE_ENV=dev target/debug/flicknote --profile "$PROFILE" login --auth-only --provider google
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

The package records public auth `https://dev-auth.flicknote.app`, PowerSync
`https://dev-sync.flicknote.app`, API `https://dev-api.flicknote.app/api/v1` and
gateway `https://dev-gw.flicknote.app`. These public endpoints carry no session
secrets. The source manifest and launcher agree on dev; live endpoint connectivity
has not been verified by the Worker.

Port 0 allocates an available **loopback** MCP listener; an explicit other port
may be used, except the normal daemon port 37789. Startup reports its actual
`IPC=…/daemon.sock` and `MCP=http://127.0.0.1:PORT/mcp`. CLI data commands select
that socket through the same profile; no MCP registration is required:

```bash
FLICKNOTE_ENV=dev target/debug/flicknote --profile "$PROFILE" list
FLICKNOTE_ENV=dev target/debug/flicknote --profile "$PROFILE" content NOTE_ID
```

GUI mode exposes local Unix IPC and local loopback MCP only. The independent
`flicknote private-mcp` headless PostgreSQL route is unchanged: its remote tools,
full-grant verifier, identity RLS, auth configuration and Host/Origin contract
remain as documented in [private-mcp.md](private-mcp.md). The GUI does not host
that remote service or route it to the local database. Local MCP still validates
Host/Origin and retains machine provenance, lifecycle and strict output schemas.

## Home, project-All, sync and creation

Cached database/Application/IPC/MCP readiness precedes the first network download.
Connection, refresh and reconnect work does not block foreground input. A visible
Kit progress bar and restrained status text follow the existing SDK status stream,
without polling or counts from the database/cloud. Notes download operations map to 0–90%.
Downloaded 100% still waits for the notes applied checkpoint; applied notes show
“90% — Finishing sync…” until **all active default subscriptions** have applied.
Completion reaches the weighted 100% state and immediately hides the first-sync
indicator; ongoing sync retains quiet status. Optional subscriptions do not block
completion. This is weighted progress, not a count of remaining unique notes.
Unknown/zero totals use the Kit indeterminate bar with Connecting/Syncing text
and no percentage. Offline/error states cannot complete an in-progress first sync. Previously completed cached sync
skips the indicator, and reconnect/window reopen does not restart it. An empty cache
before the first download is not a definitive empty day. Today watches current
account notes from local 04:00 to next-day 04:00, including DST transitions,
ordered by confirmed numeric ID descending, capped at 10,000. Canonical metadata
and current-account active project UUIDs come from the bounded watch. Click Home
or a project to switch the real destination; each project shows **All** its active
notes across dates, descending short ID and bounded at 10,000. Archived projects
leave the rail, and the selected missing/archived project falls back Home.
Each switch drops the previous query/list/detail/selection, focuses the same Kit
composer and preserves draft, caret and undo. No fixture identity/creator is injected.

Cmd1 selects Home; Cmd2..9 select the first eight projects in displayed order,
allowing a draft but blocking marked composition. Missing numbers do nothing.
Option-Up/Down requires an empty, unmarked composer and traverses only Home and
active projects without wrap; with a draft it leaves editor dispatch intact.
Option-J/K, empty Return, Option-A and Escape retain their note/detail guards on
the active surface. The composer’s local native input adapter gives bindings first
refusal only while input is empty/unmarked, so Chinese-source Option letters can
reach navigation before AppKit inserts a symbol. Unmatched initial letters still
fall through to the native IME; drafts and marked input keep IME-first priority.
Kit continues to own text, selection, undo, composition and candidate geometry.
Text highlighting retains Kit’s dedicated input-selection color and intended
alpha in Light/Dark; workbench row selection uses a separate list role.
Shared/Archive/Charts remain unavailable. Project Week,
date navigation, workspace search, settings, global shortcuts and keypad Enter
are deferred; see [the source shortcuts audit](shortcuts-audit.md).

Capture always creates a new **unassigned** note (`project=None`), including in
a project. Pending/unknown/partial operations keep their original identity across
switches; unmatched captures are never injected into a project's list.

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
Ordinary reopen restores the last available destination in this process with a
fresh watch, including changes made while closed. Draft text/caret and capture
state remain process-local across close/reopen. Completions arriving while closed
retain definite-failure recovery or unknown/partial identity and guidance; reopening
never resubmits them. These values are not persisted to disk or across Quit.
Command-1 opens Home while closed and selects Home while open; number bindings
do not navigate login. Command-Q explicitly quits: owned operations and actors
cancel, servers stop, sync disconnects, the database checkpoints and socket/lock
ownership releases with bounded shutdown. A diagnostic lock file may remain after release.
Startup cancellation and a required runtime actor failure also clean up and
report truthfully. A competing owner never deletes the original owner's socket.

After Quit, the same profile can run headlessly:

```bash
FLICKNOTE_ENV=dev target/debug/flicknote --profile "$PROFILE" daemon run --mcp-port 0
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
builds and unperformed native/cloud evidence. For #3296, the owned diagnostic established English-source Option-J/K navigation
and Chinese-source symbol insertion on the prior routing. Automated regression
checks the corrected public native priority seam, unmatched initial input, Kit
composition delegation, exact rendered text bounds, Home/project navigation and
focus. The user reports DEV-v2 Option-J/K now effective: manual native navigation PASS.
Full native IME composition/candidate behavior and corrected selection appearance
remain unverified;
these tests do not establish OS candidate windows or pixels. No repeated manual
matrix, CUA, cloud or stress gate is required.
Run the full routine and locked Mac CLI/GUI builds on final source;
keep source-bound package evidence in ignored `.scratch`. Orc starts independent
review after implementation completion. This visual trial
waits for the user to try and accept its direction before merge; review does not
deploy or authorize production adoption.
