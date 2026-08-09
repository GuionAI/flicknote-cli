use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use flicknote_core::backend::{InsertNoteReq, NoteDb, SqliteBackend};
use flicknote_core::config::{Config, ConfigPaths};
use flicknote_core::db::Database;
use flicknote_sync::app::Application;
use flicknote_sync::ipc::{BackendMode, ServerInfo, serve_app, socket_path};

fn test_config(config_root: &std::path::Path, data_root: &std::path::Path) -> Config {
    let config_dir = config_root.join("flicknote");
    let data_dir = data_root.join("flicknote");
    Config {
        supabase_url: String::new(),
        supabase_anon_key: String::new(),
        powersync_url: String::new(),
        api_url: String::new(),
        web_url: None,
        paths: ConfigPaths {
            config_file: config_dir.join("config.json"),
            session_file: config_dir.join("session.json"),
            db_file: data_dir.join("flicknote.db"),
            log_file: data_dir.join("flicknote.log"),
            config_dir,
            data_dir,
        },
    }
}

struct DaemonGuard {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _shutdown_result = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn spawn_test_daemon(config_root: &std::path::Path, data_root: &std::path::Path) -> DaemonGuard {
    let config = test_config(config_root, data_root);
    std::fs::create_dir_all(&config.paths.data_dir).unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let backend = std::sync::Arc::new(SqliteBackend {
                    db: Database::open_local(&config).await.unwrap(),
                    user_id: "test-user".to_string(),
                });
                let app = std::sync::Arc::new(Application::new(backend, BackendMode::Local));
                let listener = tokio::net::UnixListener::bind(socket_path(&config)).unwrap();
                ready_tx.send(()).unwrap();
                tokio::select! {
                    _ = shutdown_rx => {}
                    result = serve_app(listener, app, ServerInfo::local()) => result.unwrap(),
                }
            });
    });
    ready_rx.recv().unwrap();
    DaemonGuard {
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    }
}

fn write_session(config_root: &std::path::Path) {
    write_session_with_expiry(config_root, None);
}

fn write_session_with_expiry(config_root: &std::path::Path, expires_at: Option<u64>) {
    let directory = config_root.join("flicknote");
    std::fs::create_dir_all(&directory).unwrap();
    let session = serde_json::json!({
        "access_token": "test-token",
        "refresh_token": "test-refresh",
        "expires_at": expires_at,
        "user": { "id": "test-user", "email": null }
    });
    let wrapper = serde_json::json!({
        "sb-test-auth-token": serde_json::to_string(&session).unwrap()
    });
    std::fs::write(
        directory.join("session.json"),
        serde_json::to_vec(&wrapper).unwrap(),
    )
    .unwrap();
}

async fn seed_workspace(
    config_root: &std::path::Path,
    data_root: &std::path::Path,
) -> (String, String) {
    write_session(config_root);
    let config = test_config(config_root, data_root);
    std::fs::create_dir_all(&config.paths.data_dir).unwrap();
    let database = Database::open_local(&config).await.unwrap();
    let backend = SqliteBackend {
        db: database,
        user_id: "test-user".to_string(),
    };
    let project_id = backend.create_project("Legacy project").await.unwrap();
    let note_id = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &note_id,
            note_type: "normal",
            status: "synced",
            title: Some("Legacy JSON"),
            content: Some("stored body"),
            metadata: None,
            project_id: Some(&project_id),
            now: "2026-08-06T00:00:00Z",
        })
        .await
        .unwrap();
    backend.update_note_flagged(&note_id, true).await.unwrap();
    drop(backend);
    (note_id, project_id)
}

fn run_cli_json(
    config_root: &std::path::Path,
    data_root: &std::path::Path,
    args: &[&str],
) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args(args)
        .env("XDG_CONFIG_HOME", config_root)
        .env("XDG_DATA_HOME", data_root)
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_legacy_note_shape(note: &serde_json::Value, project: &serde_json::Value) {
    let object = note.as_object().unwrap();
    let keys = object
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        keys,
        [
            "content",
            "created_at",
            "deleted_at",
            "id",
            "is_flagged",
            "project",
            "project_id",
            "status",
            "summary",
            "title",
            "type",
            "updated_at",
            "uuid",
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(note["content"], "stored body");
    assert_eq!(note["is_flagged"], 1);
    assert_eq!(&note["project"], project);
}

fn spawn_gateway_server(response: &'static str) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 4096];
        let count = stream.read(&mut buffer).unwrap();
        stream.write_all(response.as_bytes()).unwrap();
        String::from_utf8_lossy(&buffer[..count]).into_owned()
    });
    (format!("http://{address}"), handle)
}

fn spawn_gateway_server_sequence(
    responses: Vec<String>,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        responses
            .into_iter()
            .map(|response| {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0_u8; 4096];
                let count = stream.read(&mut buffer).unwrap();
                stream.write_all(response.as_bytes()).unwrap();
                String::from_utf8_lossy(&buffer[..count]).into_owned()
            })
            .collect()
    });
    (format!("http://{address}"), handle)
}

fn spawn_redirect_target() -> (String, thread::JoinHandle<Option<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buffer = [0_u8; 4096];
                    let count = stream.read(&mut buffer).unwrap();
                    stream
                        .write_all(
                            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .unwrap();
                    return Some(String::from_utf8_lossy(&buffer[..count]).into_owned());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("failed to accept redirect target connection: {error}"),
            }
        }
    });
    (format!("http://{address}"), handle)
}

#[test]
fn gateway_request_writes_a_chunked_sse_response_to_stdout_without_exposing_its_token() {
    let directory = tempfile::tempdir().unwrap();
    let config_root = directory.path().join("config");
    let data_root = directory.path().join("data");
    write_session(&config_root);
    let (origin, server) = spawn_gateway_server(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nD\r\ndata: first\n\n\r\nE\r\ndata: second\n\n\r\n0\r\n\r\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args([
            "gateway",
            "request",
            "--method",
            "POST",
            "--path",
            "/llm/v1/chat/completions",
            "--json",
            r#"{"model":"deepseek-v4-pro"}"#,
        ])
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_DATA_HOME", &data_root)
        .env("FLICKNOTE_API_URL", format!("{origin}/api/v1"))
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = server.join().unwrap();
    assert_eq!(output.stdout, b"data: first\n\ndata: second\n\n");
    assert!(request.starts_with("POST /llm/v1/chat/completions HTTP/1.1\r\n"));
    assert!(request.contains("authorization: Bearer test-token\r\n"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("test-token"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("test-token"));
}

#[test]
fn gateway_request_forwards_piped_request_body_without_rewriting_it() {
    let directory = tempfile::tempdir().unwrap();
    let config_root = directory.path().join("config");
    let data_root = directory.path().join("data");
    write_session(&config_root);
    let (origin, server) =
        spawn_gateway_server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");

    let mut child = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args([
            "gateway",
            "request",
            "--method",
            "POST",
            "--path",
            "/llm/v1/chat/completions",
            "--json",
        ])
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_DATA_HOME", &data_root)
        .env("FLICKNOTE_API_URL", format!("{origin}/api/v1"))
        .env_remove("DATABASE_URL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"model\":\"deepseek-v4-pro\"}\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"{}");
    let request = server.join().unwrap();
    assert!(request.contains("{\"model\":\"deepseek-v4-pro\"}\n"));
    assert!(request.contains("content-type: application/json\r\n"));
}

#[test]
fn gateway_request_rejects_invalid_piped_json_before_sending_it() {
    let directory = tempfile::tempdir().unwrap();
    let config_root = directory.path().join("config");
    let data_root = directory.path().join("data");
    write_session(&config_root);

    let mut child = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args([
            "gateway",
            "request",
            "--method",
            "POST",
            "--path",
            "/web/v1/search",
            "--json",
        ])
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_DATA_HOME", &data_root)
        .env("FLICKNOTE_API_URL", "http://127.0.0.1:9/api/v1")
        .env_remove("DATABASE_URL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{invalid json}\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("JSON error"));
}

#[test]
fn gateway_request_bypasses_system_proxies() {
    let directory = tempfile::tempdir().unwrap();
    let config_root = directory.path().join("config");
    let data_root = directory.path().join("data");
    write_session(&config_root);
    let (origin, gateway_server) =
        spawn_gateway_server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
    let (proxy, _proxy_server) = spawn_gateway_server(
        "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args(["gateway", "request", "--path", "/healthz"])
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_DATA_HOME", &data_root)
        .env("FLICKNOTE_API_URL", format!("{origin}/api/v1"))
        .env("HTTP_PROXY", &proxy)
        .env("http_proxy", &proxy)
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = gateway_server.join().unwrap();
    assert!(request.starts_with("GET /healthz HTTP/1.1\r\n"));
    assert!(request.contains("authorization: Bearer test-token\r\n"));
}

#[test]
fn gateway_request_refreshes_sessions_without_using_system_proxies() {
    let directory = tempfile::tempdir().unwrap();
    let config_root = directory.path().join("config");
    let data_root = directory.path().join("data");
    write_session_with_expiry(&config_root, Some(0));
    let refresh_body = r#"{"access_token":"refreshed-token","refresh_token":"refreshed-refresh","expires_at":4102444800,"user":{"id":"test-user","email":null}}"#;
    let (origin, gateway_server) = spawn_gateway_server_sequence(vec![
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{refresh_body}",
            refresh_body.len()
        ),
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_string(),
    ]);
    let (proxy, _proxy_server) = spawn_gateway_server(
        "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args(["gateway", "request", "--path", "/healthz"])
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_DATA_HOME", &data_root)
        .env("FLICKNOTE_API_URL", format!("{origin}/api/v1"))
        .env("FLICKNOTE_SUPABASE_URL", &origin)
        .env("HTTP_PROXY", &proxy)
        .env("http_proxy", &proxy)
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = gateway_server.join().unwrap();
    assert!(requests[0].starts_with("POST /auth/v1/token?grant_type=refresh_token HTTP/1.1\r\n"));
    assert!(requests[1].starts_with("GET /healthz HTTP/1.1\r\n"));
    assert!(requests[1].contains("authorization: Bearer refreshed-token\r\n"));
}

#[test]
fn gateway_request_does_not_forward_session_refresh_to_redirect_target() {
    let directory = tempfile::tempdir().unwrap();
    let config_root = directory.path().join("config");
    let data_root = directory.path().join("data");
    write_session_with_expiry(&config_root, Some(0));
    let (redirect_target, redirect_server) = spawn_redirect_target();
    let (origin, auth_server) = spawn_gateway_server_sequence(vec![format!(
        "HTTP/1.1 307 Temporary Redirect\r\nLocation: {redirect_target}/capture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )]);

    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args(["gateway", "request", "--path", "/healthz"])
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_DATA_HOME", &data_root)
        .env("FLICKNOTE_API_URL", format!("{origin}/api/v1"))
        .env("FLICKNOTE_SUPABASE_URL", &origin)
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let requests = auth_server.join().unwrap();
    assert!(requests[0].starts_with("POST /auth/v1/token?grant_type=refresh_token HTTP/1.1\r\n"));
    assert!(
        redirect_server.join().unwrap().is_none(),
        "the refresh token request reached the redirect target"
    );
}

#[test]
fn gateway_request_does_not_echo_an_upstream_error_body() {
    let directory = tempfile::tempdir().unwrap();
    let config_root = directory.path().join("config");
    let data_root = directory.path().join("data");
    write_session(&config_root);
    let (origin, server) = spawn_gateway_server(
        "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 10\r\nConnection: close\r\n\r\ntest-token",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args([
            "gateway",
            "request",
            "--method",
            "POST",
            "--path",
            "/web/v1/search",
            "--json",
            r#"{"query":"rust"}"#,
        ])
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_DATA_HOME", &data_root)
        .env("FLICKNOTE_API_URL", format!("{origin}/api/v1"))
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("502"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("test-token"));
    server.join().unwrap();
}

#[test]
fn gateway_request_reports_http_date_retry_after() {
    let directory = tempfile::tempdir().unwrap();
    let config_root = directory.path().join("config");
    let data_root = directory.path().join("data");
    write_session(&config_root);
    let retry_after =
        httpdate::fmt_http_date(std::time::SystemTime::now() + std::time::Duration::from_secs(60));
    let (origin, server) = spawn_gateway_server_sequence(vec![format!(
        "HTTP/1.1 429 Too Many Requests\r\nRetry-After: {retry_after}\r\nContent-Length: 10\r\nConnection: close\r\n\r\ntest-token"
    )]);

    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args(["gateway", "request", "--path", "/healthz"])
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_DATA_HOME", &data_root)
        .env("FLICKNOTE_API_URL", format!("{origin}/api/v1"))
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("HTTP 429"));
    assert!(!stderr.contains("test-token"));
    let seconds = stderr
        .split_once("retry after ")
        .and_then(|(_, value)| value.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok());
    assert!(seconds.is_some_and(|seconds| seconds > 0));
    server.join().unwrap();
}

#[tokio::test]
async fn cli_json_commands_preserve_the_existing_machine_contracts() {
    let directory = tempfile::tempdir().unwrap();
    let config_root = directory.path().join("config");
    let data_root = directory.path().join("data");
    let (note_id, _) = seed_workspace(&config_root, &data_root).await;
    let _daemon = spawn_test_daemon(&config_root, &data_root);

    let listed = run_cli_json(&config_root, &data_root, &["list", "--json"]);
    assert_legacy_note_shape(&listed[0], &serde_json::Value::Null);

    let found = run_cli_json(&config_root, &data_root, &["find", "stored", "--json"]);
    assert_legacy_note_shape(&found[0], &serde_json::Value::Null);

    let detailed = run_cli_json(&config_root, &data_root, &["detail", &note_id, "--json"]);
    assert_legacy_note_shape(
        &detailed,
        &serde_json::Value::String("Legacy project".to_string()),
    );

    let projects = run_cli_json(&config_root, &data_root, &["project", "list", "--json"]);
    let project = projects[0].as_object().unwrap();
    assert!(project.contains_key("user_id"));
    assert!(project.contains_key("is_archived"));
    assert!(!project.contains_key("archived"));
}

#[test]
fn mcp_binary_keeps_stdout_as_json_rpc_frames() {
    let directory = tempfile::tempdir().unwrap();
    let config_root = directory.path().join("config");
    let data_root = directory.path().join("data");
    write_session(&config_root);
    let _daemon = spawn_test_daemon(&config_root, &data_root);

    let mut child = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .arg("mcp")
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_DATA_HOME", &data_root)
        .env_remove("DATABASE_URL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-11-25","capabilities":{{}},"clientInfo":{{"name":"integration-test","version":"0"}}}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#
    )
    .unwrap();
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let frames = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["id"], 1);
    assert_eq!(frames[1]["id"], 2);
    assert_eq!(frames[0]["result"]["serverInfo"]["name"], "flicknote");
    assert!(frames[1]["result"]["tools"].as_array().unwrap().len() > 20);
}

#[test]
fn mcp_requires_daemon_before_protocol_output() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .arg("mcp")
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Sync daemon is unavailable"));
}

#[test]
fn data_commands_require_the_daemon() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .arg("list")
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Sync daemon is unavailable"));
}

#[test]
fn root_help_does_not_require_the_daemon() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
}
