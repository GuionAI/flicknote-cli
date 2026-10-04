# Desktop shortcuts audit and GPUI implementation reference

Audit date: **2026-10-04**. This records authoritative audit #3289 and the final
runtime behavior of spec #3290, with its documentation delta. Source pins:

- Read-only `fn-desktop`: `65c4b6380d1c9b2087205fc0d09ed06c34b513fd`.
- `fn-cli` pre-change baseline: `52ab28b27b1efd65758285b4b088ce45cb7496f2`.
- `fn-cli` navigation implementation: `12555c9f26624f31ed3a38ba15024404fb407163`.

Swift references are repository-relative in the pinned `fn-desktop` checkout:
`Talkify/FlickNote/FlickNotePanelController.swift` (routing at 191/236,
handling at 624, rail hints at 1139/1157) and
`Talkify/FlickNote/FlickNoteInteractionController.swift` (destination mapping
at 594). GPUI references are [bindings/actions](../flicknote-gpui/src/ui.rs),
[rendered workspace/rail](../flicknote-gpui/src/presentation.rs),
[bounded watch](../flicknote-sync/src/today.rs) and Kit 0.7.0's
`gpui-base/src/input/base/state.rs` editing bindings. These are **source audit**
and rendered-test evidence; new native OS keyboard/IME/pixel and real dev cloud
acceptance were **not performed**. A binding alone does not establish a working
surface or native acceptance.

| Keys | Pinned Swift desktop behavior | GPUI after #3290 |
| --- | --- | --- |
| Cmd1 | Select Home | Implemented: select Home/current Today; open Home if closed |
| Cmd2..9 | First eight active projects in displayed sidebar order; missing index is a no-op | Implemented: same rail order, stable project UUID, project-All; no numbering for Shared/Archive/Charts |
| OptionUp/Down | Home → active projects → Shared → Archive → Charts, across groups, bounded/no wrap | Partial desktop scope: implemented Home + active projects only, bounded/no wrap; unavailable groups skipped |
| OptionLeft/Right | Home Today previous/next date; project Week previous/next week; no project-All time action | Deferred: fixed current Today and project-All only; no date/Week surface |
| OptionJ/K | Next/previous confirmed note, no wrap; reveal selection; detail follows only if already open | Implemented on either active GPUI note surface with the same selection/detail behavior |
| OptionA | Archive selected active note; success selects surviving successor, else predecessor | Implemented with existing guarded application archive; no automatic retry |
| Return | Nonempty composer submits; empty composer opens selected detail | Implemented primary Return; marked Return commits composition without premature create/open |
| ShiftReturn | Composer newline | Implemented by the retained Kit textarea |
| Keypad Enter | Swift accepts it like Return for selected-detail routing | Deferred; user explicitly does not need it for this slice; no added GPUI binding |
| CmdF / CtrlF | Focus workspace search | Deferred: no GPUI workspace search; Kit editor-local CmdF is a separate capability |
| Escape | Close detail → exit search/focus composer → dismiss workspace, according to current state | Partial: close detail and restore composer; retain marked/transient-input handling; no workspace search/dismiss stack |
| CmdA/C/V/X | Native select-all/copy/paste/cut in the owning text editor | Retained Kit native editing; these are not missing workspace actions |
| CmdZ / CmdShiftZ | Native undo/redo | Retained Kit native editing; navigation retains the same composer/undo state |
| CmdQ | Quit application | Implemented: explicit host shutdown; window close keeps host alive |
| CmdComma | Settings | Deferred: no GPUI settings surface |
| Global Fn / configured trigger | Desktop global workspace trigger | Deferred: no GPUI global hook/trigger |

## Guards and destination identity

Swift number shortcuts permit a nonempty composer but ignore marked composition.
GPUI follows that distinction: Cmd1..9 keep draft text/caret, close detail and
focus the existing composer. Only the first eight displayed active projects get
Cmd2..9 hints. Rail clicks and number indices resolve to UUIDs before dispatch;
watch swaps drop the prior work, exclude stale completion and clear old rows.

Swift Option navigation requires an empty composer, no marked text and no modal
editor. With a draft, J/K/A are swallowed; arrows pass through to native editor
motion. GPUI's OptionUp/Down destination bindings are available only with an
empty/unmarked composer; ordinary draft editor dispatch stays intact. J/K/A and
empty Return retain their existing draft/marked guards. Number bindings are
workspace-scoped and do not navigate the login pane. Native editing commands
continue to belong to the focused composer or read-only detail.

Home watches the current 04:00-to-04:00 semantic day, including DST. Projects
show **All active notes across dates**, capped at 10,000 and short-ID descending.
No Week toggle, historical pagination, date/search/settings/global or unavailable
group implementation is implied. Watched project archival/removal falls back
Home while retaining the composer. Ordinary reopen retains the last available
process-local destination and draft/caret; closed capture completions retain
recovery or unknown/partial identity and guidance. Cmd1 selects Home.
Capture is always a new unassigned note (`project=None`). Pending/unknown/partial
identity survives navigation/close/reopen, with no unmatched optimistic project rows.

All local trial/spike/test packages use dev; prod is reserved for formal releases.
Follow [the dev-account operator guide](real-account-gpui-trial.md) for new-profile,
source/hash-bound packaging and opt-in launch. This audit authorizes no account,
service, installed-app, release or deployment change.
