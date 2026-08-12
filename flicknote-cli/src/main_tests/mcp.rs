use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use flicknote_core::backend::{InsertNoteReq, LocalPowerSyncBackend, NoteDb};
use flicknote_core::config::{Config, ConfigPaths};
use flicknote_core::schema::app_schema;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::{
    CreateNote, CreatedNote, NoteCreator, ShareGateway, ShareResource,
};
use flicknote_sync::app::Application;
use flicknote_sync::ipc::{ServerInfo, serve_app, socket_path};
use powersync::{ConnectionPool, PowerSyncDatabase, env::PowerSyncEnvironment};
use rmcp::ServiceExt;
use rusqlite::params;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

use crate::mcp;

struct PersistingCreator {
    db: Arc<dyn NoteDb>,
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

struct McpHarness {
    _directory: tempfile::TempDir,
    writer: WriteHalf<DuplexStream>,
    reader: BufReader<ReadHalf<DuplexStream>>,
    next_id: u64,
    note_uuid: String,
    alpha_id: String,
    server: tokio::task::JoinHandle<()>,
    daemon: tokio::task::JoinHandle<()>,
}

impl McpHarness {
    async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let (backend, note_uuid, alpha_id) = seeded_backend(&config).await;
        let creator: Arc<dyn NoteCreator> = Arc::new(PersistingCreator {
            db: backend.clone(),
        });
        let app = Arc::new(
            Application::new(backend, creator, Arc::new(UnusedShareGateway))
                .with_web_url(config.web_url.clone()),
        );
        let listener = tokio::net::UnixListener::bind(socket_path(&config)).unwrap();
        let daemon = tokio::spawn(async move {
            serve_app(listener, app, ServerInfo::current())
                .await
                .unwrap();
        });
        let service = mcp::FlickNoteMcp::new(Arc::new(config));
        let (server_io, client_io) = tokio::io::duplex(8 * 1024);
        let server = tokio::spawn(async move {
            service
                .serve(server_io)
                .await
                .unwrap()
                .waiting()
                .await
                .unwrap();
        });
        let (reader, mut writer) = tokio::io::split(client_io);
        let mut reader = BufReader::new(reader);
        initialize_mcp(&mut writer, &mut reader).await;
        Self {
            _directory: directory,
            writer,
            reader,
            next_id: 2,
            note_uuid,
            alpha_id,
            server,
            daemon,
        }
    }

    async fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        rpc_request(&mut self.writer, &mut self.reader, id, method, params).await
    }

    async fn call(&mut self, name: &str, arguments: serde_json::Value) -> serde_json::Value {
        self.request(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": arguments }),
        )
        .await
    }

    async fn tools(&mut self) -> Vec<serde_json::Value> {
        self.request("tools/list", serde_json::json!({})).await["result"]["tools"]
            .as_array()
            .unwrap()
            .clone()
    }
}

impl Drop for McpHarness {
    fn drop(&mut self) {
        self.server.abort();
        self.daemon.abort();
    }
}

fn test_config(directory: &std::path::Path) -> Config {
    Config {
        supabase_url: "https://auth.example.test".to_string(),
        supabase_anon_key: "anon-key".to_string(),
        powersync_url: String::new(),
        api_url: "https://gateway.example.test/api/v1".to_string(),
        web_url: Some("https://app.example".to_string()),
        paths: ConfigPaths {
            config_dir: directory.to_path_buf(),
            data_dir: directory.to_path_buf(),
            config_file: directory.join("config.json"),
            session_file: directory.join("session.json"),
            db_file: directory.join("test.db"),
            log_file: directory.join("test.log"),
        },
    }
}

fn test_database(config: &Config) -> PowerSyncDatabase {
    struct NoHttp;

    #[async_trait]
    impl powersync::http::HttpClient for NoHttp {
        async fn send(
            &self,
            _request: powersync::http::Request,
        ) -> Result<powersync::http::Response, powersync::error::PowerSyncError> {
            panic!("MCP tests must not make PowerSync HTTP requests")
        }
    }

    PowerSyncEnvironment::powersync_auto_extension().unwrap();
    let pool = ConnectionPool::open(&config.paths.db_file).unwrap();
    let environment =
        PowerSyncEnvironment::custom(NoHttp, pool, PowerSyncEnvironment::tokio_timer());
    PowerSyncDatabase::new(environment, app_schema())
}

async fn seeded_backend(config: &Config) -> (Arc<LocalPowerSyncBackend>, String, String) {
    let db = test_database(config);
    let backend = Arc::new(LocalPowerSyncBackend::new(
        db.clone(),
        "test-user".to_string(),
    ));
    let project_id = backend.create_project("MCP Project").await.unwrap();
    let note_uuid = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &note_uuid,
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
    let writer = db.writer().await.unwrap();
    writer
        .execute(
            "UPDATE notes SET short_id = 42, source = ? WHERE id = ?",
            params![r#"{"link":{"content":"one\ntwo\nthree"}}"#, note_uuid],
        )
        .unwrap();
    drop(writer);
    let no_source_id = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &no_source_id,
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
    let writer = db.writer().await.unwrap();
    writer
        .execute(
            "UPDATE notes SET short_id = 43 WHERE id = ?",
            params![no_source_id],
        )
        .unwrap();
    drop(writer);
    let alpha_id = flicknote_core::services::markdown::parse_markdown(
        "## Alpha\n\nOld text.\n\n## Beta\n\nKeep me.",
    )
    .headings[0]
        .id
        .clone();
    (backend, note_uuid, alpha_id)
}

async fn initialize_mcp(
    writer: &mut WriteHalf<DuplexStream>,
    reader: &mut BufReader<ReadHalf<DuplexStream>>,
) {
    let initialized = rpc_request(
        writer,
        reader,
        1,
        "initialize",
        serde_json::json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "flicknote-test", "version": "0" }
        }),
    )
    .await;
    assert_eq!(initialized["result"]["serverInfo"]["name"], "flicknote");
    writer
        .write_all(
            concat!(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                "\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
}

async fn rpc_request(
    writer: &mut WriteHalf<DuplexStream>,
    reader: &mut BufReader<ReadHalf<DuplexStream>>,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
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

#[tokio::test]
async fn mcp_server_exposes_stable_tool_contract() {
    let mut harness = McpHarness::start().await;
    let tools = harness.tools().await;
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(names, mcp::EXPECTED_TOOLS.into_iter().collect());
    assert!(tools.iter().all(|tool| tool.get("outputSchema").is_some()));

    let list = tools
        .iter()
        .find(|tool| tool["name"] == "note_list")
        .unwrap();
    assert_eq!(
        list["inputSchema"]["$defs"]["NoteType"]["enum"],
        serde_json::json!(["normal", "meeting", "link"])
    );
    let count = tools
        .iter()
        .find(|tool| tool["name"] == "note_count")
        .unwrap();
    assert_eq!(
        count["inputSchema"]["$defs"]["NoteType"]["enum"],
        serde_json::json!(["normal", "meeting", "link", "file"])
    );
    for tool in tools.iter().filter(|tool| {
        tool["name"]
            .as_str()
            .is_some_and(|name| name.starts_with("note_"))
    }) {
        let schema = &tool["inputSchema"];
        if schema["properties"].get("id").is_some() {
            assert_eq!(schema["properties"]["id"]["type"], "integer");
        }
        assert!(!tool["outputSchema"].to_string().contains("uuid"));
    }
    let project_get = tools
        .iter()
        .find(|tool| tool["name"] == "project_get")
        .unwrap();
    assert!(
        project_get["inputSchema"]["properties"]
            .get("project")
            .is_some()
    );
    assert!(project_get["inputSchema"]["properties"].get("id").is_none());
}

/// Fail if a schema position holds a bare boolean (e.g. `"metadata": true`).
/// Boolean *keyword* values such as `additionalProperties: false` are not
/// schema positions and are left untouched.
fn assert_no_boolean_schema_terms(schema: &serde_json::Value, tool: &str) {
    match schema {
        serde_json::Value::Bool(_) => {
            panic!("tool {tool}: outputSchema contains a boolean schema term")
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let is_named_schemas =
                    matches!(key.as_str(), "properties" | "patternProperties" | "$defs");
                let is_schema_list =
                    matches!(key.as_str(), "allOf" | "anyOf" | "oneOf" | "prefixItems");
                let is_single_schema = matches!(
                    key.as_str(),
                    "items"
                        | "propertyNames"
                        | "contains"
                        | "not"
                        | "if"
                        | "then"
                        | "else"
                        | "unevaluatedItems"
                        | "unevaluatedProperties"
                );
                if is_named_schemas {
                    for subschema in value.as_object().unwrap().values() {
                        assert_no_boolean_schema_terms(subschema, tool);
                    }
                } else if is_schema_list {
                    for subschema in value.as_array().unwrap() {
                        assert_no_boolean_schema_terms(subschema, tool);
                    }
                } else if is_single_schema {
                    assert_no_boolean_schema_terms(value, tool);
                }
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn mcp_tool_output_schemas_are_strict_client_compatible() {
    let mut harness = McpHarness::start().await;
    let tools = harness.tools().await;
    for tool in &tools {
        let name = tool["name"].as_str().unwrap();
        let output = &tool["outputSchema"];
        assert_eq!(
            output["type"],
            serde_json::json!("object"),
            "tool {name}: outputSchema root type must be object"
        );
        assert_no_boolean_schema_terms(output, name);
    }

    let list = tools
        .iter()
        .find(|tool| tool["name"] == "note_list")
        .unwrap();
    assert!(
        list["outputSchema"]["properties"]["notes"]["items"].is_object(),
        "note_list outputSchema must advertise the notes array item schema"
    );
    let projects = tools
        .iter()
        .find(|tool| tool["name"] == "project_list")
        .unwrap();
    assert!(
        projects["outputSchema"]["properties"]["projects"]["items"].is_object(),
        "project_list outputSchema must advertise the projects array item schema"
    );
}

#[tokio::test]
async fn mcp_note_get_output_schema_advertises_detail_structure() {
    let mut harness = McpHarness::start().await;
    let tools = harness.tools().await;
    let note_get = tools
        .iter()
        .find(|tool| tool["name"] == "note_get")
        .unwrap();
    let output = &note_get["outputSchema"];
    let properties = output["properties"].as_object().unwrap();
    for required_property in ["content", "metadata", "extractions", "sections"] {
        assert!(
            properties.contains_key(required_property),
            "note_get outputSchema must advertise {required_property}"
        );
    }
    let required = output["required"].as_array().unwrap();
    for required_field in ["content", "extractions", "sections"] {
        assert!(
            required.iter().any(|field| field == required_field),
            "note_get outputSchema must require {required_field}"
        );
    }
    let metadata = &properties["metadata"];
    assert!(
        metadata.is_object(),
        "note_get metadata schema must be an object-form term, got {metadata}"
    );
    let metadata_types = metadata["type"].as_array().unwrap();
    for json_type in ["null", "object", "array", "string", "number", "boolean"] {
        assert!(
            metadata_types.iter().any(|allowed| allowed == json_type),
            "note_get metadata schema must represent arbitrary JSON including {json_type}"
        );
    }
}

#[tokio::test]
async fn mcp_note_source_output_schema_advertises_all_views() {
    let mut harness = McpHarness::start().await;
    let tools = harness.tools().await;
    let note_source = tools
        .iter()
        .find(|tool| tool["name"] == "note_source")
        .unwrap();
    let output = &note_source["outputSchema"];
    assert_eq!(
        output["type"],
        serde_json::json!("object"),
        "note_source outputSchema root type must be object"
    );
    let variants = output["oneOf"].as_array().unwrap();
    assert_eq!(
        variants.len(),
        3,
        "note_source must advertise rendered, raw, and info variants"
    );
    let mut views = BTreeSet::new();
    let mut rendered_fields = BTreeSet::new();
    let mut raw_value_types = Vec::new();
    let mut info_fields = BTreeSet::new();
    for variant in variants {
        let view_schema = &variant["properties"]["view"];
        let view = view_schema["const"]
            .as_str()
            .or_else(|| {
                view_schema["enum"]
                    .as_array()
                    .and_then(|values| values.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or_else(|| {
                panic!("variant must declare its view discriminator via const or enum")
            });
        views.insert(view.to_string());
        assert!(
            variant["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field == "view"),
            "{view} variant must retain the view discriminator in required"
        );
        let fields: BTreeSet<String> = variant["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        match view {
            "rendered" => rendered_fields = fields,
            "raw" => {
                raw_value_types = variant["properties"]["value"]["type"]
                    .as_array()
                    .unwrap()
                    .clone()
            }
            "info" => info_fields = fields,
            other => panic!("unexpected source view {other}"),
        }
    }
    assert_eq!(
        views,
        BTreeSet::from(["rendered", "raw", "info"].map(String::from))
    );
    for field in [
        "view",
        "source_type",
        "range_unit",
        "total_count",
        "selected_start",
        "selected_end",
        "content",
    ] {
        assert!(
            rendered_fields.contains(field),
            "rendered variant must advertise {field}"
        );
    }
    assert!(
        raw_value_types.contains(&serde_json::json!("null"))
            && raw_value_types.contains(&serde_json::json!("object")),
        "raw variant value must represent arbitrary JSON including null"
    );
    for field in ["view", "source_type", "range_unit", "count"] {
        assert!(
            info_fields.contains(field),
            "info variant must advertise {field}"
        );
    }
}

#[tokio::test]
async fn mcp_note_queries_use_short_ids_and_hide_uuid() {
    let mut harness = McpHarness::start().await;
    let listed = harness.call("note_list", serde_json::json!({})).await;
    assert_eq!(listed["result"]["isError"], false);
    assert_eq!(
        listed["result"]["structuredContent"]["notes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_json_does_not_contain_string(&listed["result"]["structuredContent"], &harness.note_uuid);

    let fetched = harness
        .call("note_get", serde_json::json!({ "id": 42 }))
        .await;
    assert!(fetched["result"]["structuredContent"].get("uuid").is_none());
    assert!(
        fetched["result"]["structuredContent"]
            .get("project_id")
            .is_none()
    );
    let string_id = harness
        .call("note_get", serde_json::json!({ "id": "42" }))
        .await;
    assert_eq!(string_id["result"]["structuredContent"]["id"], 42);
    let uuid = harness.note_uuid.clone();
    let rejected = harness
        .call("note_get", serde_json::json!({ "id": uuid }))
        .await;
    assert_eq!(rejected["result"]["isError"], true);
    assert!(
        rejected["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("invalid note ID")
    );
}

#[tokio::test]
async fn mcp_note_mutations_and_lifecycle_route_through_daemon() {
    let mut harness = McpHarness::start().await;
    let modified = harness
        .call(
            "note_modify",
            serde_json::json!({
                "id": 42,
                "before": "Old text.",
                "after": "New text.",
                "flagged": true
            }),
        )
        .await;
    assert_eq!(
        modified["result"]["structuredContent"]["note"]["flagged"],
        true
    );
    let section = harness.alpha_id.clone();
    harness
        .call(
            "note_replace_section",
            serde_json::json!({
                "id": 42,
                "section": section,
                "content": "## Alpha revised\n\nReplacement text."
            }),
        )
        .await;
    let fetched = harness
        .call("note_get", serde_json::json!({ "id": 42 }))
        .await;
    let content = fetched["result"]["structuredContent"]["content"]
        .as_str()
        .unwrap();
    assert!(content.contains("Replacement text."));
    assert!(content.contains("Keep me."));
    let found = harness
        .call(
            "note_find",
            serde_json::json!({ "keywords": ["Replacement"] }),
        )
        .await;
    assert_eq!(
        found["result"]["structuredContent"]["notes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let added = harness
        .call(
            "note_add",
            serde_json::json!({ "content": "daemon-backed note" }),
        )
        .await;
    assert_eq!(added["result"]["isError"], false);
    let archived = harness
        .call("note_archive", serde_json::json!({ "id": 42 }))
        .await;
    assert_eq!(archived["result"]["structuredContent"]["archived"], true);
    let restored = harness
        .call("note_restore", serde_json::json!({ "id": 42 }))
        .await;
    assert_eq!(restored["result"]["structuredContent"]["archived"], false);
}

#[tokio::test]
async fn mcp_project_and_source_contracts_are_preserved() {
    let mut harness = McpHarness::start().await;
    let projects = harness.call("project_list", serde_json::json!({})).await;
    assert_eq!(
        projects["result"]["structuredContent"]["projects"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        projects["result"]["structuredContent"]["projects"][0]
            .get("id")
            .is_none()
    );
    let project = harness
        .call(
            "project_modify",
            serde_json::json!({ "project": "MCP Project", "color": "#abcdef" }),
        )
        .await;
    assert_eq!(project["result"]["structuredContent"]["color"], "#abcdef");

    let info = harness
        .call(
            "note_source",
            serde_json::json!({ "id": 42, "view": "info" }),
        )
        .await;
    assert_eq!(
        info["result"]["structuredContent"],
        serde_json::json!({
            "view": "info",
            "source_type": "link",
            "range_unit": "line",
            "count": 3
        })
    );
    let range = harness
        .call(
            "note_source",
            serde_json::json!({ "id": 42, "view": "rendered", "range": "2:3" }),
        )
        .await;
    assert_eq!(
        range["result"]["structuredContent"]["content"],
        "two\nthree\n"
    );
    let no_source = harness
        .call(
            "note_source",
            serde_json::json!({ "id": 43, "view": "info" }),
        )
        .await;
    assert_eq!(
        no_source["result"]["structuredContent"]["code"],
        "no_source"
    );
}
