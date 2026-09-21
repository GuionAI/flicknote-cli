use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::test_support::*;

#[tokio::test]
async fn share_request_lock_serializes_operations() {
    let lock = Arc::new(ShareRequestLock::default());
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));

    let operation = || {
        let lock = Arc::clone(&lock);
        let active = Arc::clone(&active);
        let max_active = Arc::clone(&max_active);
        async move {
            lock.run(async {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_active.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            })
            .await;
        }
    };

    tokio::join!(operation(), operation());

    assert_eq!(max_active.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn returns_existing_note_share_without_replacing_it() {
    let (api_origin, server) = spawn_server(vec![(
        "200 OK",
        r#"{"token":"existing","url":"https://flicknote.app/s/existing"}"#,
    )]);
    let config = test_config(
        format!("{api_origin}/api/v1"),
        "http://127.0.0.1:1".to_string(),
    );
    let request = ShareRequest {
        resource: ShareResource::Note,
        id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
    };

    let url =
        get_or_create_share_with_token(&reqwest::Client::new(), &config, "access-token", &request)
            .await
            .unwrap();

    assert_eq!(url, "https://flicknote.app/s/existing");
    assert_eq!(
        server.join().unwrap(),
        ["GET /api/v1/notes/550e8400-e29b-41d4-a716-446655440000/share HTTP/1.1"]
    );
}

#[tokio::test]
async fn creates_project_share_when_none_exists() {
    let (api_url, server) = spawn_server(vec![
        (
            "404 Not Found",
            r#"{"_tag":"NotFoundError","message":"No project share link exists for this project","errorCode":"PROJECT_SHARE_NOT_FOUND"}"#,
        ),
        (
            "200 OK",
            r#"{"token":"new-token","url":"https://flicknote.app/p/new-token"}"#,
        ),
    ]);
    let config = test_config(api_url, "http://127.0.0.1:1".to_string());
    let request = ShareRequest {
        resource: ShareResource::Project,
        id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
    };

    let url =
        get_or_create_share_with_token(&reqwest::Client::new(), &config, "access-token", &request)
            .await
            .unwrap();

    assert_eq!(url, "https://flicknote.app/p/new-token");
    assert_eq!(
        server.join().unwrap(),
        [
            "GET /api/v1/projects/550e8400-e29b-41d4-a716-446655440000/share HTTP/1.1",
            "POST /api/v1/projects/550e8400-e29b-41d4-a716-446655440000/share HTTP/1.1",
        ]
    );
}

#[tokio::test]
async fn revokes_existing_note_share() {
    let (api_url, server) = spawn_server(vec![("200 OK", r#"{"success":true}"#)]);
    let config = test_config(api_url, "http://127.0.0.1:1".to_string());
    let request = ShareRequest {
        resource: ShareResource::Note,
        id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
    };

    revoke_share_with_token(&reqwest::Client::new(), &config, "access-token", &request)
        .await
        .unwrap();

    assert_eq!(
        server.join().unwrap(),
        ["DELETE /api/v1/notes/550e8400-e29b-41d4-a716-446655440000/share HTTP/1.1"]
    );
}
