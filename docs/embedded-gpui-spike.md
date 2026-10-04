# Synthetic embedded GPUI experiment

Spec #3240 / ADR #3239 evaluates an embedded local host before any production
cutover. Spec #3258 gives `flicknote-gpui` an English macOS Today workspace
referenced to the existing Swift desktop;
`flicknote-spike` runs the same host headlessly. Neither is distributed or
installed by the release workflow. The production CLI/daemon and Swift desktop
keep their existing behavior.

This guide covers explicit synthetic `--root` mode. The independent real-account
`--profile` mode and GUI email login are documented in
[the real-account trial guide](real-account-gpui-trial.md); it reuses the production
host rather than the synthetic fixture creator.

All subsequent local trial/spike/test packages explicitly use **dev**; prod is
reserved for formal releases. New real-account dev trials use a new absent short
profile and versioned artifact; preserve historical prod apps/profiles/services.
See [the dev trial packaging instructions](real-account-gpui-trial.md).

## Build and launch

On Apple Silicon macOS, using the repository Rust toolchain:

```bash
cargo build --locked -p flicknote-gpui -p flicknote-spike -p flicknote-cli
FLICKNOTE_ENV=dev target/debug/flicknote-gpui --root /tmp/flicknote-synthetic-spike --mcp-port 0 --seed 30
# Alternative, after quitting the GUI host:
FLICKNOTE_ENV=dev target/debug/flicknote-spike --root /tmp/flicknote-synthetic-spike --mcp-port 0 --seed 30
```

The root must be explicit, absolute and independent. Normal FlickNote data and
config roots, their ancestors/descendants, symlink escapes and nonempty
unmarked directories are rejected. The experiment creates `SPIKE_ONLY`,
`data/flicknote/flicknote.db`, `data/flicknote/daemon.sock`, an ownership lock,
`data/flicknote/flicknote.log` and `config/flicknote/` inside its root. It never
reads live configuration or sessions. Keep fixture roots separate from real
XDG directories. Never copy real notes, credentials or databases into them.

`--mcp-port` is required. Port 0 chooses an available loopback port; 37789 is
rejected. Operator output reports the actual IPC socket and
`http://127.0.0.1:PORT/mcp`. No service or MCP registration is needed. A second
host using the same root fails without interrupting the first. Restart preserves
fixture writes; seeding only occurs when the notes table is empty.

Connect a **locally built** CLI using fixture-owned XDG paths:

```bash
XDG_DATA_HOME=/tmp/flicknote-synthetic-spike/data \
XDG_CONFIG_HOME=/tmp/flicknote-synthetic-spike/config \
  target/debug/flicknote list
XDG_DATA_HOME=/tmp/flicknote-synthetic-spike/data \
XDG_CONFIG_HOME=/tmp/flicknote-synthetic-spike/config \
  target/debug/flicknote add 'Synthetic CLI capture'
```

Use the reported MCP URL directly in a temporary test client. Its actual
initialize/tools-list/create/read/archive handlers, Host/Origin validation and
output schema contracts are reused. Do not alter daily MCP client registrations.
Machine creation retains MCP provenance; GUI creation uses the human application
input. Machine draft arguments remain invalid.

## Fixture and interaction contracts

Synthetic seeding uses the existing PowerSync schema. An injected `NoteCreator`
allocates fixture short IDs and writes through real application/storage seams.
It performs no cloud authentication or network sync and does not establish
production offline creation semantics. Maximum seed is 10,000 notes.
`--delay-ms N` delays fixture creation (maximum 10,000 ms); submitting exactly
`[fixture-fail]` fails deterministically. `--burst-batches N` updates 100 fixture
rows per batch at 20 ms intervals (maximum 100 batches). These are local synthetic
sync-style updates, not a replacement cloud server.

Today spans the local calendar from 04:00 through next-day 04:00, including DST
length changes. The watched projection includes canonical content and collapsed
single-line previews, ordered by real short ID descending, capped at 10,000.
The same bounded watched query resolves note type and project color, including
project-color updates. The small synthetic project set is Ideas, Reading and
Workspace. Fixtures include short, long, multiline, Chinese and URL content.
Rows have stable IDs, a fixed 32-point height and truncated previews. Confirmed
and pending rows fill the list viewport, independently of preview length; selected
and hover backgrounds and confirmed-row hit targets span that same width. One watched
query publishes immutable latest snapshots; no per-row fetch or polling is used.
Its next calendar boundary replaces the query. Closing Today cancels that watch.

Return accepts capture and clears input immediately; Shift-Return inserts a
newline. Marked composition text cannot submit. Pending rows are non-interactive
and reconcile by persisted ID whether watch or acknowledgement arrives first.
A failed capture restores text only into an empty, noncomposing composer;
otherwise a recovery action retains it without replacing new typing. The detail
is selectable, copyable plain canonical text. Selection does not retarget editor
focus. A confirmed row opens or replaces detail; Close, Escape or exposed canvas
dismisses it while preserving selection and composer text. Escape consumed by
marked composition or the input's own transient surface does not close detail.
Home selects current Today and closes detail. In the focused Today window, Option-J
selects the next confirmed note and Option-K the previous. Either starts at the
first note when nothing is selected; neither wraps. These actions retain editor
focus, reveal the selected row through the existing virtual list and update
canonical detail only when it is already open. Empty-composer Return opens the
selected note. Nonempty Return creates; Shift-Return inserts a newline. IME Return
commits composition without prematurely creating or opening. Option-A uses the
same guarded archive action. Navigation/open/archive shortcuts are blocked by
unsubmitted or marked composer input; native Command-A/C/V editing is retained.
The supported selection/archive actions are also in the application menu.
The empty/unmarked composer gives native shortcut dispatch first refusal before
an active Chinese input source. A local public `InputHandler` adapter delegates
the retained Kit text/selection/composition methods and exact text bounds; draft
or marked input restores IME-first routing. Unmatched initial letters still reach
the native input context. The #3296 regression covers this seam, not native
Chinese acceptance on the corrected source; see the shortcuts audit for evidence.

Active projects are UUID destinations showing All active notes across dates,
bounded at 10,000 and ordered by canonical short ID. Cmd1 selects Home;
Cmd2..9 select the first eight active projects in rail order, retaining draft/caret
unless marked. Empty/unmarked Option-Up/Down traverses Home and projects without
wrap. Shared/Archive/Charts are unavailable and skipped. A watched missing or
archived project falls back Home while preserving the composer. Switching drops
the old list/detail/selection and watch; capture stays global/unassigned with
pending/recovery/unknown identity retained. Unmatched captures stay out of project
lists. See [the shortcuts audit](shortcuts-audit.md) for deferred mappings.

The composer always creates new notes,
even with detail open; it never implies append support. Archive is blocked while the
composer has unsubmitted text or marked composition. On success it selects the
first surviving successor from the prior persisted-ID order, or surviving
predecessor if no successor remains. Batched insertions/removals and either
watch/acknowledgement order preserve that choice; failure keeps rows/detail usable. No
mutation retries occur automatically. Watch errors offer an explicit retry.

Closing the window keeps MCP/IPC available. Use the application menu's Open Today
or Command-1 to open Home with a fresh subscription. Ordinary reopen retains the
last available destination during the process lifetime. Command-Q explicitly stops the host,
drains existing services and releases socket/ownership. Headless Ctrl-C/SIGTERM
uses the same shutdown coordinator. GUI database/runtime operations run on Tokio;
the native event loop receives snapshots and operation completions.

## Today presentation

The layout is sourced from the read-only Swift desktop checkout at
`65c4b6380d1c9b2087205fc0d09ed06c34b513fd`: `FlickNotePanelController`
(workspace, rail, header, composer and detail), `FlickNoteTimelineView`,
`FlickNoteWorkspaceTheme` and `FlickNoteSourceToggleStyle`.

The rail is 252 points wide with 22-point outer insets; the main pane starts
after a 24-point gap. Today uses a 23-point semibold header and full-width
32-point rows with 14-point previews, packaged vector type glyphs and trailing
project dots. Pending captures have the same appearance without status labels
or spinners and remain nonselectable. Search, Shared, Archive browsing and Charts are static inactive landmarks excluded from keyboard
focus. Inactive destination labels use Kit’s secondary foreground; Home uses
foreground. Every destination reserves the same 17-point icon/dot slot and 10-point gap,
so text aligns regardless of symbol size. Unsupported filter, day-navigation,
microphone, project-add, sharing and title-edit actions are omitted.

The bottom composer is centered across the whole canvas, at most 620 points wide,
with 16-point padding, an 18-point radius, 17-point text and a 28-point bottom
inset. Kit's textarea grows from one to six wrapped lines, then scrolls internally.
A fixed blank timeline tail lets the final row scroll above the largest composer
without a geometry feedback loop. Read/copy detail is a bounded right overlay
with an approved 520-point maximum width, 16-point radius and right inset.
Its reading area fills the height above the current composer, including feedback,
with a 16-point gap. It narrows within the main pane and leaves row content
exposed at the native minimum window size of 760×560 points. These width/height
adaptations reflect the manual feedback on spec #3258. This is a desktop slice;
there is no phone layout contract.

Choose Appearance: System, Light or Dark from the FlickNote application menu.
Selection is session-local, including across close/reopen; System follows native
appearance changes while a window is open. It changes this app only.

Text and caret colors follow the active iOS universal Light/Dark assets in read-only registered
`fn-ios` at `300261c7467827208dfd71f7af0eb6615d9935cb`:
`Shared/Theme/Color+Theme.swift` and
`Resources/Assets.xcassets/ThemeColor`. The user approved this after trying Kit’s
default colors; it supersedes both the original Swift desktop palette and the
intermediate default-Kit decision. Following the rejected v3 Light screenshot,
the user approved distinct desktop surface roles instead of the shared iOS
surface/label fills. One local adapter maps these roles into the existing Kit
theme; presentation still reads Kit semantic tokens.
Mac/watch asset overrides are excluded. Fixture project dots retain stored colors.

| iOS universal asset | Light / Dark | Kit roles |
| --- | --- | --- |
| theme-background | FFFFFF / 0D0D0D | background |
| font-primary | 171717 / EDEDED | foreground, surface/accent/button foreground, glyphs |
| font-secondary | 525252 / A6A6A6 | secondary foreground/inactive destinations |
| font-tertiary | 737373 / 808080 | muted foreground/search/headings |
| theme-primary | 2E2E2E / C6C6C6 | primary |
| font-on-primary | E2E2E2 / 222222 | primary foreground |
| theme-cursor | 05C7F7 / 05C7F7 | caret and focus ring |

| Approved desktop role | Light / Dark | Kit roles |
| --- | --- | --- |
| Quiet rail | FAFAFA / 141414 | secondary |
| Elevated composer/detail | FFFFFF / 1C1C1C | popover/surface, neutral button |
| Hover / muted search landmark | F5F5F5 / 202020 | accent, muted, neutral button hover |
| List-row selection | EBEBEB / 2B2B2B | list_active, neutral button active |
| Subtle edge | E8E8E8 / 303030 | border, input |

Text selection retains Kit’s dedicated `selection` role and intended alpha,
separate from the neutral list-row fill.

These are product-authored starting values, not exact Radix tokens. The role
separation follows [Radix's use cases](https://www.radix-ui.com/colors/docs/palette-composition/understanding-the-scale)
and [Spectrum's background layers](https://spectrum.adobe.com/foundations/color/background-layers).
Only composer/detail use GPUI's restrained `shadow_xs` (5% black, 1pt offset,
2pt blur); Dark separation comes primarily from the lighter fill and border.
Appearance-free textareas expose their enclosing elevated background. Search
stays inactive and uses a weaker fill than selected Home. Keyboard actions
remain immediate, without animation. The user manually accepted the v4 visual
direction with “可，先这样吧。” No v4 screenshots or explicit native shortcut/IME
results were supplied; neither the v3 image nor this acceptance proves every state.

Kit retains structural radii, typography machinery, shadows, motion, scrollbars
and unused specialized component colors (e.g. warning/destructive). Today keeps
the approved explicit layout/type sizes. No XCAsset runtime parser, generator,
extra accent palette, customization framework or dependency was introduced.
Inline Apple semantic accent is unnecessary for plain text.

The synthetic boundary is identified in the window title and these operator docs.
Plain text rather than Markdown and creation rather than append are intentional
limits. Lucide vector icons are optical equivalents, not SF Symbols replicas.
The app packages its screen icons explicitly alongside Kit’s default component
assets; icon names alone do not register the full catalog.

## Verification and limits

```bash
cargo test --locked -p flicknote-sync --features experimental-spike --test spike -- --nocapture
cargo test --locked -p flicknote-gpui
cargo tree --locked -p flicknote-spike
bash scripts/check-routine.sh
```

Host tests use temporary roots and actual local PowerSync/MCP/IPC. They exercise
ownership, restart persistence, watched membership, failure, teardown, DST and
bounded 10,000-note updates concurrent with MCP. GPUI test windows exercise the
public composition protocol, Return/Shift-Return, pending reconciliation, row
selection, copying and archive. Such simulated input does not verify a real OS
Chinese candidate window. Native foreground-window checks must separately cover
English/Chinese IME, copy, scrolling, visible repaint, close/reopen and Quit.
Watch emission/initial-query timings, visible render counts and main-thread
heartbeat measurements go to the fixture log, without note content.

The stack is pinned to `gpui-kit = 0.7.0` (Apache-2.0), using its matched
`gpui-pre = 0.3.7` family and component/base/assets 0.7.0. It uses GPUI's native
runtime and the maintained input component, without a framework fork or a
second UI runtime. The headless package does not depend on GPUI. Build acceptance for this slice is macOS Apple Silicon only. Linux/musl
compilation and Linux runtime validation are deferred and not validated; do not
install a cross-toolchain to verify this spike. Upstream build and API evidence is recorded
in the local implementation report.

Synthetic mode supplies no login, real cloud connectivity, workspace search/charts,
Markdown parity, unsupported navigation/date shortcuts, voice/global
trigger, updater/signing distribution, Linux GUI,
production takeover or GUI/headless handoff. The source layout targets Swift
fidelity for the supported Today slice;
the user accepted the current v4 direction and chose to proceed without the
exhaustive screenshot matrix as a further gate for this slice.
Native/automated results and outstanding evidence must be reported
separately; build success alone does not establish responsiveness.

For #3290, existing source/rendered evidence is the automated gate. No new native
launch, CUA, cloud trial or manual stress/matrix is required. Native keyboard,
IME, pixels and real dev cloud acceptance remain unperformed for this source.

## Visual verification workflow

Use a newly built scratch-only app and a new short synthetic root; record the
source commit, binary SHA-256 and root in the local implementation report. Keep
previous trial artifacts and running apps intact. Capture only the owned
synthetic app, never the installed desktop or personal notes. Foreground the
scratch app with `open -a /absolute/path/to/OwnedSynthetic.app` before using
native app capture. Capture tooling failures are unresolved evidence, not proof
of an application repaint failure or of correct pixels.

Record actual images at 980×720 and 1440×900 points in Light and Dark with detail
closed/open, selected/hovered rows and composer text. Also cover empty/error and
760×560 minimum-size behavior. Compare each image against the pinned source
reference, with a table of viewport, theme, state, screenshot path and
match/deviation/unverified reason. Note intentional omissions and icon optical
differences. Any reconstructed SwiftUI reference is source-derived, not a
production screenshot; keep it scratch-only. No Penpot design is identified.
Rendered bounds, AX state and compilation do not establish visual fidelity;
missing native pixels remain unverified evidence. For this accepted #3258 slice,
the user's v4 proceed decision removes the exhaustive matrix as a further user
gate; it does not establish an all-state pass or native shortcut/IME results.
Preserve native vs simulated evidence separately. No repeat captures, manual
matrix requests or prior fixed-ID scratch stress run are required for this handoff.

## Historical verification evidence (before #3258)

Mac Apple Silicon native compilation and the routine suite pass, including
498 workspace tests, Clippy, dependency policy and release fixtures. Temporary
host tests cover actual PowerSync/MCP/IPC contracts. Simulated GPUI windows cover
creation, failure recovery, canonical copy, archive and rendered full-width rows
at 980- and 360-point window widths, including right-edge selection and pending
nonselection. These layout checks measure rendered bounds and hit testing on the
test platform; they do not claim new-layout native pixel observation.

The user separately confirmed immediate foreground English pixels without resize,
real Chinese candidate/Return behavior, Shift-Return, immediate clear/pending to
one confirmed row, canonical copy, archive input guards and successor/predecessor
selection on the prior synthetic artifact. The user also reported no visible
stutter in the 10k fixture and explicitly accepted concurrent responsiveness.
These are user/manual passes, not simulated or CUA visual passes. The scratch
fixed-ID MCP burst stopped when note9995 was archived; that run did not complete.
The owned database confirms its archived timestamp, and the existing write
contract intentionally resolves active notes. Successful bounded integration
traffic is recorded separately; the user accepted the scratch failure as no
merge blocker, with no repeat stress gate.

Native small-fixture close/reopen and explicit Quit were exercised: window
teardown dropped its watch, MCP/CLI stayed responsive with no window, reopening
published a fresh snapshot, and Quit removed the socket and closed the MCP port.
Restart retained synthetic notes. Prior stale captures/cgWindowNotFound have an
unknown root cause; neither a tooling defect nor an application repaint defect
is proven. No periodic repaint or polling masks that uncertainty. The local
report records artifact hashes, measured activity and the distinction between
prior native observations and the subsequent layout correction.

## Approved dependency boundary

Spec #3240 records a user-approved bounded decision for this synthetic spike.
The pinned stack has these informational maintenance notices. No patched
compatible releases remove them without changing the upstream stack; the
experiment retains the verified matching component/runtime versions. Each
notice is named individually in `deny.toml`; global maintenance checking remains
active. Vulnerabilities and unsoundness receive no waiver. `anyhow` is locked to
patched 1.0.103 for RUSTSEC-2026-0190.

| Advisory | Pinned crate | Shortest path from gpui-kit 0.7.0 | Reason for bounded acceptance |
|---|---|---|---|
| RUSTSEC-2024-0384 | instant 0.1.13 | gpui-base 0.7.0 → instant | Existing component timing abstraction; no fixed compatible release |
| RUSTSEC-2024-0436 | paste 1.0.15 | gpui-component 0.7.0 → paste | Component compile-time macros; upstream migration deferred |
| RUSTSEC-2025-0134 | rustls-pemfile 2.2.0 | gpui-kit-assets 0.7.0 → gpui-pre-reqwest 0.12.15 → rustls-pemfile | Assets' non-macOS path; absent from resolved macOS graph |
| RUSTSEC-2026-0206 | rustybuzz 0.20.1 | gpui-pre 0.3.7 → usvg 0.46.0 → rustybuzz | SVG text shaping; upstream parser/shaper migration deferred |
| RUSTSEC-2026-0192 | ttf-parser 0.25.1 | gpui-pre 0.3.7 → ttf-parser | Native font parsing; upstream migration deferred |

`cargo metadata --locked --format-version 1` normal/build dependency paths show
none of these five reachable from production `flicknote-cli`, standalone
`flicknote-client`, or headless `flicknote-spike`. The same metadata with
`--filter-platform aarch64-apple-darwin` excludes rustls-pemfile. This is graph
evidence, not a Linux build/runtime claim. Recheck versions and reachability on
upgrades. All five exceptions must be re-reviewed before any live-data takeover;
they are not automatically approved for production. Future cloud/auth/sync
compatibility and production switching require separately approved work.

The registry manifests and distributed LICENSE files were inspected. The global
license allow-list stays unchanged; only the following exact package/version
exceptions are allowed:

| Package | Manifest SPDX / LICENSE attribution | LICENSE SHA-256 |
|---|---|---|
| enum-iterator 2.3.0 | 0BSD; Stephane Raux, 2018 | 9b166edfb0b4b70f1e8b2fdaae320b0acd977c520e0fb95fa881f54f68e724e3 |
| enum-iterator-derive 1.5.0 | 0BSD; Stephane Raux, 2018 | 9b166edfb0b4b70f1e8b2fdaae320b0acd977c520e0fb95fa881f54f68e724e3 |
| libbz2-rs-sys 0.2.5 | bzip2-1.0.6; Julian Seward 1996–2021, Federico Mena Quintero 2019–2020, Micah Snyder 2021, Trifecta Tech Foundation/contributors 2024–2025 | 54e1fd7bd53273e601c9599eb162a004dacddee031ae297ee7109a7beb93b1c2 |

For reproducibility, inspect `LICENSE` beside each downloaded registry
`Cargo.toml` and hash it with `shasum -a 256`. The 0BSD licenses permit use,
copying, modification and distribution with or without fee and carry a warranty
disclaimer. The bzip2-derived license permits source/binary redistribution;
retain source attribution/conditions/disclaimer, accurately identify original
and altered work, and do not imply author endorsement. Preserve these obligations
and the upstream Apache-2.0 notices in any future distribution. Re-review license
exceptions when versions change and before live-data use. This experiment does
not distribute an installed desktop app.

## Real-account experimental scope recheck (#3279)

The exact pinned Kit/GPUI maintenance notices and license exceptions above were
rechecked for independent-profile email login and real production local-host use.
No version, exception, fork or vulnerability/soundness policy changed. Native Mac
GUI reaches the existing exact exceptions except non-Mac rustls-pemfile; CLI,
pure client and headless normal graphs remain free of them. The source-bound
report records the graph and matching license hashes; the full routine deny check
passes. This is experimental trial approval, not installed production takeover.

First-sync presentation uses the pinned Kit horizontal progress component, with
a structured weighted value from SDK events. Unknown totals use indeterminate
loading without a percentage; applied required-default completion removes the bar.
Kit owns presentation animation; no application timer advances readiness.
