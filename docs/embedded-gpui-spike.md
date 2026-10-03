# Synthetic embedded GPUI experiment

Spec #3240 / ADR #3239 evaluates an embedded local host before any production
cutover. `flicknote-gpui` provides a simple English macOS Today window;
`flicknote-spike` runs the same host headlessly. Neither is distributed or
installed by the release workflow. The production CLI/daemon and Swift desktop
keep their existing behavior.

## Build and launch

On Apple Silicon macOS, using the repository Rust toolchain:

```bash
cargo build --locked -p flicknote-gpui -p flicknote-spike -p flicknote-cli
target/debug/flicknote-gpui --root /tmp/flicknote-synthetic-spike --mcp-port 0 --seed 30
# Alternative, after quitting the GUI host:
target/debug/flicknote-spike --root /tmp/flicknote-synthetic-spike --mcp-port 0 --seed 30
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
focus. Previous/Next and row clicks select notes. Archive is blocked while the
composer has unsubmitted text or marked composition. On success it selects the
first surviving successor from the prior persisted-ID order, or surviving
predecessor if no successor remains. Batched insertions/removals and either
watch/acknowledgement order preserve that choice; failure keeps rows/detail usable. No
mutation retries occur automatically. Watch errors offer an explicit retry.

Closing the window keeps MCP/IPC available. Use the application menu's Open Today
or Command-1 to reopen a fresh subscription. Command-Q explicitly stops the host,
drains existing services and releases socket/ownership. Headless Ctrl-C/SIGTERM
uses the same shutdown coordinator. GUI database/runtime operations run on Tokio;
the native event loop receives snapshots and operation completions.

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

This slice supplies no login, real cloud connectivity, projects/search/charts,
Markdown parity, voice/global trigger, updater/signing distribution, Linux GUI,
production takeover or GUI/headless handoff. It makes no design-fidelity claim.
Native/automated results and outstanding evidence gates must be reported
separately; build success alone does not establish responsiveness.

## Current verification evidence

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
