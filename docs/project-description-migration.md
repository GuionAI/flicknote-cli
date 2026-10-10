# One-time dev project description conversion (#3634)

This is an operator action for **dev only**, owner
`fe4de0a3-0cf4-4d79-92e4-4be3fae2c634`, including archived projects and notes.
The approved reference environment is `guion-tunnel` / namespace `flicknote-dev` /
`flicknote-pg-1` / container `postgres` / database `supabase`. It is never a test
fixture. Workers deliver scripts and isolated verification; Orc owns execution.
No startup migration, background job, durable migration table or requeue is added.
Backend cloud processing is a separate operation, not part of this conversion.

The new client uses `projects.metadata.description` only. It rejects old project
summary modification inputs and never reads/writes a summary alias. Note summaries
stay unchanged. Existing assigned `project_id` values remain unchanged. Client Jev,
Automatic organization/Catch up, probability routing and their credential readers
are retired; no real Keychain items or old preference files are deleted.

## Before execution

Orc must verify the reviewed source and newly signed package, merge/install under
the existing operator boundary, and establish that **all old writers have stopped**,
including pending PowerSync writes from old GUI/CLI clients. Installation does not
quit/restart a running app. Wait for the user to quit normally; do not kill/restart,
change services or clear drafts. Resume cloud sync through existing operational
procedures only. A stale old writer can recreate project summary or old routing
metadata after this transaction; fresh verification alone cannot prevent that.

Confirm the exact environment/database and owner independently before any SQL.
Do not use these artifacts for prod or another account. Historical counts were
22 projects (15 active/7 archived), 912 old blocks (75 none), and 26 new blocks.
These are evidence, not constants. Review current IDs, values, counts and conflicts;
changed evidence requires inspection, not automatic replay or blind approval.

The scripts need a maintenance role able to read the complete owner snapshot and
lock/update the three tables. Keep credentials out of command arguments/logs.
Use the established operator psql transport; this runbook does not supply a live
connection string or initiate tunnel/database operations.

## Preview, approve and dry run

Create a **new**, private, ignored scratch directory (`umask 077`). Keep full
preimages private: they contain every owner project/note row and extraction,
including content, source, metadata, assignments, timestamps and archived state.
Do not put snapshots in Git, PR comments or public logs. Commands below operate
on local files; `PSQL` denotes the already verified operator psql transport with
`-X -qAt -v ON_ERROR_STOP=1` and the confirmed dev database.

```bash
python3 scripts/project-description/render.py preview > /NEW/PRIVATE/preview.sql
# Execute preview.sql through the verified PSQL transport; save its single JSON
# result to /NEW/PRIVATE/preimages.json. No application rows are written.
python3 -m json.tool /NEW/PRIVATE/preimages.json > /NEW/PRIVATE/preview-readable.json
python3 scripts/project-description/render.py apply \
  --snapshot /NEW/PRIVATE/preimages.json > /NEW/PRIVATE/dry-run.sql
# Execute dry-run.sql through PSQL; save its result/error and verify ROLLBACK.
```

Review all current targets and exact old/new values. Any project with a `summary`
key is a target; its value must be a JSON string, and **any existing description
key, including null, is a conflict**. Conflicts fail the whole transaction.
Notes are targets only when `metadata.project_routing` is an object with exactly
`probability` and `routed`, with `routed` JSON boolean true. Old assigned and none
blocks are removed as whole blocks. New `project_id/reason/routed`, extended,
false, missing and unknown shapes remain untouched. Do not weaken this predicate.

Apply locks projects, notes and extractions in one transaction with bounded lock
and statement timeouts. It compares the full current owner snapshot to the
reviewed preimage; edits, inserts, deletes and extraction changes invalidate it.
Table locks block competing writes during the short transaction. A mismatch or
conflict stops with no partial updates. Preview again and inspect the difference;
do not automatically regenerate and replay. No historical expected counts are
hardcoded. Dry runs perform the UPDATEs/assertions but end in ROLLBACK by default.

## Commit and fresh verification

After reviewed dry-run evidence and stopped old writers, render the same approved
preimages with explicit `--commit`:

```bash
python3 scripts/project-description/render.py apply \
  --snapshot /NEW/PRIVATE/preimages.json --commit > /NEW/PRIVATE/commit.sql
# Execute through verified PSQL; save the exact SQL, stdout/stderr and exit code.
python3 scripts/project-description/render.py preview > /NEW/PRIVATE/fresh-read.sql
# Execute in a NEW PSQL session; save fresh JSON and compare with reported postimage.
```

The result includes actual project/note counts and a full postimage. Assertions
compare every owner row to the exact permitted metadata transformation, preserving
all other values, assignments, statuses, note summaries, source, timestamps and
extractions. Owner predicates exclude other accounts. No schema/trigger change or
trigger disabling is used. Archive membership is never a filter.

A fresh preview after successful conversion has no eligible legacy keys: applying
that fresh approved snapshot reports zero changes. Replaying a stale preimage
fails rather than silently succeeding. If old writers recreate targets, stop and
investigate; a rerun is not permission to erase unknown later data.

## Constrained rollback

Retain the original preimages, reviewed SQL and result hashes. Rollback restores
only rows still equal to their **complete expected postimage**, and updates only
metadata. Later description edits, new backend routing markers, unrelated content
or metadata changes, and missing rows cause explicit skipped ID lists. It never
reassigns projects, requeues processing or replaces later user/backend edits.
Inspect every skip; do not force overwrite. A second rollback is a no-op with
already restored rows reported as skipped.

```bash
python3 scripts/project-description/render.py rollback \
  --snapshot /NEW/PRIVATE/preimages.json > /NEW/PRIVATE/rollback-dry-run.sql
# Inspect restored counts/skipped IDs; default ROLLBACK.
python3 scripts/project-description/render.py rollback \
  --snapshot /NEW/PRIVATE/preimages.json --commit > /NEW/PRIVATE/rollback.sql
# Execute only after inspecting the concrete rollback result, then fresh-read.
```

Rollback assertions compare the entire owner snapshot before/after, substituting
only approved, unchanged matching rows. No persistent migration objects remain.
The offline renderer emits SQL only; it has no database connection capability.

## Isolated verification

Use the existing migration-backed harness, unchanged fb dbmate DDL authority and
Drizzle mapping. For this spec pin:

```bash
FLICKNOTE_TEST_FB_CHECKOUT=/Users/neil/code/guion/flick-backend \
FLICKNOTE_TEST_FB_REVISION=af59e65909365f5c0902daf80bd07734775e0b8e \
scripts/test-private-pg.sh
```

The container-backed harness runs `scripts/test-project-description-pg.py` against
only its newly owned container, followed by private MCP/PG Rust tests. SQL checks
cover active/archived exact migration, full row/extraction preservation, owner
isolation, assigned/none, new/unknown shapes, nonstring/conflicting descriptions,
preimage races/new targets, post-update assertion rollback, dry run, rerun no-op,
full restoration and rollback skipping later edits. Rust tests cover the remote
project description adapter and HTTP MCP boundary. Ordinary tests never connect
to live/shared PostgreSQL. Record pinned revision/image, all applied migrations,
owned resource names and cleanup in the implementation report. Passing tests or
packaging does not establish online migration/deployment or native/Keychain acceptance.
