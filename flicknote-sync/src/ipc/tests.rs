use super::*;
use flicknote_core::config::{Config, ConfigPaths};
use serde_json::json;
use tokio::net::UnixListener;

fn test_config(directory: &std::path::Path) -> Config {
    Config {
        supabase_url: String::new(),
        supabase_anon_key: String::new(),
        powersync_url: String::new(),
        api_url: String::new(),
        web_url: None,
        paths: ConfigPaths {
            config_dir: directory.to_path_buf(),
            data_dir: directory.to_path_buf(),
            config_file: directory.join("config.json"),
            session_file: directory.join("session.json"),
            db_file: directory.join("flicknote.db"),
            log_file: directory.join("sync.log"),
        },
    }
}

async fn serve_response(
    config: &Config,
    response: DaemonResponse,
) -> tokio::task::JoinHandle<DaemonRequest> {
    let listener = UnixListener::bind(socket_path(config)).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await.unwrap();
        write_response(&mut stream, &response).await.unwrap();
        request
    })
}

#[test]
fn socket_path_lives_in_data_dir() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "flicknote-ipc-test-{}-{suffix}",
        std::process::id()
    ));
    let config = Config {
        supabase_url: String::new(),
        supabase_anon_key: String::new(),
        powersync_url: String::new(),
        api_url: String::new(),
        web_url: None,
        paths: ConfigPaths {
            config_dir: dir.clone(),
            data_dir: dir.clone(),
            config_file: dir.join("config.json"),
            session_file: dir.join("session.json"),
            db_file: dir.join("flicknote.db"),
            log_file: dir.join("sync.log"),
        },
    };

    assert_eq!(socket_path(&config), dir.join("sync.sock"));
}

#[test]
fn versioned_health_and_app_requests_have_stable_contracts() {
    assert_eq!(PROTOCOL_VERSION, 2);
    let health = DaemonRequest::Health {
        protocol: PROTOCOL_VERSION,
    };
    assert_eq!(
        serde_json::to_value(health).unwrap(),
        json!({
            "type": "health",
            "payload": { "protocol": PROTOCOL_VERSION }
        })
    );

    let request = DaemonRequest::App {
        protocol: PROTOCOL_VERSION,
        request: Box::new(AppRequest::NoteList(NoteListInput {
            note_type: None,
            project: None,
            archived: false,
            limit: 20,
        })),
    };
    let value = serde_json::to_value(request).unwrap();
    assert_eq!(value["type"], "app");
    assert_eq!(value["payload"]["protocol"], PROTOCOL_VERSION);
    assert!(value["payload"].get("surface").is_none());
    assert_eq!(value["payload"]["request"]["type"], "note_list");
}

#[test]
fn server_info_only_reports_protocol_and_version() {
    let info = ServerInfo::current();
    assert_eq!(info.protocol, PROTOCOL_VERSION);
    assert!(!info.version.is_empty());
    assert_eq!(
        serde_json::to_value(&info).unwrap(),
        json!({
            "protocol": PROTOCOL_VERSION,
            "version": env!("CARGO_PKG_VERSION"),
        })
    );
}

#[test]
fn wire_error_preserves_partial_success_details() {
    let details = json!({"created": true, "short_id": 80});
    let wire = WireError::from_service(ServiceError::Remote {
        code: "note_create_partial".to_string(),
        message: "note created; topics pending".to_string(),
        retryable: false,
        details: Some(details.clone()),
    });

    assert_eq!(wire.code, "note_create_partial");
    assert_eq!(wire.details, Some(details));
}

#[tokio::test]
async fn daemon_client_maps_missing_socket_to_retryable_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());

    let error = DaemonClient::new(&config).health().await.unwrap_err();

    assert_eq!(error.code(), "daemon_unavailable");
    assert!(error.retryable());
    assert!(error.to_string().contains("flicknote sync start"));
}

#[tokio::test]
async fn health_request_has_a_bounded_response_wait() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let listener = UnixListener::bind(socket_path(&config)).unwrap();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    });

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(1_200),
        send_request(
            &config,
            &DaemonRequest::Health {
                protocol: PROTOCOL_VERSION,
            },
        ),
    )
    .await;
    server.abort();

    let response = result.expect("IPC must enforce its own response timeout");
    assert!(matches!(response, Err(DaemonError::Unavailable { .. })));
}

#[test]
fn mutating_application_requests_do_not_have_an_automatic_response_timeout() {
    let request = DaemonRequest::App {
        protocol: PROTOCOL_VERSION,
        request: Box::new(AppRequest::NoteArchive {
            id: "note-1".to_string(),
        }),
    };

    assert_eq!(response_timeout_for(&request), None);
}

#[tokio::test]
async fn daemon_client_preserves_versioned_app_results_and_errors() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let server = serve_response(
        &config,
        DaemonResponse::App(Box::new(AppResponse::NoteCount { count: 7 })),
    )
    .await;
    let response = DaemonClient::new(&config)
        .app(AppRequest::NoteCount(NoteCountInput {
            keywords: Vec::new(),
            project: None,
            note_type: None,
            archived: false,
        }))
        .await
        .unwrap();
    assert!(matches!(response, AppResponse::NoteCount { count: 7 }));
    assert!(matches!(server.await.unwrap(), DaemonRequest::App { .. }));

    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let server = serve_response(
        &config,
        DaemonResponse::AppError(WireError {
            code: "note_not_found".to_string(),
            message: "missing".to_string(),
            retryable: false,
            details: Some(json!({ "id": "42" })),
        }),
    )
    .await;
    let error = DaemonClient::new(&config)
        .app(AppRequest::NoteGet {
            id: "42".to_string(),
            archived: false,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), "note_not_found");
    assert_eq!(error.to_string(), "missing");
    server.await.unwrap();
}

#[tokio::test]
async fn health_rejects_unexpected_daemon_responses() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let server = serve_response(
        &config,
        DaemonResponse::App(Box::new(AppResponse::NoteCount { count: 0 })),
    )
    .await;
    let error = DaemonClient::new(&config).health().await.unwrap_err();
    assert_eq!(error.code(), "daemon_protocol_mismatch");
    assert!(error.to_string().contains("sync stop"));
    server.await.unwrap();
}

#[tokio::test]
async fn protocol_v2_client_rejects_protocol_v1_server_info() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let server = serve_response(
        &config,
        DaemonResponse::ServerInfo(ServerInfo {
            protocol: 1,
            version: "legacy".to_string(),
        }),
    )
    .await;

    let error = DaemonClient::new(&config).health().await.unwrap_err();

    assert_eq!(error.code(), "daemon_protocol_mismatch");
    assert!(error.to_string().contains("sync stop"));
    server.await.unwrap();
}

#[tokio::test]
async fn application_maps_unknown_envelope_to_protocol_mismatch() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let listener = UnixListener::bind(socket_path(&config)).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _request = read_request(&mut stream).await.unwrap();
        write_json(&mut stream, &json!({"type":"legacy_result","payload":{}}))
            .await
            .unwrap();
    });

    let error = DaemonClient::new(&config)
        .app(AppRequest::NoteCount(NoteCountInput {
            keywords: Vec::new(),
            project: None,
            note_type: None,
            archived: false,
        }))
        .await
        .unwrap_err();

    assert_eq!(error.code(), "daemon_protocol_mismatch");
    assert!(!error.retryable());
    server.await.unwrap();
}

#[tokio::test]
async fn mutating_application_maps_incomplete_response_to_unknown_outcome() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let listener = UnixListener::bind(socket_path(&config)).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _request = read_request(&mut stream).await.unwrap();
        stream.write_all(br#"{"type":"app""#).await.unwrap();
        stream.shutdown().await.unwrap();
    });

    let error = DaemonClient::new(&config)
        .app(AppRequest::NoteArchive {
            id: "note-1".to_string(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), "daemon_request_outcome_unknown");
    assert!(!error.retryable());
    server.await.unwrap();
}

#[tokio::test]
async fn malformed_transport_responses_are_classified_by_mutation_safety() {
    for (request, expected_code, retryable) in [
        (
            AppRequest::NoteArchive {
                id: "note-1".to_string(),
            },
            "daemon_request_outcome_unknown",
            false,
        ),
        (
            AppRequest::NoteCount(NoteCountInput {
                keywords: Vec::new(),
                project: None,
                note_type: None,
                archived: false,
            }),
            "daemon_unavailable",
            true,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let listener = UnixListener::bind(socket_path(&config)).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _request = read_request(&mut stream).await.unwrap();
            stream.write_all(b"not-json").await.unwrap();
            stream.shutdown().await.unwrap();
        });

        let error = DaemonClient::new(&config).app(request).await.unwrap_err();

        assert_eq!(error.code(), expected_code);
        assert_eq!(error.retryable(), retryable);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn unexpected_typed_responses_are_classified_by_mutation_safety() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let server = serve_response(&config, DaemonResponse::App(Box::new(AppResponse::Unit))).await;
    let error = DaemonClient::new(&config)
        .call::<u64>(AppRequest::NoteCount(NoteCountInput {
            keywords: Vec::new(),
            project: None,
            note_type: None,
            archived: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "daemon_protocol_mismatch");
    server.await.unwrap();

    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let server = serve_response(&config, DaemonResponse::App(Box::new(AppResponse::Unit))).await;
    let error = DaemonClient::new(&config)
        .call::<NoteArchiveResult>(AppRequest::NoteArchive {
            id: "note-1".to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), "daemon_request_outcome_unknown");
    server.await.unwrap();
}

#[tokio::test]
async fn unexpected_outer_responses_are_classified_by_mutation_safety() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let server = serve_response(&config, DaemonResponse::ServerInfo(ServerInfo::current())).await;

    let error = DaemonClient::new(&config)
        .app(AppRequest::NoteArchive {
            id: "note-1".to_string(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), "daemon_request_outcome_unknown");
    assert!(!error.retryable());
    server.await.unwrap();
}

#[tokio::test]
async fn health_maps_legacy_daemon_error_to_protocol_mismatch() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let listener = UnixListener::bind(socket_path(&config)).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await.unwrap();
        let response = json!({
            "type": "error",
            "payload": {
                "code": "other",
                "message": "Failed to parse daemon request: unknown variant `health`"
            }
        });
        write_json(&mut stream, &response).await.unwrap();
        request
    });

    let error = DaemonClient::new(&config).health().await.unwrap_err();

    assert_eq!(error.code(), "daemon_protocol_mismatch");
    assert!(!error.retryable());
    assert!(error.to_string().contains("sync stop"));
    assert!(matches!(
        server.await.unwrap(),
        DaemonRequest::Health { .. }
    ));
}

#[tokio::test]
async fn health_maps_empty_startup_response_to_retryable_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let listener = UnixListener::bind(socket_path(&config)).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _request = read_request(&mut stream).await.unwrap();
        drop(stream);
    });

    let error = DaemonClient::new(&config).health().await.unwrap_err();

    assert_eq!(error.code(), "daemon_unavailable");
    assert!(error.retryable());
    server.await.unwrap();
}
