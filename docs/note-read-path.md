# Note discovery reads

List and extraction-only find share an internal `NoteDiscovery` storage projection.
It contains the existing discovery fields, raw metadata, association IDs and a
UTF-8 byte count, without materializing body or source in Rust. SQLite counts
`length(CAST(content AS BLOB))`; PostgreSQL uses `octet_length(content)`, with
NULL mapped to zero. This includes embedded NUL on SQLite; PostgreSQL text does
not support NUL. PowerSync still stores JSON documents, so this does not promise
to eliminate underlying document/page reads or quantify allocation savings.

Notes are limited before association enrichment. One topic query receives only
page note IDs; one project-name query receives only distinct page project IDs.
There is no account-wide preload, parallel per-note lookup or cross-request cache.
Project names include owned archived projects; missing/foreign projects have no
name. List preserves descending short-ID/cursor pagination; structured find
preserves its existing descending updated-time ordering. Lexical FTS is separate.

SQLite's extraction query disqualifies the global key index with unary `+` on
its TEXT key predicate, allowing the existing note-ID index to bound access.
It preserves key values and NULL semantics; it adds no schema/index or dependency
on a physical PowerSync index name. An owned real-schema regression checks the
query plan and that unrelated extraction growth does not increase VM work.

Detail reads topics and entities together once. Only `::topic` values populate
summary topics, while the full extraction list remains visible. Stored content
moves into the response after section parsing; mutation summaries still load
all their existing fields.

Owned tests cover DTO equivalence, paging, owner isolation and transport contracts.
The explicitly ignored `read_path_work_probe` records statement/VM work and
first/repeated sequential timings for synthetic account/page/body sizes. These
are bounded mechanism evidence, not live latency guarantees or a diagnosis of
host unavailability. PostgreSQL changes additionally require the isolated
migration-backed [private MCP harness](private-mcp.md#migration-backed-cli-verification).
