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
The URL is never printed by this command. PostgreSQL TLS can be selected with
its connection URL `sslmode` and must be configured for the target database.
The Linux musl binary includes static OpenSSL, whose default CA location is
`/usr/local/ssl`. For PostgreSQL TLS, provide the trusted PEM CA bundle through
`SSL_CERT_FILE` (for example, `/etc/ssl/certs/ca-certificates.crt` when installed
in the runtime image). Include the database's CA for a private certificate;
certificate and hostname verification remain enabled.
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
`notes` has `short_id`, `status`, `flag`, `metadata`, `source`, `project_id`,
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
deployment. The image's recovery evidence is a separate cross-repository
acceptance gate. The test-owned compatible fixture is
[`flicknote-sync/tests/fixtures/private_pg.sql`](../flicknote-sync/tests/fixtures/private_pg.sql).
Run the current-schema extraction/RLS and MCP regression with the published
immutable peer image (or another compatible test image via the same override):

```bash
FLICKNOTE_TEST_PG_IMAGE=ghcr.io/guionai/cloudnative-supabase-postgres-pgroonga@sha256:d555bf68fad60626664e22cf90390fc8936b42092d504f7f02a592ec70b92626 \
  scripts/test-private-pg.sh
```

The script creates and removes its own container and database. The integration test uses
real PGroonga and a scripted local verifier. This PR does not deploy or mutate
any live database.

## Remote tools and behavior

The remote server advertises `entity_list`, `topic_list`; `note_add`,
`note_append`, `note_archive`, `note_count`, `note_delete_section`, `note_find`,
`note_get`, `note_get_section`, `note_insert`, `note_list`, `note_modify`,
`note_rename_section`, `note_replace_section`, `note_restore`, `note_submit`,
`note_write`; and `project_add`, `project_archive`, `project_get`, `project_list`,
`project_modify`. It reuses the local MCP tool arguments, results, and strict
output schemas. The local server retains its complete tool set; remote sharing,
source/open tools, and host-triggered recall are not advertised.

Normal text creation enters `ai_queued`; recognized URLs enter
`source_queued`; `draft:true` creates a draft. Content and metadata edits keep
the lifecycle state, and `note_submit` is the explicit draft transition.
Keyword find uses PGroonga with OR terms, active non-draft notes, project
and creation-time and human filtering, coverage ranking, and segmented snippets. Extraction-only find can
include archived notes; lexical terms cannot combine with archived or
extraction filters. PGroonga and local SQLite FTS do not promise identical
ranking or tokenization.
