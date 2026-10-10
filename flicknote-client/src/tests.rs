use super::*;
use crate::client::response_timeout_for;
use crate::dto::*;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

async fn serve_response(
    config: &std::path::Path,
    response: DaemonResponse,
) -> tokio::task::JoinHandle<DaemonRequest> {
    let listener = UnixListener::bind(config).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await.unwrap();
        write_response(&mut stream, &response).await.unwrap();
        request
    })
}

#[test]
fn versioned_health_and_app_requests_have_stable_contracts() {
    assert_eq!(PROTOCOL_VERSION, 15);
    let health = DaemonRequest::Health {
        protocol: PROTOCOL_VERSION,
    };
    assert_eq!(
        serde_json::to_value(health).unwrap(),
        json!({
            "type": "health",
            "payload": { "protocol": 15 }
        })
    );

    let request = DaemonRequest::App {
        protocol: PROTOCOL_VERSION,
        request: Box::new(AppRequest::NoteList(NoteListInput {
            note_type: None,
            status: Some("ready".to_string()),
            project: None,
            no_project: false,
            created_after: None,
            created_before: None,
            human: false,
            archived: false,
            shared: false,
            limit: 20,
            cursor: None,
        })),
    };
    let value = serde_json::to_value(request).unwrap();
    assert_eq!(value["type"], "app");
    assert_eq!(value["payload"]["protocol"], 15);
    assert!(value["payload"].get("surface").is_none());
    assert_eq!(value["payload"]["request"]["type"], "note_list");
    assert_eq!(value["payload"]["request"]["payload"]["status"], "ready");
    assert_eq!(value["payload"]["request"]["payload"]["human"], false);
    assert_eq!(value["payload"]["request"]["payload"]["shared"], false);
}

#[test]
fn protocol_v15_uses_typed_project_contracts() {
    let project = ProjectDto {
        id: "project-id".to_string(),
        name: "Work".to_string(),
        color: Some("#123456".to_string()),
        description: Some("Project boundary".to_string()),
        archived: false,
        created_at: Some("2026-09-25T00:00:00Z".to_string()),
    };
    let value = serde_json::to_value(AppResponse::Projects(vec![project])).unwrap();
    assert_eq!(value["payload"][0]["description"], "Project boundary");
    assert!(value["payload"][0].get("metadata").is_none());
    assert!(value["payload"][0].get("user_id").is_none());

    let modify = serde_json::to_value(AppRequest::ProjectModify(ProjectModifyInput {
        id: "project-id".to_string(),
        color: Patch::Missing,
        description: Patch::Value("Updated boundary".to_string()),
    }))
    .unwrap();
    assert_eq!(modify["payload"]["description"], "Updated boundary");
    assert!(modify["payload"].get("pinned").is_none());

    assert!(
        serde_json::from_value::<AppRequest>(json!({
            "type": "project_records",
            "payload": { "include_archived": false }
        }))
        .is_err()
    );
}

#[test]
fn server_info_reports_precise_runtime_status_contract() {
    let info = test_server_info();
    assert_eq!(info.protocol, PROTOCOL_VERSION);
    assert!(!info.version.is_empty());
    assert!(!info.executable.is_empty());
    assert_eq!(
        serde_json::to_value(&info).unwrap(),
        json!({
            "protocol": 15,
            "version": env!("CARGO_PKG_VERSION"),
            "executable": info.executable,
            "sync_errors": {
                "download": null,
                "upload": null,
            },
        })
    );
}

#[tokio::test]
async fn daemon_client_maps_missing_socket_to_retryable_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("missing-data-dir/daemon.sock");

    let client = DaemonClient::new(&config);
    assert!(!config.parent().unwrap().exists());
    let error = client.health().await.unwrap_err();
    assert!(!config.parent().unwrap().exists());

    assert_eq!(error.code(), "daemon_unavailable");
    assert!(error.retryable());
    assert!(error.to_string().contains("flicknote daemon start"));
    assert!(!error.to_string().contains("daemon.sock"));
}

#[tokio::test]
async fn health_request_has_a_bounded_response_wait() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&config).unwrap();
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

#[test]
fn recall_application_requests_use_the_long_generic_transport_guard() {
    let request = DaemonRequest::App {
        protocol: PROTOCOL_VERSION,
        request: Box::new(AppRequest::NoteRecall {
            prompt: "Ada".to_string(),
            project: None,
        }),
    };

    assert_eq!(
        response_timeout_for(&request),
        Some(std::time::Duration::from_secs(300))
    );
}

#[tokio::test]
async fn daemon_client_preserves_versioned_app_results_and_errors() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("daemon.sock");
    let server = serve_response(
        &config,
        DaemonResponse::App(Box::new(AppResponse::NoteCount { count: 7 })),
    )
    .await;
    let response = DaemonClient::new(&config)
        .app(AppRequest::NoteCount(NoteCountInput {
            project: None,
            note_type: None,
            archived: false,
            human: false,
        }))
        .await
        .unwrap();
    assert!(matches!(response, AppResponse::NoteCount { count: 7 }));
    assert!(matches!(server.await.unwrap(), DaemonRequest::App { .. }));

    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("daemon.sock");
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
    let config = directory.path().join("daemon.sock");
    let server = serve_response(
        &config,
        DaemonResponse::App(Box::new(AppResponse::NoteCount { count: 0 })),
    )
    .await;
    let error = DaemonClient::new(&config).health().await.unwrap_err();
    assert_eq!(error.code(), PROTOCOL_MISMATCH_CODE);
    assert!(error.to_string().contains("daemon restart"));
    server.await.unwrap();
}

#[tokio::test]
async fn protocol_v15_client_rejects_protocol_v14_server_info() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("daemon.sock");
    let server = serve_response(
        &config,
        DaemonResponse::ServerInfo(ServerInfo {
            protocol: 14,
            version: "legacy".to_string(),
            executable: "/opt/legacy/flicknote".to_string(),
            sync: None,
            sync_errors: PowerSyncErrors::default(),
            search: None,
        }),
    )
    .await;

    let error = DaemonClient::new(&config).health().await.unwrap_err();

    assert_eq!(error.code(), PROTOCOL_MISMATCH_CODE);
    let message = error.to_string();
    assert!(message.contains(&format!(
        "CLI version {} protocol 15",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(message.contains("daemon executable /opt/legacy/flicknote"));
    assert!(message.contains("daemon version legacy protocol 14"));
    assert!(message.contains("daemon restart"));
    server.await.unwrap();
}

#[tokio::test]
async fn health_preserves_protocol_mismatch_details_from_daemon() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("daemon.sock");
    let server = serve_response(
        &config,
        DaemonResponse::AppError(WireError {
            code: PROTOCOL_MISMATCH_CODE.to_string(),
            message: "old daemon".to_string(),
            retryable: false,
            details: Some(json!({
                "daemon_executable": "/usr/local/bin/flicknote",
                "daemon_version": "0.8.0",
                "daemon_protocol": 2
            })),
        }),
    )
    .await;

    let error = DaemonClient::new(&config).health().await.unwrap_err();

    assert_eq!(error.code(), PROTOCOL_MISMATCH_CODE);
    match error {
        ClientError::Remote { details, .. } => {
            let details = details.unwrap();
            assert_eq!(details["daemon_executable"], "/usr/local/bin/flicknote");
            assert_eq!(details["daemon_version"], "0.8.0");
        }
        other => panic!("expected remote protocol details, got {other:?}"),
    }
    server.await.unwrap();
}

#[tokio::test]
async fn application_maps_unknown_envelope_to_protocol_mismatch() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&config).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _request = read_request(&mut stream).await.unwrap();
        write_json(&mut stream, &json!({"type":"legacy_result","payload":{}}))
            .await
            .unwrap();
    });

    let error = DaemonClient::new(&config)
        .app(AppRequest::NoteCount(NoteCountInput {
            project: None,
            note_type: None,
            archived: false,
            human: false,
        }))
        .await
        .unwrap_err();

    assert_eq!(error.code(), PROTOCOL_MISMATCH_CODE);
    assert!(!error.retryable());
    server.await.unwrap();
}

#[tokio::test]
async fn mutating_application_maps_incomplete_response_to_unknown_outcome() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&config).unwrap();
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
                project: None,
                note_type: None,
                archived: false,
                human: false,
            }),
            "daemon_unavailable",
            true,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("daemon.sock");
        let listener = UnixListener::bind(&config).unwrap();
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
    let config = directory.path().join("daemon.sock");
    let server = serve_response(
        &config,
        DaemonResponse::App(Box::new(AppResponse::Values(Vec::new()))),
    )
    .await;
    let error = DaemonClient::new(&config)
        .call::<u64>(AppRequest::NoteCount(NoteCountInput {
            project: None,
            note_type: None,
            archived: false,
            human: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), PROTOCOL_MISMATCH_CODE);
    server.await.unwrap();

    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("daemon.sock");
    let server = serve_response(
        &config,
        DaemonResponse::App(Box::new(AppResponse::Values(Vec::new()))),
    )
    .await;
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
    let config = directory.path().join("daemon.sock");
    let server = serve_response(&config, DaemonResponse::ServerInfo(test_server_info())).await;

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
    let config = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&config).unwrap();
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

    assert_eq!(error.code(), PROTOCOL_MISMATCH_CODE);
    assert!(!error.retryable());
    assert!(error.to_string().contains("daemon restart"));
    assert!(matches!(
        server.await.unwrap(),
        DaemonRequest::Health { .. }
    ));
}

#[tokio::test]
async fn health_maps_empty_startup_response_to_retryable_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&config).unwrap();
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

fn test_server_info() -> ServerInfo {
    ServerInfo {
        protocol: PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").into(),
        executable: "fake-daemon".into(),
        sync: None,
        sync_errors: PowerSyncErrors::default(),
        search: None,
    }
}

async fn read_request(
    stream: &mut tokio::net::UnixStream,
) -> Result<DaemonRequest, std::io::Error> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn write_response(
    stream: &mut tokio::net::UnixStream,
    response: &DaemonResponse,
) -> Result<(), std::io::Error> {
    write_json(stream, response).await
}

async fn write_json(
    stream: &mut tokio::net::UnixStream,
    response: &(impl serde::Serialize + Sync),
) -> Result<(), std::io::Error> {
    stream.write_all(&serde_json::to_vec(response)?).await?;
    stream.shutdown().await
}
