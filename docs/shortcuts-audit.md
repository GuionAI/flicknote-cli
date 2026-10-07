# Desktop shortcuts audit and GPUI implementation reference

Audit date: **2026-10-04**. This records authoritative audit #3289 and the final
runtime behavior of specs #3290 and #3296. Source pins:

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

| Keys | Pinned Swift desktop behavior | GPUI through #3430 |
| --- | --- | --- |
| Cmd1 | Select Home | Implemented: select Home/current Today; open Home if closed |
| Cmd2..9 | First eight active projects in displayed sidebar order; missing index is a no-op | Implemented: same rail order, stable project UUID, retained All/Week; no numbering for Shared/Archive/Charts |
| OptionUp/Down | Home → active projects → Shared → Archive → Charts, across groups, bounded/no wrap | Implemented Home → active projects → Shared → Archive, bounded/no wrap; Charts skipped (#3402) |
| OptionH/Left, OptionL/Right | Home Today previous/next date; project Week previous/next week; no project-All time action | Implemented #3422: previous/next Home semantic day or project Week; current forward and All are no-ops; draft/marked/editor retains input priority |
| OptionJ/K | Next/previous confirmed note, no wrap; reveal selection; detail follows only if already open | Home/project-All/Shared/Archive with empty/unmarked composer native priority adapter; user DEV-v2 manual native navigation PASS; full IME/candidate behavior unverified |
| OptionA | Archive selected active note; success selects surviving successor, else predecessor | Implemented with existing guarded application archive; no automatic retry |
| Return | Nonempty composer creates with detail closed or appends with confirmed detail open; empty composer opens selected detail | Implemented primary Return; archived detail creates new; marked Return commits composition without premature create/append/open |
| ShiftReturn | Composer newline | Implemented by the retained Kit textarea |
| Keypad Enter | Swift accepts it like Return for selected-detail routing | Deferred; user explicitly does not need it for this slice; no added GPUI binding |
| CmdF / CtrlF | Focus workspace search | Implemented #3430: focus native same-list workspace search; CtrlF remains editor-local |
| Escape | Close detail → exit search/focus composer → dismiss workspace, according to current state | Implemented search exit/restore origin and composer focus, else close detail; marked/transient input retains priority; workspace dismissal remains deferred |
| CmdA/C/V/X | Native select-all/copy/paste/cut in the owning text editor | Retained Kit native editing; these are not missing workspace actions |
| CmdZ / CmdShiftZ | Native undo/redo | Retained Kit native editing; navigation retains the same composer/undo state |
| CmdReturn | Save project summary while editing | Implemented #3384: save without adding a newline; marked input retains Kit priority |
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

Home watches the chosen 04:00-to-04:00 semantic day, including DST. Projects
default to **All active notes across dates**, capped at 10,000 and short-ID descending.
Project All/Week and Home historical day navigation are implemented in #3422;
historical pagination, settings/global and Charts remain deferred. Watched project archival/removal falls back
Home while retaining the composer. Ordinary reopen retains the last available
process-local destination and draft/caret; closed capture completions retain
recovery or unknown/partial identity and guidance. Cmd1 resets Home current Today.
With detail closed, capture is a new unassigned note (`project=None`). With confirmed
detail open, Return appends to the accepted UUID with exact text; `Append to #ID`
shows the next target. The same-target busy guard retains new typing. Append identity,
recovery and completion survive navigation/close/reopen without automatic retries. Pending/unknown/partial
identity survives navigation/close/reopen, with no unmatched optimistic project rows.

Normal GUI packaging reuses existing config/session/endpoints; synthetic tests use owned roots and port0.
Follow [the normal GUI operator guide](normal-gui-host.md) for existing-config,
source/hash-bound packaging and opt-in launch. This audit authorizes no account,
service, installed-app, release or deployment change.

## Project and organization editors (#3384)

Add project and summary Edit use focused Kit inputs. Workspace number/Option,
note capture and detail actions stay inactive under the modal editor. Return
saves a project name; in the summary it inserts a newline, while Cmd-Return saves
without inserting one. Marked input keeps composition priority. Escape cancels
only after input transient/composition handling; Cancel leaves stored data intact.
Saving disables duplicate submissions and dismissal until completion; errors
retain editable text. Summary completion is bound to its editor/UUID, while
watch is authoritative. The masked organization key field uses Kit's single-line
input and no composer native adapter. See [organization](automatic-organization.md)
for configuration and retained host behavior. No general Settings shortcut is added.

## Only mine header control (#3389)

Tab focuses **Only mine**; Space/Return activates its accessible pressed state.
It has no new global or workspace shortcut. Activation preserves ordinary composer
draft/caret and accepted capture/append work; modal editors and marked composition
block it. Its account-scoped default-off choice persists across Home/project-All,
window close/reopen and restart. It excludes only JSON boolean `created_by_ai:true`
before the bounded watch LIMIT. Rail/context and explicit-ID operations stay
unfiltered; Jev includes hidden eligible notes. Shared/Archive use the same filter; Charts remains
deferred. See [source choice and storage recovery](normal-gui-host.md#workspace-source-choice-3389).

## Chinese input-source Option routing (#3296)

The user’s owned five-note diagnostic established **English source: navigation
works; Chinese source: symbols are inserted**. The prior Swift/AppKit desktop
worked under the Chinese source according to the user; that comparison is a
manual baseline, not an automated native result. Exact input source names were
not recorded. UTC 2026-10-04 12:15:48–12:15:56 contains English `alt-j/k` callback,
navigation action and selection. At 12:16:02 the Chinese stage inserts U+2206
without a corresponding J callback/action. Earlier 12:15:40–12:15:41 records
U+2206/U+02DA; source identity for those earlier events is unknown.

Pinned Swift `FlickNotePanel.sendEvent` routes workspace hardware keys before
`super.sendEvent`; marked input passes through, and note navigation requires an
empty composer. Pinned `gpui-pre-macos 0.3.7` `window.rs` routes printable keys
to an active IME before the application callback when its input handler prefers
IME. Its unmatched callback path subsequently calls `inputContext.handleEvent`,
so giving bindings first refusal does not replace ordinary initial composition.
Kit’s `ElementInputHandler` opts into IME-first for editable inputs.

The local composer adapter uses the public `InputHandler` priority capability
only while empty/unmarked and delegates all other methods to Kit. It registers
after Kit paint because the last focused handler is the native handler; it uses
Kit’s public `text_bounds()` after rendered layout, preserving candidate geometry
without measurement state or feedback. Login and read-only detail retain their
own handlers. Draft/marked input keeps IME-first priority; editing and focus stay
with Kit. No global interception, key remapping or dependency fork was added.

The [Kit 0.7 keybinding guide](https://gpui-kit.com/docs/keybinding/) documents
focused contexts, consuming handlers and platform-delivery limits. The historical
[Zed Option-key issue](https://github.com/zed-industries/zed/issues/20425) is
comparative context; it does not prove current pinned runtime behavior.
The normalized `alt-j→∆` replay passed before repair and is not native proof.
The native priority regression failed before the policy change and passes with
empty/draft/marked, unmatched input, composition, exact bounds and focus checks.
**Corrected-source native Chinese green remains unverified.** Automated event
posting was unavailable; no system permission change or repeated matrix is implied.


## DEV-v2 navigation and selection feedback

The user reports Option-J/K now effective on DEV-v2 source
`310f05e30d6d4e98a71d84bc86f00905856a0916`: manual corrected-source native
navigation PASS. This does not establish full IME composition/candidate behavior.
The user then clarifies functional text selection is likely painted but too faint.

The minimal workspace adapter had assigned neutral list-row selection to Kit’s
`Theme.selection`, whose documented role is **input selection background**.
Pinned Kit defaults are Light `#55a0fc` / Dark `#1d4ed8` with intended alpha `0.3`.
The correction retains those maintained defaults and puts neutral row selection
in the dedicated list-active role. No input-handler or global palette change.
Read-only iOS `RsEditor` uses cursor tint at alpha `0.3` for text selection; Swift
desktop `selectionFill` is a workspace row role and its NSTextView leaves text
selection defaults intact. These comparisons clarify roles, not a palette to copy.
Apple documents distinct [selected-text background](https://developer.apple.com/documentation/appkit/nscolor/selectedtextbackgroundcolor)
and [selected-text foreground](https://developer.apple.com/documentation/appkit/nscolor/selectedtextcolor)
roles. No system-color bridge was added.

Owned rendered tests compare baseline Kit and wrapper functional partial selection,
focus, marking and positive identical text bounds in Light/Dark, retain the default
role/alpha, and verify composed selected-area color distinction and text readability.
The old neutral override fails the regression. Public standard test windows cannot
capture pixels (`no HeadlessRenderer configured`); that limitation is recorded,
not an extra gate. Corrected native selection appearance awaits personal user trial.

## Workbench presentation trial (#3326)

The continuous rail/list/dock/right-reading layout retains the audited bindings,
IME priority adapter, draft/marked guards and detail-focus recovery. The same Kit
input selection remains separate from row selection in both themes. No shortcut,
preview parity or destination scope expands. Owned rendered tests cover 980×720
and 760×560 detail open/closed, final-row/full-width/action reachability and
composer growth without reading-pane overlap. Native pixels/IME/cloud and personal
aesthetic acceptance are separate evidence, unperformed by this implementation.

## Catch up and pointer release (#3398)

Catch up lives in Automatic organization, with three/seven-day choice and
Start/Stop; no workspace shortcut or date-navigation surface is added. Closing
its editor retains host-owned work. Number/Option/capture guards under editors
remain unchanged. Pointer selection uses Kit's root layer: release ends the
interval before unpressed motion, and exact plain Cmd-C preserves whitespace.
The pane-wide click interceptor blocked MouseUp bubbling; bare Kit passed while
the actual reader and a minimal intercepted TextView failed. Disabling Copy
capture or code-block actions separately left the same failure. The repair removes
that interception and scopes dismissal to the center canvas. Canonical and block
Copy, focus/IME and keyboard guards remain as documented. This is rendered evidence,
not a native input/pixel claim.

## Note dragging and collections (#3402)

Shared and Archive are functional bounded destinations across dates. Only mine
filters before LIMIT on these pages as on Home/project-All. Option-Up/Down adds
Shared then Archive after active projects; number hints remain Home/project-only.
Archive detail is readonly with Restore; its composer creates a new unassigned
note. Option-A remains an active-note archive action and never restores.
Active detail exposes Share/Copy share link and watched Unshare.

Public GPUI note dragging targets active project UUIDs, Archive or Shared. Shared
publishes and copies only the confirmed gateway URL; already-shared drops reuse
get-or-create semantics. Empty/unmarked/editor/append busy guards retain priority.
Accepted drag does not also click a row or destination. Pointer hover and Option-J/K
use last-input related-project emphasis, with drop-target priority, without
changing the persistent destination. See [the operator contract](normal-gui-host.md#dragging-shared-and-archive-3402)
for bounded watch reconciliation, uncertainty, close/reopen and verification limits.
Charts, reorder and global shortcuts remain deferred. Draft dragging remains deferred.


## Calendar navigation (#3422)

Home's Today/date header and project All/Week controls use real watched ranges.
Week starts Monday at local midnight, independently of Home's 04:00 day boundary.
Previous/next and Today/This week are keyboard-reachable Kit buttons; forward is
disabled at current time. Option-H/Left and Option-L/Right share actions. Empty
workspace input has a scoped descendant binding overriding Kit word motion;
with draft or marking that context disappears and Kit input/IME retains priority.
Mouse controls permit ordinary draft while marked input and editors block them.

Home day and each project UUID's mode/week survive rail visits and close/reopen
in process memory. Cmd1 resets Today; project numbers restore retained scope.
Current modes renew on clock boundaries; fixed historical selections stay anchored.
Scope changes close detail and clear selection/old rows while preserving composer
identity/caret/undo and accepted operations. Shared/Archive/All do no time action.
See [the operator calendar contract](normal-gui-host.md#historical-days-and-project-weeks-3422)
for range identity, pending membership and canonical action reconciliation.
Owned LocalHost/rendered tests cover actual header clicks, focused key dispatch,
minimum Light/Dark layout and stale period emissions. Native pixels/OS IME/cloud
remain unverified; no native launch or new manual gate is required.

## Workspace search (#3430)

CmdF focuses the native Kit input in the existing center list. Nonempty keywords
search active non-draft notes across all dates/projects; Only mine filters before
the top-50 bound. Positive numeric/`#ID` access puts an exact canonical note first
and ignores discovery source filtering. Arrows/Return in the search input select
and open canonical results; Return can flush debounce. Native CmdA/C/V, caret,
composition and candidate handling stay with Kit. Marked Return/Escape commits or
cancels composition first, and modal project/organization editors block search.
OptionJ/K follows search selection only under the existing empty/unmarked composer
guards; focused search text never navigates origin notes or enters the composer.

Clear or Escape exits to the saved destination UUID/period/selection/scroll under
current Only mine. Rail clicks cancel search and explicitly enter their destination;
no later Escape rebound. Missing projects fall Home current Today. Draft/caret and
accepted operations survive query edits, exits and close/reopen; search is disposable.
Canonical reader/copy/append/sharing/archive/classification retain their guards,
including for results outside origin watch. Owned rendered tests at 980/760 points
in Light/Dark and actual LocalHost/FTS establish behavior; native pixels/OS IME and
cloud remain unverified. No Command lookup or archived keyword search is provided.
