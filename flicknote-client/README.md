# flicknote-client

Lightweight typed async Rust client for the existing FlickNote daemon over a
Unix socket. This crate owns the canonical application DTOs, request/response
envelopes, protocol constants, typed result extraction, and client/transport
errors. Its dependency graph contains serialization, schema, error, and Tokio
support; it excludes database, PowerSync, authentication, MCP, and HTTP server
implementations.

## Direct async usage

Run calls inside a Tokio runtime with I/O and time enabled. The caller supplies
the exact endpoint. The human CLI resolves its existing configuration and passes
`data_dir/daemon.sock`; another Rust application can resolve its own endpoint.
Construction only stores the path. It loads no configuration or credentials,
creates no directories, opens no database, and performs no service management.
The daemon must already be running.

```rust,no_run
use std::path::PathBuf;
use flicknote_client::{AppRequest, ClientError, DaemonClient};
use flicknote_client::dto::NoteCountInput;

async fn count_notes(socket: PathBuf) -> Result<u64, ClientError> {
    let client = DaemonClient::new(socket);
    let info = client.health().await?;
    assert_eq!(info.protocol, flicknote_client::PROTOCOL_VERSION);
    client.call(AppRequest::NoteCount(NoteCountInput {
        project: None,
        note_type: None,
        archived: false,
    })).await
}
```

`call<T: AppResult>` extracts the expected typed result. `app` returns the
application response enum; `send_request` provides the lower-level versioned
envelope transport. DTOs live in `dto`, `source`, and `editable_document`.
`ClientError` preserves error codes, retryability, and remote JSON details
without depending on backend errors.

## Ownership and failure semantics

The daemon retains storage, PowerSync, credentials, MCP/HTTP serving, parsing,
business rules, and service lifecycle. Storage/Markdown projections and internal
error conversions remain in core/sync. The CLI and daemon use these canonical
client types directly, with no legacy type paths or compatibility re-exports.

The wire protocol and existing CLI/MCP JSON and schema contracts are unchanged.
Connection and health waits remain bounded; ordinary reads retain their existing
transport backstop. Human recall has a five-second whole-call deadline, and
command-hook/MCP recall has three seconds, enforced at those entrypoints.
Mutations have no automatic retry or automatic send/response timeout after
connection. If a response is lost, malformed, incomplete, or unexpected after a
mutation, `daemon_request_outcome_unknown` is non-retryable: inspect state before
deciding whether to repeat the operation. Cancelling a mutation future does not
prove the daemon rolled it back.

## Standalone verification

From the workspace root:

```bash
cargo build -p flicknote-client
cargo test -p flicknote-client
cargo tree -p flicknote-client --edges normal
```

Tests use fake temporary sockets. The Rust example above is included in crate
documentation and compiled by the documentation tests without contacting a
daemon. This library is groundwork for future Rust clients; it does not include
a GPUI or desktop app.
