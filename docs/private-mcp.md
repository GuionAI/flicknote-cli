# Private remote MCP operator contract

`flicknote private-mcp` is an explicit foreground server. It uses PostgreSQL
and an OAuth grant verifier; it does not load the local CLI configuration,
login session, PowerSync database, or daemon. Local CLI and Codex MCP continue
to use the daemon and SQLite. This server supplies the remote capability for
spec #2809. It is not yet a production ChatGPT authorization flow: the fb OAuth
Worker must issue and verify full grants bound to the canonical MCP resource,
and fb db-init must provision the PGroonga extension/indexes before deployment.

## Start

Set `FLICKNOTE_PRIVATE_DATABASE_URL` in the server's secret environment, then
run the foreground process with explicit listener and public URLs:

```bash
flicknote private-mcp \
  --listen 0.0.0.0:8080 \
  --resource https://notes.example.com/mcp \
  --issuer https://auth.example.com \
  --verifier https://auth.example.com/mcp/verify \
  --allowed-host notes.example.com \
  --allowed-origin https://chatgpt.com
```

The database URL is read from `FLICKNOTE_PRIVATE_DATABASE_URL` by default; use
`--database-url-env NAME` when the operator supplies another environment key.
The URL is never printed by this command. PostgreSQL connections use `NoTls`
inside the trusted server cluster; set `sslmode=disable` in the database URL.
Database TLS and CA configuration are not supported. Public MCP and OAuth
connections still use HTTPS; outbound HTTP clients use rustls.
`--listen` is the socket inside the container and is independent of the
canonical `--resource` URL. All three public URLs require HTTPS, except
explicit localhost/loopback development URLs. The resource must end at `/mcp`.

Repeat `--allowed-host` and `--allowed-origin` for each exact permitted value.
The server rejects unexpected Host, Origin, `Forwarded`, and `X-Forwarded-*`
headers. A trusted proxy must preserve the configured public Host and strip
forwarding headers before this service. Requests without Origin are accepted;
requests carrying Origin must match an allowed value. There is no wildcard
setting. The endpoint publishes RFC 9728 metadata at
`/.well-known/oauth-protected-resource/mcp`; unauthenticated calls to `/mcp`
receive a Bearer challenge pointing to that metadata and requiring
`flicknote:full`.

## Grant verifier

For every `/mcp` HTTP request, including `initialize`, the server sends a
`POST` to the configured verification URL with the incoming
`Authorization: Bearer <token>` header. A valid response is HTTP 200 JSON:

```json
{
  "ok": true,
  "userId": "11111111-1111-4111-8111-111111111111",
  "scope": ["flicknote:full"],
  "resource": "https://notes.example.com/mcp"
}
```

The UUID, full scope, and exact configured resource are required. Unrelated
response fields are ignored. A 401, 403, `ok:false`, or malformed/missing grant
is rejected with a 401 challenge. Verifier network failure, timeout, or 5xx
returns 503. Verification is performed on every request and never cached;
MCP session IDs convey no authority. The verifier must not accept older partial
grants as full access. There is one full grant for all supported remote tools;
there is no per-tool permission selection.

## Database contract

Use a shared pool with a dedicated login role that can `SET ROLE authenticated`.
Neither the login role nor `authenticated` may be a superuser, a table owner,
or have `BYPASSRLS`; startup requests fail closed if these checks fail. Both
roles must lack membership in an owner role. Grant `authenticated` only the
needed table and schema privileges. The current fb RLS policies use
`user_id = auth.uid()` on `notes`, `projects`, and `note_extractions`; the
adapter installs the verified UUID in `request.jwt.claim.sub` and role in
`request.jwt.claim.role` with transaction-local settings before executing
application SQL. A single transaction connection covers each tool operation.
Failed/cancelled requests roll back or discard the connection. Project assignment
checks ownership and active state in the same transaction.

The adapter expects the current fb columns and short-ID trigger contract:
`notes` has `short_id`, `status`, `is_flagged`, `metadata`, `source`, `project_id`,
timestamps and user ownership; `projects` has `metadata`, `color`,
`is_archived`, timestamps and user ownership; `note_extractions` has
`note_id`, `user_id`, `key`, and `value`, with uniqueness on
`(note_id, key, value)`. Keys are stored and queried verbatim: `::topic`,
`::person`, `::company`, `::location`, and `::product`. Replacing one key
preserves other keys; an empty replacement clears it and duplicate values
produce one row. There is no legacy `type` column or prefix conversion.
A schema mismatch fails the request. The server does not run migrations.

Provision PGroonga and indexes through a separately reviewed fb db-init
migration before production use:

```sql
CREATE EXTENSION IF NOT EXISTS pgroonga;
CREATE INDEX notes_title_pgroonga ON public.notes USING pgroonga (title);
CREATE INDEX notes_summary_pgroonga ON public.notes USING pgroonga (summary);
CREATE INDEX notes_content_pgroonga ON public.notes USING pgroonga (content);
```

Every database node participating in replication or recovery must use the
compatible cnsupa PG18/PGroonga image with the WAL resource manager and crash
safer settings. The peer fixture verified standby-mode WAL replay and indexed
search without REINDEX. Native CNPG/Barman restore is not yet validated;
its restore mode and primary/standby preload settings must be resolved before
deployment. Recovery evidence belongs to the image owner and is separate from
CLI consumer verification. These checks do not establish backup/restore correctness.

## Migration-backed CLI verification

fb owns PostgreSQL DDL through `tanka/charts/db-init/db/migrations/*.sql`,
applied by dbmate. Its `packages/queries/src/drizzle/schema.ts` maps the columns
to TypeScript queries. Inspect both before changing a Rust PostgreSQL query:
the SQL column is `is_flagged`, while Drizzle calls the property `isFlagged`.

The CLI harness applies **all unchanged formal fb migrations**, verifies the
complete `schema_migrations` version set against the source files, and then
runs fb's opt-in `tanka/charts/db-init/db/opt-in/private-mcp.sql`. It does not
copy business-table DDL, filter migrations, or weaken migrated constraints,
triggers, or policies. The infrastructure-only
[`private_pg_bootstrap.sql`](../flicknote-sync/tests/fixtures/private_pg_bootstrap.sql)
creates minimal Auth prerequisites and referenced roles. `auth.uid()` reads
transaction-local request claims, so Alice/Bob isolation exercises real RLS.
The database is named `supabase` and enables `wal_level=logical` for CDC
migrations. Test users are seeded after migrations and MCP preparation; only
the disposable fixture activates the `flicknote_mcp` login without credentials.

Requires Podman (or `CONTAINER_TOOL=docker`), dbmate 2.33.0, Git, Python 3,
Rust and og with read access to fb. Run from this checkout:

```bash
scripts/test-private-pg.sh
```

The default uses `og clone --reference` to obtain current fb `origin/main`,
resolves one commit, and archives that immutable snapshot for the entire run.
It prints `FB_COMMIT`, `FB_DIRTY`, the immutable PostgreSQL image reference and
resolved local image ID, and `FB_MIGRATIONS_APPLIED`. The script owns its fresh
source reference, temporary snapshot and container, and cleans them on exit,
failed migrations or cancellation. Because og's reference destination is fixed,
a pre-existing reference is preserved and causes an actionable error; use a
checkout override in that case. The registered sibling fb checkout is untouched.

Replay a logged commit, or test an explicitly selected local checkout:

```bash
FLICKNOTE_TEST_FB_REVISION=97356591a029a396c8172214f508057d518602ec scripts/test-private-pg.sh
FLICKNOTE_TEST_FB_CHECKOUT=/path/to/fb \
  FLICKNOTE_TEST_FB_REVISION=97356591a029a396c8172214f508057d518602ec scripts/test-private-pg.sh
FLICKNOTE_TEST_FB_CHECKOUT=/path/to/fb scripts/test-private-pg.sh
```

With a revision, the checkout's committed snapshot is used. Without a revision,
the checkout's working `tanka` tree is copied and dirty state is reported; this
is useful for pending migration changes. Overrides are never reset or pulled.
The script defaults to the published, digest-pinned PG18/PGroonga image and
prints its immutable reference. Override `FLICKNOTE_TEST_PG_IMAGE` only for a
compatible disposable test image.
No mutating test accepts a shared/live database URL.

The real PostgreSQL/HTTP suite verifies ordinary text/URL creation, rejection of
draft input, seeded draft get/list, metadata and content edits preserving drafts, short IDs, owner isolation, extraction
replacement, PGroonga search, cancellation, and a failed canonical read whose
insert rolls back without durable rows or false success. Validate harness
failure propagation and container cleanup separately:

```bash
python3 scripts/test-private-pg-harness.py /path/to/fb
```

`--provision-only` applies and checks the entire migration set and MCP
preparation without running Rust tests. Missing or failed migrations fail the
run. This fixture verifies fresh fb provisioning and CLI consumer behavior;
it does not certify fse service migrations, historical upgrades/backfills,
PowerSync download/upload, live OAuth, or disaster recovery. fse keeps its own
service migrations and acceptance responsibilities; this CLI PR changes
neither sibling repository.

Local PowerSync tables remain defined in `flicknote-core/src/schema.rs`.
For synchronized fields, check that schema, both adapters and fb's sync rules;
SQLite and PostgreSQL representations can differ. Shared development databases
are read-only diagnostic references, not fixtures. The host's ordinary fb
postgres-dev image lacks PGroonga and cannot substitute for this fixture.

For changes to columns, SQL adapters, PG setup, RLS, search, or shared database/MCP
behavior that affects PostgreSQL, agents must inspect fb migrations and Drizzle
types/nullability/triggers/RLS, update affected adapters, and run the isolated
migration-backed consumer tests locally. Pin the resolved fb commit with
`FLICKNOTE_TEST_FB_REVISION` for replay and report that revision, image and harness
outcome in implementation and PR evidence. MCP boundary changes also require the
strict-client output-schema contract test. Tests and merge do not deploy the
server; release, deployment and authenticated live acceptance are separate.

## Routine CI and local PostgreSQL verification

GitHub runs the routine Rust/dependency/release-fixture suite daily at 03:17 UTC,
on manual dispatch, and on each release's exact source before any publication.
The release gate requires successful checks before cargo-dist plan, public
GitHub upload/release, and Homebrew updates. See the
[developer workflow](../README.md#routine-verification) and
[release regeneration procedure](../README.md#release).

The PostgreSQL harness never runs in CI. Ordinary workspace tests leave its
integration tests ignored; passing routine CI does not prove PostgreSQL behavior.
Relevant changes require the local isolated harness and pinned revision/result
evidence described above. No private fb CI credentials, private-forge network,
Woodpecker activation or Dagger runner is required. Existing local Git hooks
and the Podman/Docker harness remain available. Configured workflows do not
establish an observed scheduled run or an active branch-protection policy.

## Remote tools and behavior

The remote server advertises `entity_list`, `topic_list`; `note_add`,
`note_append`, `note_archive`, `note_count`, `note_delete_section`, `note_find`,
`note_get`, `note_get_section`, `note_insert`, `note_list`, `note_modify`,
`note_rename_section`, `note_replace_section`, `note_restore`, `note_submit`,
`note_write`; and `project_add`, `project_archive`, `project_get`, `project_list`,
`project_modify`. It reuses the local MCP tool arguments, results, and strict
output schemas. The local server retains its complete tool set; remote sharing,
source/open tools, and host-triggered recall are not advertised.

Local and remote MCP `note_add` accept content and optional project only.
Text creation enters `ai_queued`; recognized URLs create link notes and enter
`source_queued`. Supplied `draft` arguments are rejected before any write.
Human CLI `flicknote add --draft` creates drafts; existing drafts remain readable
and editable through MCP. Content and metadata edits keep
the lifecycle state, and `note_submit` is the explicit draft transition.
MCP creation persists `metadata.created_by_ai: true` independently of client
or session, including link notes while preserving `link.url`. Human creation
omits the marker. Local SQLite and PostgreSQL human filters exclude only JSON
boolean true; missing and false remain included in list, count and find.
Edits, lifecycle actions and AI processing preserve creation classification.
The old `created_by` field is no longer read or written. Historical conversion
is a separately authorized one-time operator action, with previewed owner/IDs,
transactional guards and fresh verification; the server runs no backfill.
Old installed writers can recreate obsolete markers until separately upgraded.

Keyword find uses PGroonga with OR terms, active non-draft notes, project
and creation-time and human filtering, coverage ranking, and segmented snippets. Extraction-only find can
include archived notes; lexical terms cannot combine with archived or
extraction filters. PGroonga and local SQLite FTS do not promise identical
ranking or tokenization.
