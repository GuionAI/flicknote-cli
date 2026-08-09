use async_trait::async_trait;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::{
    CreateNote, CreatedNote, NoteCreator, ShareGateway, ShareResource,
};

use super::*;

struct PersistingCreator {
    db: std::sync::Arc<dyn flicknote_core::backend::NoteDb>,
}

#[async_trait]
impl NoteCreator for PersistingCreator {
    async fn create(&self, request: CreateNote) -> Result<CreatedNote, ServiceError> {
        let inserted = self.db.insert_note(&request.as_insert_request()).await?;
        Ok(CreatedNote {
            inserted,
            confirmed_extraction_ids: Vec::new(),
        })
    }
}

struct UnusedShareGateway;

#[async_trait]
impl ShareGateway for UnusedShareGateway {
    async fn share(&self, _resource: ShareResource, _id: &str) -> Result<String, ServiceError> {
        Err(ServiceError::Daemon("unexpected share".to_string()))
    }

    async fn unshare(&self, _resource: ShareResource, _id: &str) -> Result<(), ServiceError> {
        Err(ServiceError::Daemon("unexpected unshare".to_string()))
    }
}

async fn call_mcp_tool(
    writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>,
    reader: &mut tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    id: u64,
    name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    });
    writer
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let mut response = String::new();
    reader.read_line(&mut response).await.unwrap();
    serde_json::from_str(&response).unwrap()
}

fn assert_json_does_not_contain_string(value: &serde_json::Value, excluded: &str) {
    match value {
        serde_json::Value::String(actual) => assert_ne!(actual, excluded),
        serde_json::Value::Array(values) => {
            for value in values {
                assert_json_does_not_contain_string(value, excluded);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                assert_json_does_not_contain_string(value, excluded);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

#[tokio::test(flavor = "current_thread")]
async fn mcp_server_lists_contract_and_calls_note_list() {
    use flicknote_core::backend::{NoteDb, SqliteBackend};
    use flicknote_core::db::Database;
    use flicknote_sync::app::Application;
    use flicknote_sync::ipc::{ServerInfo, serve_app, socket_path};
    use rmcp::ServiceExt;
    use std::rc::Rc;
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().unwrap();
                let config = Config {
                    supabase_url: "https://auth.example.test".to_string(),
                    supabase_anon_key: "anon-key".to_string(),
                    powersync_url: String::new(),
                    api_url: "https://gateway.example.test/api/v1".to_string(),
                    web_url: Some("https://app.example".to_string()),
                    paths: flicknote_core::config::ConfigPaths {
                        config_dir: directory.path().to_path_buf(),
                        data_dir: directory.path().to_path_buf(),
                        config_file: directory.path().join("config.json"),
                        session_file: directory.path().join("session.json"),
                        db_file: directory.path().join("test.db"),
                        log_file: directory.path().join("test.log"),
                    },
                };
                let database = Database::open_local(&config).await.unwrap();
                let backend = Arc::new(SqliteBackend {
                    db: database,
                    user_id: "test-user".to_string(),
                });
                let project_id = backend.create_project("MCP Project").await.unwrap();
                let note_id = uuid::Uuid::new_v4().to_string();
                backend
                    .insert_note(&flicknote_core::backend::InsertNoteReq {
                        id: &note_id,
                        note_type: "normal",
                        status: "synced",
                        title: Some("MCP Note"),
                        content: Some("## Alpha\n\nOld text.\n\n## Beta\n\nKeep me."),
                        metadata: None,
                        project_id: Some(&project_id),
                        now: "2026-08-05T00:00:00Z",
                    })
                    .await
                    .unwrap();
                sqlx::query("UPDATE notes SET short_id = 42 WHERE id = ?")
                    .bind(&note_id)
                    .execute(&backend.db.pool)
                    .await
                    .unwrap();
                sqlx::query("UPDATE notes SET source = ? WHERE id = ?")
                    .bind(r#"{"link":{"content":"one\ntwo\nthree"}}"#)
                    .bind(&note_id)
                    .execute(&backend.db.pool)
                    .await
                    .unwrap();
                let no_source_note_id = uuid::Uuid::new_v4().to_string();
                backend
                    .insert_note(&flicknote_core::backend::InsertNoteReq {
                        id: &no_source_note_id,
                        note_type: "normal",
                        status: "synced",
                        title: Some("No source note"),
                        content: Some("Editable content"),
                        metadata: None,
                        project_id: None,
                        now: "2026-08-05T00:00:00Z",
                    })
                    .await
                    .unwrap();
                sqlx::query("UPDATE notes SET short_id = 43 WHERE id = ?")
                    .bind(&no_source_note_id)
                    .execute(&backend.db.pool)
                    .await
                    .unwrap();
                let alpha_id = flicknote_core::services::markdown::parse_markdown(
                    "## Alpha\n\nOld text.\n\n## Beta\n\nKeep me.",
                )
                .headings[0]
                    .id
                    .clone();
                let creator: Arc<dyn NoteCreator> = Arc::new(PersistingCreator {
                    db: backend.clone(),
                });
                let app = Arc::new(
                    Application::new(
                        backend,
                        creator,
                        Arc::new(UnusedShareGateway),
                    )
                        .with_web_url(config.web_url.clone()),
                );
                let daemon_listener = tokio::net::UnixListener::bind(socket_path(&config)).unwrap();
                let daemon_server = tokio::spawn(serve_app(
                    daemon_listener,
                    app,
                    ServerInfo::current(),
                ));
                let server = mcp::FlickNoteMcp::new(Rc::new(config));
                let (server_io, client_io) = tokio::io::duplex(8 * 1024);
                let server = tokio::task::spawn_local(async move {
                    server.serve(server_io).await.unwrap().waiting().await
                });
                let (client_read, mut client_write) = tokio::io::split(client_io);
                let mut client_read = BufReader::new(client_read);

                client_write
                    .write_all(concat!(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"flicknote-test","version":"0"}}}"#, "\n").as_bytes())
                    .await
                    .unwrap();
                let mut response = String::new();
                client_read.read_line(&mut response).await.unwrap();
                let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
                assert_eq!(parsed["id"], 1);
                assert_eq!(parsed["result"]["serverInfo"]["name"], "flicknote");

                client_write
                    .write_all(concat!(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, "\n", r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#, "\n").as_bytes())
                    .await
                    .unwrap();
                response.clear();
                client_read.read_line(&mut response).await.unwrap();
                let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
                assert_eq!(parsed["id"], 2);
                let tools = parsed["result"]["tools"].as_array().unwrap();
                let names = tools
                    .iter()
                    .map(|tool| tool["name"].as_str().unwrap())
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(names, mcp::EXPECTED_TOOLS.into_iter().collect());
                assert!(!names.contains("gateway_web_search"));
                assert!(!names.contains("gateway_web_fetch"));
                assert!(tools.iter().all(|tool| tool.get("outputSchema").is_some()));
                let list_schema = tools
                    .iter()
                    .find(|tool| tool["name"] == "note_list")
                    .unwrap();
                assert_eq!(
                    list_schema["inputSchema"]["$defs"]["NoteType"]["enum"],
                    serde_json::json!(["normal", "meeting", "link"])
                );
                let count_schema = tools
                    .iter()
                    .find(|tool| tool["name"] == "note_count")
                    .unwrap();
                assert_eq!(
                    count_schema["inputSchema"]["$defs"]["NoteType"]["enum"],
                    serde_json::json!(["normal", "meeting", "link", "file"])
                );
                for tool in tools.iter().filter(|tool| {
                    tool["name"]
                        .as_str()
                        .is_some_and(|name| name.starts_with("note_"))
                }) {
                    let schema = &tool["inputSchema"];
                    if schema["properties"].get("id").is_some() {
                        assert_eq!(
                            schema["properties"]["id"]["type"],
                            "integer",
                            "{} must accept only numeric short IDs",
                            tool["name"]
                        );
                    }
                    assert!(
                        !tool["outputSchema"].to_string().contains("uuid"),
                        "{} output schema must not expose UUID fields",
                        tool["name"]
                    );
                }
                let project_get_schema = tools
                    .iter()
                    .find(|tool| tool["name"] == "project_get")
                    .unwrap();
                assert!(
                    project_get_schema["inputSchema"]["properties"]
                        .get("project")
                        .is_some()
                );
                assert!(
                    project_get_schema["inputSchema"]["properties"]
                        .get("id")
                        .is_none()
                );

                client_write
                    .write_all(concat!(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"note_list","arguments":{}}}"#, "\n").as_bytes())
                    .await
                    .unwrap();
                response.clear();
                client_read.read_line(&mut response).await.unwrap();
                let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
                assert_eq!(parsed["id"], 3);
                assert_eq!(parsed["result"]["isError"], false);
                assert_eq!(parsed["result"]["structuredContent"].as_array().unwrap().len(), 2);
                assert_json_does_not_contain_string(
                    &parsed["result"]["structuredContent"],
                    &note_id,
                );

                let modified = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    4,
                    "note_modify",
                    serde_json::json!({
                        "id": 42,
                        "before": "Old text.",
                        "after": "New text.",
                        "flagged": true
                    }),
                )
                .await;
                assert_eq!(modified["result"]["isError"], false);
                assert_eq!(modified["result"]["structuredContent"]["note"]["flagged"], true);
                assert_json_does_not_contain_string(
                    &modified["result"]["structuredContent"],
                    &note_id,
                );

                let replaced = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    5,
                    "note_replace_section",
                    serde_json::json!({
                        "id": 42,
                        "section": alpha_id,
                        "content": "## Alpha revised\n\nReplacement text."
                    }),
                )
                .await;
                assert_eq!(replaced["result"]["isError"], false);

                let fetched = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    6,
                    "note_get",
                    serde_json::json!({ "id": 42 }),
                )
                .await;
                let content = fetched["result"]["structuredContent"]["content"]
                    .as_str()
                    .unwrap();
                assert!(content.contains("Replacement text."));
                assert!(content.contains("Keep me."));
                assert!(fetched["result"]["structuredContent"].get("uuid").is_none());
                assert!(
                    fetched["result"]["structuredContent"]
                        .get("project_id")
                        .is_none()
                );
                assert_json_does_not_contain_string(
                    &fetched["result"]["structuredContent"],
                    &note_id,
                );

                let string_id_fetched = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    17,
                    "note_get",
                    serde_json::json!({ "id": "42" }),
                )
                .await;
                assert_eq!(string_id_fetched["result"]["isError"], false);
                assert_eq!(string_id_fetched["result"]["structuredContent"]["id"], 42);

                let uuid_rejected = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    15,
                    "note_get",
                    serde_json::json!({ "id": note_id }),
                )
                .await;
                assert_eq!(uuid_rejected["result"]["isError"], true);
                assert!(
                    uuid_rejected["result"]["content"][0]["text"]
                        .as_str()
                        .unwrap()
                        .contains("invalid note ID")
                );

                let found = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    13,
                    "note_find",
                    serde_json::json!({ "keywords": ["Replacement"] }),
                )
                .await;
                assert_eq!(
                    found["result"]["structuredContent"].as_array().unwrap().len(),
                    1
                );

                let projects = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    14,
                    "project_list",
                    serde_json::json!({}),
                )
                .await;
                assert_eq!(
                    projects["result"]["structuredContent"]
                        .as_array()
                        .unwrap()
                        .len(),
                    1
                );
                assert!(
                    projects["result"]["structuredContent"][0]
                        .get("id")
                        .is_none()
                );
                let project = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    7,
                    "project_modify",
                    serde_json::json!({
                        "project": "MCP Project",
                        "color": "#abcdef"
                    }),
                )
                .await;
                assert_eq!(
                    project["result"]["structuredContent"]["color"],
                    "#abcdef"
                );

                let source_info = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    8,
                    "note_source",
                    serde_json::json!({ "id": 42, "view": "info" }),
                )
                .await;
                assert_eq!(
                    source_info["result"]["structuredContent"],
                    serde_json::json!({
                        "view": "info",
                        "source_type": "link",
                        "range_unit": "line",
                        "count": 3
                    })
                );

                let source_range = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    9,
                    "note_source",
                    serde_json::json!({
                        "id": 42,
                        "view": "rendered",
                        "range": "2:3"
                    }),
                )
                .await;
                assert_eq!(
                    source_range["result"]["structuredContent"]["content"],
                    "two\nthree\n"
                );
                assert_eq!(
                    source_range["result"]["structuredContent"]["selected_start"],
                    2
                );

                let no_source = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    16,
                    "note_source",
                    serde_json::json!({ "id": 43, "view": "info" }),
                )
                .await;
                assert_eq!(no_source["result"]["isError"], true);
                assert_eq!(
                    no_source["result"]["structuredContent"]["code"],
                    "no_source"
                );
                assert_eq!(
                    no_source["result"]["content"][0]["text"],
                    "Note has no source data"
                );

                let added = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    10,
                    "note_add",
                    serde_json::json!({ "content": "daemon-backed note" }),
                )
                .await;
                assert_eq!(added["result"]["isError"], false);
                assert_eq!(added["result"]["structuredContent"]["title"], serde_json::Value::Null);

                let archived = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    11,
                    "note_archive",
                    serde_json::json!({ "id": 42 }),
                )
                .await;
                assert_eq!(archived["result"]["structuredContent"]["archived"], true);
                assert_json_does_not_contain_string(
                    &archived["result"]["structuredContent"],
                    &note_id,
                );
                let restored = call_mcp_tool(
                    &mut client_write,
                    &mut client_read,
                    12,
                    "note_restore",
                    serde_json::json!({ "id": 42 }),
                )
                .await;
                assert_eq!(restored["result"]["structuredContent"]["archived"], false);

                drop(client_write);
                drop(client_read);
                server.await.unwrap().unwrap();
                daemon_server.abort();
            })
            .await;
}

#[test]
fn detail_rejects_section_flag() {
    assert!(Cli::try_parse_from(["flicknote", "detail", "abc123", "--section", "a1"]).is_err());
}

#[test]
fn content_rejects_raw_flag() {
    assert!(Cli::try_parse_from(["flicknote", "content", "abc123", "--raw"]).is_err());
}

#[test]
fn skill_install_command_parses() {
    assert!(Cli::try_parse_from(["flicknote", "skill", "install"]).is_ok());
}

#[test]
fn note_share_command_parses() {
    assert!(Cli::try_parse_from(["flicknote", "share", "123"]).is_ok());
}

#[test]
fn project_share_command_parses() {
    assert!(
        Cli::try_parse_from([
            "flicknote",
            "project",
            "share",
            "550e8400-e29b-41d4-a716-446655440000",
        ])
        .is_ok()
    );
}

#[test]
fn note_unshare_command_parses() {
    assert!(Cli::try_parse_from(["flicknote", "unshare", "123"]).is_ok());
}

#[test]
fn project_unshare_command_parses() {
    assert!(
        Cli::try_parse_from([
            "flicknote",
            "project",
            "unshare",
            "550e8400-e29b-41d4-a716-446655440000",
        ])
        .is_ok()
    );
}

#[test]
fn upload_command_parses() {
    assert!(Cli::try_parse_from(["flicknote", "upload", "file.pdf"]).is_ok());
}

#[test]
fn metadata_discovery_and_source_commands_parse() {
    assert!(Cli::try_parse_from(["flicknote", "topic", "list"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "entity", "list", "--type", "person"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "source", "42"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "source", "42", "12:19"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "source", "42", "--json"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "source", "42", "--info"]).is_ok());
    assert!(Cli::try_parse_from(["flicknote", "find", "::topic::AI::person::瓜子"]).is_ok());
}

#[test]
fn note_type_filters_accept_meeting_and_reject_voice() {
    for command in ["list", "count"] {
        assert!(Cli::try_parse_from(["flicknote", command, "--type", "meeting"]).is_ok());
        assert!(Cli::try_parse_from(["flicknote", command, "--type", "voice"]).is_err());
    }
}

#[test]
fn replace_requires_section() {
    assert!(Cli::try_parse_from(["flicknote", "replace", "1"]).is_err());
    assert!(Cli::try_parse_from(["flicknote", "replace", "1", "--section", "a1"]).is_ok());
}

#[test]
fn replace_rejects_metadata_flags() {
    for flag in ["--project", "--flagged", "--unflagged"] {
        let mut argv = vec!["flicknote", "replace", "1", "--section", "a1", flag];
        if flag == "--project" {
            argv.push("work");
        }
        assert!(Cli::try_parse_from(argv).is_err(), "accepted {flag}");
    }
}

#[test]
fn mcp_subcommand_parses() {
    assert!(Cli::try_parse_from(["flicknote", "mcp"]).is_ok());
}
