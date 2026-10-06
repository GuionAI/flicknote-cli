# Normal macOS GUI host

Spec #3374 makes the GPUI workspace the normal single local host, reusing
ADR #3239's LocalHost ownership, watch and close/Quit lifecycle. The GUI exposes
existing Unix IPC and loopback MCP; remote PostgreSQL `private-mcp` remains
independent with its existing verifier, RLS and Host/Origin contract.

## Build and package

On Apple Silicon macOS, from clean committed source:

```bash
cargo build --locked -p flicknote-cli -p flicknote-gpui
python3 scripts/package-gpui.py --output .scratch/normal-gui-host/normal-v1-3374
```

Choose a new output version if it exists. The normal `FlickNote.app` directly
runs `Contents/MacOS/flicknote-gpui` without a wrapper, profile flags, forced
endpoint environment or CLI runtime dependency. Packaging creates no runtime
state, launches nothing and installs no service. `SOURCE.json` records source
commit/tree and app hashes; run `shasum -a 256 -c SHA256SUMS` in that output.
Preserve historical apps/profiles/sessions and scratch evidence unchanged.
Signing, release distribution, updater and boot-at-login registration are deferred.

## Existing config, ownership and login

Normal launch takes no arguments. `Config::load()` resolves the same normal
XDG config/data directories as the daemon (default ~/.config/flicknote and
~/.local/share/flicknote). It reads the existing config and session in place;
there is no copy, migration or separate GUI environment. Explicit FLICKNOTE_*
endpoint overrides take precedence over saved config. Saved nonempty fields are
retained; missing fields use FLICKNOTE_ENV, default dev when unset. Backend DEV
support and shared CLI config/profile semantics remain unchanged.

The host uses existing daemon.sock and http://127.0.0.1:37789/mcp by default.
FLICKNOTE_MCP_PORT retains the daemon's explicit port override (0 allocates).
Startup reports actual IPC/MCP endpoints and logs to the existing flicknote.log.
The GUI acquires the data-directory lock before reading/authenticating the
session or starting listeners. An incumbent rejects startup without takeover,
session replacement or socket unlink. Bind failure cleans only owned startup
resources and releases ownership.

A usable identity/access/refresh session skips login. Otherwise the current Kit
email pane offers Send code, Verify, Change email and Resend code. Requests are
bounded to 30 seconds and duplicate submissions are disabled. Window close
cancels that pane's request; Quit cancels startup. Stale responses cannot start
a host after cancellation. Errors retain email and recovery; tokens/OTP are
never printed. Authentication cannot change identity under an active host.
For unrecoverable auth failure, Quit before authenticating again.

## Manual cutover after acceptance

These are operator steps, not packaging/test actions. Quit the old GUI trial,
then use the existing installed CLI to remove managed startup and stop the
headless incumbent:

```bash
flicknote daemon uninstall
flicknote daemon stop
flicknote daemon status
# After verifying the new artifact hashes, launch the packaged app:
open /ABSOLUTE/NEW/OUTPUT/FlickNote.app
flicknote daemon status
flicknote list
```

Verify the existing MCP client still connects to its original endpoint and the
CLI reports the normal socket. Preserve the old trial's independent profile;
never copy its sessions/database into the normal directory. If another owner
remains, the GUI fails; resolve ownership manually instead of retrying takeover.
Closing the workspace retains sync, IPC and MCP; Command-Q ends the host.
No login item is installed: manually launch after login until that work is done.
For rollback, Quit the GUI, then run `flicknote daemon install` and
`flicknote daemon start`; verify status and original endpoints before use.
Installed binaries, registrations and cloud accounts are unchanged by this PR.
Normal native launch/pixels/IME, live normal cutover and cloud writes were not
performed by automated verification.

## Home, project-All, sync and creation

Cached database/Application/IPC/MCP readiness precedes the first network download.
Connection, refresh and reconnect work does not block foreground input. A visible
Kit progress bar and restrained status text follow the existing SDK status stream,
without polling or counts from the database/cloud. Notes download operations map to 0–90%.
Downloaded 100% still waits for the notes applied checkpoint; applied notes show
“90% — Finishing sync…” until **all active default subscriptions** have applied.
Completion reaches the weighted 100% state and immediately hides the first-sync
indicator. After completion, ordinary connecting/downloading activity shows no
message, bar or reserved status space; cached completion/reopen/reconnect keeps the
list viewport stable. Real offline/error feedback remains. Optional subscriptions do not block
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

Spec #3341 applies the [desktop preview policy](embedded-gpui-spike.md#fixture-and-interaction-contracts)
to both Home and project-All: <=512 raw UTF-8 bytes show folded content; longer
notes show nullable title or exact `Untitled note`. Empty titles remain empty;
internal spacing is preserved. Pending previews use content until reconciliation,
and canonical detail/copy, row geometry and selection remain intact.

Spec #3348 adds real title, known project (including archived assignments) and
short ID to readonly Markdown detail. Missing metadata is silently omitted.
Toolbar Copy preserves exact stored Markdown; selected text Cmd-C copies exact
plain rendered text, including code whitespace. Each code block's top-right Copy
button copies its whole payload without fences or language. Only explicitly
activated http/https links open; images do not
fetch network/file/data resources. Formula/Mermaid rendering and editing remain
deferred. See [the reader contract](embedded-gpui-spike.md#markdown-detail-reader-3348)
for scroll, identity, focus and verification limits. Preview/list behavior stays
as specified above.

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

With detail closed, capture creates a new **unassigned** note (`project=None`),
including in a project. With confirmed detail open, the composer shows `Append to #ID` and Return appends to that note using the existing Application NoteAppend.
The accepted UUID and exact text stay immutable after navigation or window close.
Nonempty content receives two newline characters before the exact submitted text;
an empty body receives that text alone. Accepted input clears immediately and the
reader extends optimistically, without a new timeline row. One outstanding append
per target retains later typing on Return; other targets and closed-detail capture
remain available. Watched content is authoritative, with no duplicate optimistic
suffix in either watch/ack order. Toolbar Copy always copies stored Markdown.

Definite append rejection removes only its overlay and restores text only into an
empty, unmarked composer still showing the same target. Otherwise submitted text
remains available through Copy submitted text. A possible-after-write error preserves
target/text with check-before-resubmitting guidance, never a blind retry: the current
service updates content before reading mutation metadata, so even a metadata read
failure cannot establish rejection. Later typing, titles, project assignment, lifecycle
and creation provenance remain intact. Pending/unknown/partial creation operations
keep their original identity across
switches; unmatched captures are never injected into a project's list.

GUI create/append/read/copy/archive use production Application operations. Existing
local reads and mutations follow production sync semantics during an outage;
remote-backed creation still requires the network. There is no added offline
creation queue or automatic mutation retry. Definite rejection preserves text
recovery without replacing new typing. An unknown/partial create preserves its
structured identity, known canonical detail and **do not submit again** guidance;
check the identified note after sync/recovery before considering another create.
Pending/watch acknowledgement reconciles by persisted ID in either order.

## Workspace source choice (#3389)

**Only mine** sits on the right of the existing header. Its pressed state applies
across Home/current Today and project-All, defaults off, and survives destination
changes, window close/reopen and normal restart. Click it or use Tab then Space/Return.
Only `metadata.created_by_ai` JSON boolean `true` is excluded. Missing metadata/key,
null, false, numbers, strings and other JSON types remain visible; legacy `created_by`
strings, note type, current AI status and later processing/edits do not define source.
GUI-created pending and confirmed notes remain included.

The owner/destination/source predicate precedes ordering and the 10,000-note SQL
limit. The active project rail and summaries stay available even with no matching
notes. Today retains its 04:00 bounds and fixed 32-point preview rows. Switching
source preserves ordinary composer text/caret, pending capture/append and a still
visible selection/reader. If detail disappears, it closes with existing focus
recovery and neighbor selection; another detail never opens automatically. Project
and organization editors and marked composition block activation.

The process owns the choice. `gui-source.json` beside `daemon.sock` stores only
account identifiers and booleans with private permissions, separately from
organization cutoff/enabled state and Keychain. Loading and saving run off the UI
thread after directory ownership; IPC/MCP/auth/sync readiness does not wait.
The loaded scope precedes the first note projection. A storage error retains the
session choice, shows **Not saved** beside the control, and offers retry after fixing
storage access. Rapid choices and late watch/save completions cannot restore an
older projection or overwrite a newer choice. No Swift preferences are imported.

This is a list projection, not access control. Explicit-ID Application, CLI IPC
and MCP operations remain unfiltered without warning. Background Jev continues
across all eligible creation channels, including hidden MCP notes. No direct-ID GUI,
Shared/Archive/Charts browser, search or date surface is added.

## Projects and automatic organization (#3384)

The Plus icon beside the rail's Projects heading creates by name through Application. A project's
summary appears above its All list; Edit offers multiline Save/Cancel and
Cmd-Return. Editors retain failure text, own native input and block workspace
navigation/capture. Project creation selects the canonical watched UUID while
preserving the note draft/caret. Long summary display scrolls within a bounded area.

The application menu's Automatic organization control offers masked Save/Replace,
Remove and enable/disable, with its own error feedback. Normal GUI startup starts
an independent host-owned organization coordinator after publishing cached readiness.
It records a per-account first-start cutoff even without a key, uses a new
account-scoped Keychain service and compact OpenRouter Decisions Choice requests,
and survives window close. No old Swift credentials/preferences are imported.
Eligibility, privacy/cost, bounded retries, cancellation/manual precedence,
non-secret preference location and configuration steps are authoritative in
[automatic organization](automatic-organization.md). Headless routing is deferred;
private PostgreSQL remains unchanged. Synthetic mode starts no live adapter.

## Recent organization and reader release (#3398)

Automatic organization also offers Catch up for three or seven local 04:00
workdays, including today. Preview is local and free of inference; enabled
credentials are required for Start. Start freezes the range end without changing
the persisted cutoff. The shared coordinator prioritizes regular notes and drains
without a total cap, at least ten seconds between shared provider batch starts.
Progress, Stop and retained completed/unfinished results stay in the organization
control. Closing it or the workspace retains the run; Quit does not resume it.
See [the organization contract](automatic-organization.md#catch-up-recent-notes-3398)
for eligibility, canonical accounting and bounded failure recovery.

Markdown drag selection freezes after primary-pointer release, including release
outside the reader. The reading pane lets MouseUp reach Kit's existing root
selection layer. Exposed-center-canvas dismissal is scoped to that canvas,
so clicks in the reading pane preserve detail and selection. Plain selected-text
Cmd-C, whitespace, canonical toolbar Copy and code-block Copy retain their
separate contracts. Owned rendered evidence is distinct from native acceptance.

Accepted merged normal GUI updates are installed by Orc to
`/Applications/FlickNote.app` under the user's standing authorization, after
independent review/merge and source/hash verification. Preserve a rollback package.
Installation does not quit/restart a running app or clear its drafts. Worker
packaging and tests perform no installation, native launch or service changes.

## Close, reopen and Quit

Closing Today cancels only its watch/input work; IPC, local MCP and sync continue.
Ordinary reopen restores the last available destination in this process with a
fresh watch, including changes made while closed. Draft text/caret, capture and append
state remain process-local across close/reopen. Completions arriving while closed
retain definite-failure recovery or unknown/partial identity and guidance; reopening
never resubmits them. Append work also completes while closed; reopening observes
its original note through a fresh watch and retains recovery without retargeting. Draft/capture/append values are not persisted to disk or across Quit. Only mine is persisted separately as described above.
Command-1 opens Home while closed and selects Home while open; number bindings
do not navigate login. Command-Q explicitly quits: owned operations and actors
cancel, servers stop, sync disconnects, the database checkpoints and socket/lock
ownership releases with bounded shutdown. A diagnostic lock file may remain after release.
Startup cancellation and a required runtime actor failure also clean up and
report truthfully. A competing owner never deletes the original owner's socket.

After Quit, the normal directory can run headlessly with `flicknote daemon run`.
Run one owner at a time; there is no automatic GUI/headless handoff.
Shared CLI independent `--profile` auth-only/headless functionality remains
available with its original safety/port restrictions. GUI profile launch is removed.
Synthetic mode stays explicit `--root`; see [the synthetic guide](embedded-gpui-spike.md).

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
Run the full routine and locked Mac CLI/GUI builds on final clean committed source.
Keep source-bound packages in ignored `.scratch`. Orc starts independent review
after implementation completion and governs merge. Review/merge do not deploy or
perform normal endpoint cutover. No native launch or new manual matrix is required
for #3374; live cloud, normal native input/pixels and cutover remain unverified.
