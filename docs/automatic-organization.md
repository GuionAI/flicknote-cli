# Automatic organization in the macOS GUI

The normal GUI host offers **Add project** in the rail. Create a project by
name, then use **Edit** above its All list to maintain its summary. Names are
trimmed and must be nonempty and unique. Summary Save preserves the entered
text; a whitespace-only value clears the summary. Cancel preserves the stored
value, and a failed save retains editable text. Cmd-Return saves a summary;
ordinary Return inserts a newline. The focused Kit editor owns composition,
Escape and editing commands. Workspace navigation/capture is suspended while
an editor is open. Creating a project selects its watched UUID while retaining
the note composer draft and caret.

## Configure organization

In the **FlickNote** application menu, choose **Automatic organization…**.
Enter an OpenRouter key in the masked field and select **Save**. **Replace key**
stores a replacement and enables routing. **Disable** pauses routing while
retaining the key; **Enable** resumes it. **Remove key** deletes this GUI's
account-specific item and disables routing. A storage failure cancels routing
and retains the editor for recovery; save the key again after correcting access.
Before saving/replacing a secret, the host durably disables routing while
preserving the cutoff. If that write fails, the secret is not replaced. The host
enables only after secret storage succeeds; a failed final preference write leaves
routing disabled across restart, even if Keychain now contains the replacement.
Save again after correcting storage access to enable it explicitly.
The control displays the latest configuration/provider error. It does not add
an ongoing status banner or resize the note list.

Keys use the new macOS generic-password service
`ai.guion.flicknote.gpui.automatic-organization.v1`, with the current FlickNote
account identifier as the account. The Swift app's item is never read, imported
or modified. Keys are absent from JSON preferences, logs and packages.
Keychain access may require macOS permission during personal use; that prompt
and real Keychain behavior have not been exercised by automated verification.
Credential access and preference I/O run off the UI thread after host ownership.
The host fixes account identity until Quit; account switching is outside scope.

## Eligibility and the first-start cutoff

The first organization-capable normal GUI startup records a UTC cutoff for the
current account, even without a key. Account cutoff and enabled state are stored
without secrets in `gui-organization.json` beside `daemon.sock` in the existing
data directory, with private file permissions. Other account records are retained.
Restart, key replacement and enable/disable preserve the cutoff. Existing Swift
preferences are not imported, and pre-cutoff notes are never swept automatically.
Do not reset this file to request historical reclassification.

After configuration, post-cutoff notes can catch up. Eligibility requires the
current owner, an active ready note, no project assignment, and
`metadata.project_routing.routed` other than JSON boolean true. Draft, processing,
archived, foreign and previously routed notes stay outside the classifier.
A post-cutoff note still processing becomes eligible when it reaches ready.
All creation channels participate, including MCP-created notes hidden by the workspace
**Only mine** filter. That separate account-scoped list choice never changes Jev
credentials, cutoff, eligibility or routing. Blank summary and blank content defer the
note until meaningful input appears. With no active projects, no request is made.

## Compact input and provider costs

Requests go to OpenRouter's fixed Decisions endpoint with
`typesafe/jev-1.13`. Each note gets a mutually exclusive Choice among **all**
active same-account projects and `none`. Model text contains project names and
summaries, and trimmed note summary; a missing/blank note summary falls back to
canonical content trimmed and cut at a valid UTF-8 boundary at 2048 bytes.
No title, project-note history, week memory, internal UUID or timestamp is added.
Request-local aliases are mapped back to canonical identities locally.

This sends private note/project descriptions to a paid external provider.
Review [OpenRouter's Decisions reference](https://openrouter.ai/docs/api/api-reference/alphadecisions/submit-a-decisions-request)
and [Jev guide](https://openrouter.ai/docs/guides/community/jev) before enabling.
No real provider/account or paid acceptance call was made during implementation.

One request contains at most eight notes and at most 64KiB of encoded JSON.
Eligible batches drain without waiting for another watch event. Large note inputs
may reduce the batch size. Oversized project context produces a recoverable
configuration error and no request; candidates are never silently dropped.
Shorten summaries to resume. There is no summary-generation pass.

## Failure, manual precedence and lifetime

The host watches eligibility and project descriptions. It allows one request or
route batch at a time, with a 30-second complete HTTP deadline. Responses must
have exactly the expected answer keys, Choice types, allowed choices and finite
selected probabilities in [0,1]. Invalid responses fail the whole batch.
`none` records a routed marker with no project assignment. All successful
assignments use the existing Application transaction and PowerSync watch, so
project dots and membership update naturally. Content, lifecycle and creation
provenance are preserved.

Changes to requested input/project context invalidate old responses. Credential
changes, disable/remove and Quit cancel old-generation work. The transaction
compares the expected compact note/project snapshot under the same writer before
any assignment or routed marker, so edits between response verification and route
acquisition cannot receive a stale decision. It also rejects concurrent manual
assignment, archival, nonready state and
inactive/foreign projects. Manual assignment takes precedence. Routed notes
are never reclassified automatically after later edits.

Unchanged failed batches receive at most three automatic attempts, with
increasing delays of two then four seconds. Exhausted batches pause until
meaningful input/project context or configuration changes. Unrelated watch
emissions do not reset their budgets. Auth/credit failures (401/402/403) pause
until credential/configuration changes. Inspect the control, correct the key,
credits or summaries and save/enable again. No malformed/partial decision is
applied, and failures do not invent a project or success.

Organization belongs to the normal GUI host's process lifetime. Window close
retains it, sync, IPC and local MCP; reopen observes fresh routing state. Explicit
Quit cancels HTTP/watch/retry and shuts down ownership. Cached-first workspace
readiness does not wait for credentials or provider networking. Synthetic mode
starts no live Keychain/provider adapter. Headless automatic organization is
deferred; headless/local MCP behavior and private PostgreSQL MCP are unchanged.

## Owned verification and package use

Tests use temporary roots, fake secrets/provider HTTP and port0. They exercise
Application project creation/modification/routing, compact input and answer
validation, cutoff persistence, watched dots/membership, retry and stale/manual
races, foreground typing, close/reopen/Quit, and retained workbench contracts.
These checks establish neither native pixels/OS IME nor real Keychain permissions,
provider acceptance or live cloud/cutover behavior.

Build clean committed source and use `scripts/package-gpui.py` with a **new**
versioned ignored `.scratch` output. Verify `SOURCE.json`, source/tree identity,
binary equality and `SHA256SUMS`. The normal app directly runs its GUI binary.
Follow the matching [normal host guide](normal-gui-host.md) and package `RUN.md`
for separately directed manual launch/cutover and rollback, then configure through
the application menu above. Packaging itself launches/installs nothing and changes
no services, credentials, historical profiles or prior artifacts.
