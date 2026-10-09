# Normal macOS GUI host

Spec #3374 makes the GPUI workspace the normal single local host, reusing
ADR #3239's LocalHost ownership, watch and close/Quit lifecycle. The GUI exposes
existing Unix IPC and loopback MCP; remote PostgreSQL `private-mcp` remains
independent with its existing verifier, RLS and Host/Origin contract.

## Build and package

On Apple Silicon macOS, from clean committed source:

```bash
cargo build --locked -p flicknote-cli -p flicknote-gpui
python3 scripts/package-gpui.py --output .scratch/normal-gui-host/normal-v2 \
  --signing-identity 3C696934260FDAFA30B7A1959AD9CC0D964C5BB7
```

Choose a new output version if it exists. The normal `FlickNote.app` directly
runs `Contents/MacOS/flicknote-gpui` without a wrapper, profile flags, forced
endpoint environment or CLI runtime dependency. Packaging creates no runtime
state, launches nothing and installs no service. `SOURCE.json` records source
commit/tree, unsigned input digest, certificate metadata and final signed app hashes; run `shasum -a 256 -c SHA256SUMS` in that output.
Preserve historical apps/profiles/sessions and scratch evidence unchanged.
The explicit local identity is **Apple Development: Sifang Feng (Z283R4SYUZ)**,
SHA1 `3C696934260FDAFA30B7A1959AD9CC0D964C5BB7`, team `RL5WAW5828`.
The stable bundle identifier remains `app.flicknote.gpui`. Packaging requires a
certificate SHA1; it never chooses the first identity or falls back to ad-hoc signing.
It signs the complete bundle after Info.plist/resources, preserves existing
entitlements, and lets codesign generate the certificate-bound designated requirement.
No hardened runtime, sandbox or additional entitlement is introduced.

Before recording hashes it verifies strict signatures, certificate fingerprint,
Apple Development authority, identifier, team and matching bundle/executable
requirements. `sha256` and `SHA256SUMS` cover every final app file, including the
resource seal and signed executable; `unsigned_input_sha256` separately identifies
the cached Cargo input. Do not compare that input digest to the signed executable
as if signing preserved bytes. A failed or timed-out signing/verification operation
leaves no success manifest; retain that failed output for diagnosis and choose a new
output on retry. Missing or locked identities fail clearly. Any Keychain authentication
prompt needs the user; never automate it, unlock a Keychain or change certificate,
trust or private-key ACL settings. Tests inject a fake signer and own all fixtures.

Reuse the existing cached build after the locked build verifies clean committed
source; do not compile a second historical version just to compare requirements.
For artifact verification, sign new scratch copies of two available different
source-bound GUI versions with the same identity and identifier, verify each,
and compare `codesign --display -r-` output. Preserve the historical originals;
label the older copy as a reference, never as the current-source deliverable.

This is local development signing, not Developer ID distribution or notarization.
Apple describes update identity and Keychain requirement tracking in
[TN2206](https://developer.apple.com/library/archive/technotes/tn2206/_index.html)
and [the requirement language guide](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/RequirementLang/RequirementLang.html).
An existing Keychain item may prompt once on the first transition from ad-hoc to
certificate identity. Do not promise zero prompts, recreate the item or broaden
access to all applications. Static signatures and hashes do not prove native
launch, dyld loading or actual Keychain acceptance. Those remain the user's manual
check after reviewed merge and Orc installation, without another acceptance matrix.
Release distribution, updater and boot-at-login registration remain deferred.

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
or a project to switch the real destination; each new project opens **All** its active
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

Cmd1 selects Home current Today; Cmd2..9 select the first eight projects in displayed order,
allowing a draft but blocking marked composition. Missing numbers do nothing.
Option-Up/Down requires an empty, unmarked composer and traverses Home,
active projects, Failed, Shared and Archive without wrap; with a draft it leaves editor dispatch intact.
Option-J/K, empty Return, Option-A and Escape retain their note/detail guards on
the active surface. The composer’s local native input adapter gives bindings first
refusal only while input is empty/unmarked, so Chinese-source Option letters can
reach navigation before AppKit inserts a symbol. Unmatched initial letters still
fall through to the native IME; drafts and marked input keep IME-first priority.
Kit continues to own text, selection, undo, composition and candidate geometry.
Text highlighting retains Kit’s dedicated input-selection color and intended
alpha in Light/Dark; workbench row selection uses a separate list role.
Shared and Archive are available; Charts is available (#3450). Settings, global shortcuts and keypad Enter are deferred; see [the source shortcuts audit](shortcuts-audit.md).

With detail closed, capture creates a new **unassigned** note (`project=None`),
including in a project. With confirmed active detail open, the composer shows `Append to #ID` and Return appends to that note using the existing Application NoteAppend.
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

## Historical days and project weeks (#3422)

Home starts Today and offers previous/next semantic days, selected date and Today
return. Home days use local 04:00–04:00 half-open bounds. Project first visits remain
All; All/Week retains a separate choice per project UUID. Week uses Monday local
00:00 to next Monday 00:00 exclusive, displays its actual range, and offers
previous/next week and This week return. Calendar arithmetic handles DST and
cross-year weeks. Forward at the current period is disabled; future periods are
unavailable. Shared/Archive and project All have no temporal actions.

Option-H/Left and Option-L/Right dispatch the same previous/next actions only with
empty, unmarked composer and no modal editor. Draft arrows retain Kit word motion;
marked text and modal editors retain priority. Pointer controls preserve ordinary
draft/caret/undo but block marked input and editors. Compact Kit controls sit
below the project summary and above the note list;
project All/Week keeps the same control-region height, with Week's range on its
own readable line at narrow widths. Home controls sit immediately below its
header, without a summary spacer.
Scope/date changes close detail, reset row selection, discard old rows and replace
the watch. Native pixels and OS IME remain separate, unverified user handoff.

Home day and per-UUID project mode/week survive rail visits and close/reopen in
process memory, without saved preferences. Current Today/Week follows its boundary;
historical selections remain fixed on database emissions. Cmd1 explicitly resets
Home to current Today; Cmd2..9 restores project choice. A selected missing/archived
project falls Home current Today and loses its unavailable project choice.

The owner/source/project/date predicates apply before LIMIT 10,000; metadata,
previews and fixed 32-point rows remain unchanged. Period plus watch epoch rejects
old snapshots and errors; action observation/optimism includes the resolved range.
Absence in another period cannot acknowledge an accepted action. Confirmed Share
retains the existing five-second bound, uncertainty and no automatic retry.
Creation always uses current time and no project; historical Home and project
ranges never acquire invented pending members. Accepted capture/recovery identity
stays process-owned. Historical detail appends to its immutable UUID and completion
never moves the viewed date. Empty history leaves completed sync presentation quiet.

## Workspace source choice (#3389)

**Only mine** sits on the right of the existing header. Its pressed state applies
across Home/day, project-All/Week and Charts, defaults off, and survives destination
changes, window close/reopen and normal restart. Click it or use Tab then Space/Return.
Only `metadata.created_by_ai` JSON boolean `true` is excluded. Missing metadata/key,
null, false, numbers, strings and other JSON types remain visible; legacy `created_by`
strings, note type, current AI status and later processing/edits do not define source.
GUI-created pending and confirmed notes remain included.

Failed, Shared and Archive ignore this choice (#157), including global keyword
search initiated from those destinations. Their entire source-control group,
including **Not saved** and its retry action, is absent with no reserved space or
keyboard/accessibility target. Entering them never writes or clears the saved
choice; returning to Home, a project or Charts restores its pressed state and
filtering. Direct internal watches enforce the same utility policy before LIMIT.
Late preference load/save notifications leave their effective watch/search/action
scopes unchanged, retaining rows, reader and accepted operation identities.

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
The loaded scope precedes the first eligible note projection; utility views can
project immediately while the preference loads. A storage error retains the
session choice, shows **Not saved** beside the control, and offers retry after fixing
storage access. Rapid choices and late watch/save completions cannot restore an
older projection or overwrite a newer choice. No Swift preferences are imported.

This is a list projection, not access control. Explicit-ID Application, CLI IPC
and MCP operations remain unfiltered without warning. Background Jev continues
across all eligible creation channels, including hidden MCP notes. Search exact-ID access follows this same unfiltered read contract.

## Same-list workspace search (#3430)

The native single-line Kit search input sits in the upper-left rail, in a
borderless 44-point cell aligned with the center header (#3443). CmdF
focuses it from regular workspace controls or composer, preserving draft/caret;
CtrlF/B retain native editor-local forward/backward editing; login and modal
editors retain their own input. Empty input keeps ordinary browsing.
Nonempty keywords replace that same list with up to 50 ranked active non-draft hits across all projects and dates,
including from Failed/Shared/Archive origins. Only mine is applied by the existing FTS
before its bound for Home/project/Charts origins; utility origins bypass it and
keep the source-control group absent. Search uses the exact Today row renderer: the same 32-point
height, folded content/title preview, type glyph, project-color dot, accessories,
hover and selection. Only the collection and ranked order differ. Long previews
truncate with the same reserved metadata space. #3446 supersedes #3430's two-level
excerpt/highlight rows and successful scope/Top 50 strip; successful results reserve
no status space. The 50-hit backend bound remains, without pagination or a total
count. Numeric or `#number` queries put an exact canonical ID first, deduplicating
any lexical hit. Exact access ignores discovery source filtering and retains draft/archive
readability; archived lexical search and Command lookup remain deferred.

On first nonempty query, search snapshots the destination UUID, period, selected
UUID and scroll anchor. Edits never replace that origin. Clear/Escape restores it
under the origin destination's effective Only mine choice (always unfiltered for
Failed/Shared/Archive), retaining surviving selection/scroll and normal
neighbor behavior. Current periods follow the clock; historical ranges stay fixed.
Unavailable projects fall back to Home current Today. Any actual rail click clears
search and enters the requested destination, including the same project; Escape
cannot later rebound. Closing discards search and retains origin browsing and the
process-owned composer/caret and accepted capture/append work for ordinary reopen.

Input, native editing, selection and composition belong to Kit. About 200ms debounce
waits for unmarked input; Enter flushes and opens the selected/first result, and arrows
navigate results in the search input context. Composition consumes Return/Escape
first. Option note/navigation guards retain composer/editor priority. Modal editors
cannot launch search. Unmarked Escape from empty search returns focus to the
same composer without changing browsing; populated search exits even with detail
open. Reading-focused Escape closes detail first. Kit consumes native transient
surfaces and marking before workspace exit. Loading, empty and error feedback stays quiet and distinct; errors offer explicit
Retry search. Composition shows feedback only when there are no retained rows.
Effective Only mine changes, query edits, exit, rail navigation and window teardown invalidate
older responses. Local database notifications refresh only active search, without
polling or automatic error retries.

Search discovery uses Application NoteFind and a bounded local identity/metadata/
preview projection, without preloading bodies. Only selected or exact-ID notes use
canonical Application NoteGet; mutations never target a synthesized UUID or row
index. The reader retains exact Markdown Copy, plain/code-block Copy, append,
sharing, archive/restore and classification guards. Accepted action membership
includes search query/source separately from origin calendar membership. Canonical
body/lifecycle/project changes reconcile through the active notification stream;
missing notes close stale readers and preserve safe selection. Discovery itself
never creates or changes a note. Closed detail creates current unassigned notes;
only an explicitly open canonical active reader supplies an append target.

Owned production LocalHost/fake HTTP/port0 and rendered Kit tests cover body-only
FTS outside the 10,000-row slice, backend rank/snippet contracts, source/limit/lifecycle boundaries,
exact IDs, canonical outside-origin reader/append/actions, input/composition guards,
stale generations, origin periods/scroll and close/reopen. Light/Dark rendered
980/760 geometry includes same-note Today/search preview, glyph and project-dot
comparison. This is separate from unverified native OS pixels/IME and cloud use.
No native launch or new manual acceptance gate is required.

## Creation Chart (#3450)

Charts is a real rail destination after Archive, included in bounded Option-Up/Down
navigation without a Command-number shortcut. The center keeps its existing header,
Only mine control and composer dock. A native Kit 0.7.0 public Plot uses Stack and
Bar primitives, scales, understated date/count axes and grid. The 30 dates include
zero days; project legend entries show labels and counts. Pointer hover resolves
one painted segment and shows its semantic date, project and exact creation count.
#3452 supersedes the initial cursor-relative tooltip. Loaded nonempty plots always
reserve 68 points above their axes/bars, even without hover. A 190-point card stays
in that band, centered on the selected date bar and clamped to an 8-point canvas
inset; pointer movement within the same segment leaves its placement fixed. A
subdued 1-point vertical guide follows that bar center through the plot. Neither
card nor guide intercepts pointers. Only actual painted segments resolve; zero
days, gaps, axes and leaving clear the card and guide immediately.

The two-line card uses the current Light/Dark surface/border/text roles: 12-point
semibold date, then a 7-point project-color circle, single truncated project name
and right-aligned 11-point tabular-digit count. Padding and corner radius are 9
points, with a 6-point row gap and at least 12 points between name and count.
No internal day-boundary explanation, totals, actions, shadows or animation appear.
Snapshot-specific native Plot identity prevents an old hover from surviving new
data; scope changes discard it. Hover does not rebuild the background aggregate.
This recovers the settled old desktop interaction (#2798 / read-only 65c4b638),
while retaining Kit 0.7 native pointer tracking. The wrapping legend scrolls
inside its own bounded region. The chart and legend expose text labels/totals
independently of bar color. No drilldown, note selection, export or new analytics preference
is introduced.

Statistics follow the pinned old Swift chart's active-list contract: **stored
active notes include drafts, queued and ready statuses equally**. Status is not
creation eligibility; archived notes are excluded. This corrects #3450's initial
list/search conflation before implementation: Swift creationHistory supplies no
status filter, listAll paginates the ordinary list, and its aggregator does not
inspect draft status. Ordinary search retains its independent non-draft discovery
contract unchanged. Only mine excludes only JSON boolean `created_by_ai:true`,
before counting; every other JSON marker value remains included.

The source is canonical persisted notes for the host's account, with valid
RFC3339 created_at in a half-open rolling range. It includes the current local
04:00 semantic day and 29 predecessors; each boundary uses calendar arithmetic,
including DST/month/year changes, ending at the next 04:00. Count all matching
UUIDs without the 10,000-row browsing or 50-hit search limits. Short-ID allocation,
edits, append/share optimism and routes do not constitute creation. Archive/restore
changes active totals; project assignment moves its count without changing the
total. No note content, title or summary is loaded for aggregation.

Groups use project UUID, never display name. The owned project JOIN retains labels
and configured six-digit colors for archived assignments, keeps same-name projects
separate, and hides foreign project metadata. Dangling/foreign assignments retain
isolated Unknown project identities; unassigned is a distinct neutral group.
Missing/invalid configured colors use a stable UUID-derived fallback palette.
Renaming/recoloring updates presentation without duplicating creation counts.

The watch/query and aggregation run on the host's Tokio runtime using cached local
PowerSync data, without a network analytics endpoint. SQL reduces timestamp/project
counts; Rust applies exact RFC3339 half-open boundaries after a small SQL candidate
envelope that avoids SQLite date rounding dropping boundary timestamps. Active
rail context remains independent of source/range. This projection makes no cloud
completeness claim. Loading, empty “No notes created in this period” and an error
with explicit Retry remain distinct; changing source clears the old chart.

Entering Charts closes detail and clears note rows/selection. Option-J/K/A, empty
Return and period-H/L/arrows cannot operate a hidden prior note. Charts does not
accept a note drop. The same Kit composer preserves ordinary draft/caret/undo and
creates current-time unassigned notes, then canonical watch updates the counts.
Accepted operations and Home/project period memories remain process-owned.

CmdF and the borderless rail search still discover ordinary active notes. Starting
search from Charts drops the Chart subscription; Escape/clear restores Charts as
the saved origin under current Only mine. Explicit rail clicks cancel that origin.
Search input, same-Today rows and native CtrlF/B behavior retain #3443/#3446.
Ordinary close/reopen retains Charts and composer state in process memory and
starts a fresh watch; closing, leaving or searching cancels the chart work.
Existing IPC/MCP/sync/organization hosting continues independently. Local 04:00
rolls the range without a database write; source/epoch/range guards reject older
completions. No analytics daemon, periodic polling or saved chart cache is added.

Owned host checks cover >10,000 eligible notes, status/source/owner/project matrices,
exact boundaries, archive/restore/classification/creation notifications and rollover.
Rendered Kit pointer checks cover actual segment tooltips/legend and geometry at
760/980 points in Light/Dark, reserved hover bounds, day-center/clamp/guide,
painted colors and stable bars, same-segment motion, snapshot/source/leave clearing,
rail traversal, retained draft/caret, no hidden note actions, search-origin restoration, stale responses, retry and close/reopen.
These are isolated host and rendered-component evidence; native pixels, OS input,
cloud completeness and installed-app/Keychain acceptance remain unverified.

## Projects and automatic organization (#3384)

The Plus icon beside the rail's Projects heading creates by name through Application. A project's
summary appears above All/Week/date controls in a fixed total 80-point region,
including inset and Edit. The region never shrinks or grows with its text;
wrapped long text scrolls internally, and an empty summary retains the same
region with **Add a project summary**. Edit offers multiline Save/Cancel and
Cmd-Return. Editors retain failure text, own native input and block workspace
navigation/capture. Project creation selects the canonical watched UUID while
preserving the note draft/caret. Add project, summary and Automatic organization
share Kit 0.7.0's public unstyled `base::Dialog` host, with the existing skin and
no added animation/shadow. Its stable focus trap keeps Tab inside the editor;
guarded confirmation/cancellation retains busy and marked input, and only the
UUID-bound successful async operation dismisses a saving editor. The standard
backdrop/popup parts occlude the workspace. Inner textarea scrolling uses Kit input;
wheel input at its bounds, panel padding and backdrop cannot scroll the covered
note list. Dismissal restores ordinary list scrolling. No click/MouseUp/key
interceptor is added; Kit Root selection release and IME retain their normal route.
Login remains a full-window authentication page, and detail/composer remain
persistent workspace panes rather than dialogs.
Owned LocalHost/rendered tests establish wheel offsets and 980/760 Light/Dark
geometry; native OS wheel, pixels and IME remain separate user handoff evidence.

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

## Failed processing notes (#153)

Failed appears before Shared in the utility rail and bounded Option-Up/Down
traversal. It watches the current account's active notes across all dates and
projects whose stored status is exactly `ai_failed` or `source_failed`, descending
numeric short ID with the existing 10,000 bound. Owner, active and failure
predicates apply before LIMIT; both human and MCP creation channels are included
regardless of saved Only mine. The entire source-control group is absent (#157). The current six-status backend
contract has no plain `failed` status. This local discovery view adds no public
MCP status schema or generic lifecycle setter.

The same row previews, selection, Markdown reader, canonical Copy, append,
classification, archive and sharing remain available under their existing guards.
Failed is not a drop target; supported project and collection action targets
retain their existing semantics. Classification and unsharing preserve Failed
membership; archive or a canonical transition to a nonfailure status removes it.
Absence from another destination cannot acknowledge an accepted action. A watch
removal from its accepted Failed scope ends its overlay without resurrection.

There are no date controls or new saved preference. Loading, errors and sync
feedback remain shared, with a Failed heading and `No failed notes` empty state.
Ordinary capture creates current unassigned notes and never adds queued/ready
pending rows here. Detail append keeps its immutable accepted target and lifecycle.
Destination/source switches reject stale emissions and preserve composer
text/caret/undo and marked-input/modal priority. Close/reopen retains Failed in
process memory; Cmd1 selects Home Today and Cmd2..9 still select projects only.

Owned runtime and rendered tests cover status/source/owner/archive matrices,
pre-limit membership, transitions, navigation, capture, action reconciliation
and retained destination. Native pixels/IME and cloud account use remain
unverified. #157 prohibits native/CUA launches and captures; owned rendered
checks are the UI gate for this slice. Packaging never
launches the normal deliverable or performs installation or service changes.

## Dragging, Shared and Archive (#3402)

The rail's Shared and Archive rows open real bounded pages across dates. Shared
includes active notes with a current same-account share row, excluding expired
links; Archive includes deleted notes, including archived drafts. Both use numeric
short-ID descending order and the 10,000-note bound. Both include every creation
channel regardless of saved Only mine, and omit the entire source-control group
including storage-error retry (#157).
Known project metadata remains visible even for archived assignments; the rail
still contains active projects only. Destination/source swaps retain the same
composer draft/caret/undo and discard old watch completions. Pending creation is
shown only on Home. Option-Up/Down traverses Home, active projects, Failed, Shared and
Archive without wrapping; Cmd1 and Cmd2..9 retain their numbering. Charts follows Archive in bounded traversal.

Drag one confirmed active non-draft note from Home, project-All or Shared onto an
active project to classify, Archive to archive, or Shared to publish and copy its
confirmed share URL. These are intentional actions without a confirmation modal.
Home/Charts and archived, draft, pending or busy notes cannot accept these drops.
Unsubmitted/marked input and modal editors retain priority; an outstanding append
blocks conflicting actions on that note. Same-project drops do nothing. GPUI owns
the drag threshold, cancel/outside release and drop hit testing; no external OS
payload, import/export, multi-select, unassign or reorder is provided.

The plus glyph is centered in the same accessory column as the numbered rail
hints, with a 28-point accessible hit target. The existing color-only trailing
note dot remains unchanged; a project without color has no invented indicator.
Row pointer hover and Option-J/K show lighter related-project feedback while the
persistent destination selection stays in place. Last input wins, leaving the
list falls back to the selected note, and drag-target feedback takes priority.
Missing/archived assignments never highlight an active project falsely.

Classification/removal feedback appears immediately, then canonical watch wins
in either acknowledgement order or after another writer. A completed operation's
projection expires after five seconds if no watch confirms it; the complete
workspace operation has a 30-second deadline. Definite pre-write rejection removes
its feedback; uncertain failures say to inspect the note before trying again.
Confirmed Share acknowledgement projects shared feedback until canonical watch or
the existing five-second expiry; already watched shared notes need no overlay.
No failure copies a stale/fabricated link or automatically retries publishing.
Accepted operations retain their original UUID/short ID and project UUID with the
host across navigation and window close. Quit cancels them through the existing
operation registry. Classification preserves content/lifecycle/provenance and
uses the manual assignment path, without calling Jev.

Active detail offers Share or Copy share link, and Unshare when watched shared.
Copy share link reuses the canonical get-or-create gateway semantics. Unshare
removes the row from Shared; archive removes an accepted row from the active
page. Archive detail offers Restore instead, has no sharing/classification/append,
and its composer creates a new unassigned note. Restore removes the row from
Archive without switching destinations. Row removals choose the surviving
successor, else predecessor, through the existing model; final removal closes
detail and restores composer focus. Markdown selection, MouseUp propagation,
canonical Copy and code-block Copy retain their reader contracts.

Owned LocalHost/fake HTTP and rendered pointer tests verify these interactions,
account/source/expiry/limit membership, gateway errors, delayed close/reopen and
geometry in both themes at 980/760 points. They use only temporary state and port0.
Native pixels/input/IME, real cloud/share links and real Keychain acceptance remain
separate, unperformed evidence for the user's trial after reviewed installation.

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
independent review/merge and source/hash/signature verification. Preserve a rollback package.
Install the already signed whole bundle; never sign or patch Applications in place.
Verify the sealed signature and final hashes again without modifying the bundle.
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

## Retry failed processing (#155)

Each canonical Failed row has a compact **Retry** button at its far right,
within the fixed 32-point row. Its reserved width keeps previews and project
markers separate at minimum/normal widths in Light/Dark. Search and other
views have no Retry button. Pointer activation and Tab then Space/Return target
the button's immutable UUID and observed failed stage, independently of selection
or reader. There is no new shortcut. Ordinary composer draft/caret/undo remain
intact; marked input and modal editors block activation.

The process-owned action host calls a narrow local Application operation over
the normal PowerSync writer. Within one writer transaction, current-account,
active UUID and exact observed failed stage must still match. `ai_failed` maps
to `ai_queued`; `source_failed` maps to `source_queued`. Only status and
`metadata.error` change. Both `created_at` and `updated_at`, other metadata,
provenance, content/source/title/summary/project/flag/extractions and archive
state are preserved. NULL metadata stays NULL. Unlike the read-only iOS
reference's `deleted_at=NULL`, this active-only action never restores an archive
in a race. Ready, queued, opposite-stage, foreign, archived and plain `failed`
rows are truthful no-matches, with canonical refresh rather than claimed success.

The row shows **Busy…** while its UUID has an accepted action. Duplicate retry,
append, archive, share and classification on that UUID are blocked. Navigation
and close/reopen retain accepted UUID/stage in the existing host; they cannot
retarget or resurrect a row. No optimistic retry removal is invented: the watch
removes the queued row. Errors stay in the workspace feedback area even if the
row disappears. An unknown result, including a timeout, locks another retry
until **Check note #ID before retrying** explicitly reads the canonical local
note and refreshes the view. Another processor failure alone cannot unlock it.
Checking does not resubmit processing; there is no blind or automatic retry.
A successful local queue write does not prove cloud upload or processing success;
normal sync continues with the existing connector, including during an outage.

Reference authority: read-only fn-ios `Shared/Database/Note/NoteModels.swift`
(`NoteStatus.retryQueuedStatus`), `Shared/Database/Note/NoteRepo+Write.swift`
(`retryNoteProcessing(id:failedStatus:)`) and
`Shared/Components/Notes/NotePreviewListItem.swift` (`handleRetry`). This slice
adds no machine DTO/status setter, IPC/MCP tool/schema, PostgreSQL adapter or
DDL change. Owned runtime/rendered tests establish local behavior and geometry;
native pixels and cloud processing remain separately reported evidence.
