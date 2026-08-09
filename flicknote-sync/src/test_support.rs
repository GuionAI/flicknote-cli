use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;

use flicknote_core::{
    REMOTE_COMMITTED_INSERT_METADATA,
    config::{Config, ConfigPaths},
    schema::app_schema,
};
use powersync::{ConnectionPool, PowerSyncDatabase, env::PowerSyncEnvironment};
use rusqlite::params;

pub(crate) async fn test_powersync_db() -> (tempfile::TempDir, PowerSyncDatabase) {
    PowerSyncEnvironment::powersync_auto_extension().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let db = test_powersync_db_at(directory.path().join("test.db"), app_schema());
    db.writer().await.unwrap();
    (directory, db)
}

pub(crate) fn test_powersync_db_at(
    path: impl AsRef<std::path::Path>,
    schema: powersync::schema::Schema,
) -> PowerSyncDatabase {
    PowerSyncEnvironment::powersync_auto_extension().unwrap();
    let pool = ConnectionPool::open(path).unwrap();
    let env = PowerSyncEnvironment::custom(
        reqwest::Client::new(),
        pool,
        PowerSyncEnvironment::tokio_timer(),
    );
    PowerSyncDatabase::new(env, schema)
}

pub(crate) async fn insert_note_with_metadata(db: &PowerSyncDatabase, metadata: &str) {
    let writer = db.writer().await.unwrap();
    writer
        .execute(
            r#"INSERT INTO notes (
                id, short_id, user_id, type, status, title, content,
                is_flagged, created_at, updated_at, _metadata
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            params![
                "note-1",
                42,
                "user-1",
                "normal",
                "ai_queued",
                "Title",
                "Body",
                0,
                "2026-08-09T00:00:00Z",
                "2026-08-09T00:00:00Z",
                metadata,
            ],
        )
        .unwrap();
}

pub(crate) async fn insert_marked_note(db: &PowerSyncDatabase) {
    insert_note_with_metadata(db, REMOTE_COMMITTED_INSERT_METADATA).await;
}

pub(crate) fn test_config(api_url: String) -> Config {
    Config {
        supabase_url: String::new(),
        supabase_anon_key: String::new(),
        powersync_url: String::new(),
        api_url,
        web_url: None,
        paths: ConfigPaths {
            config_dir: PathBuf::new(),
            data_dir: PathBuf::new(),
            config_file: PathBuf::new(),
            session_file: PathBuf::new(),
            db_file: PathBuf::new(),
            log_file: PathBuf::new(),
        },
    }
}

pub(crate) fn spawn_server(
    responses: Vec<(&'static str, &'static str)>,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let count = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..count]);
            requests.push(request.lines().next().unwrap_or_default().to_string());
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        requests
    });
    (format!("http://{address}"), handle)
}

pub(crate) fn read_complete_http_request(stream: &mut std::net::TcpStream) -> String {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    let mut request = Vec::new();
    loop {
        let mut buffer = [0_u8; 4096];
        let count = stream.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
        let Some(headers_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..headers_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        if request.len() >= headers_end + 4 + content_length {
            break;
        }
    }
    String::from_utf8(request).unwrap()
}

pub(crate) fn spawn_capture_server(
    expected_requests: usize,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().unwrap();
            requests.push(read_complete_http_request(&mut stream));
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        }
        requests
    });
    (format!("http://{address}"), handle)
}

pub(crate) fn spawn_disconnected_response_then_server(
    status: &'static str,
    body: &'static str,
) -> (String, thread::JoinHandle<Vec<String>>) {
    spawn_disconnected_then_retry_responses(vec![(status, body)])
}

pub(crate) fn spawn_disconnected_then_retry_responses(
    responses: Vec<(&'static str, &'static str)>,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        listener.set_nonblocking(true).unwrap();
        let accept = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                match listener.accept() {
                    Ok(pair) => return Some(pair),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return None;
                        }
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            }
        };

        let mut requests = Vec::new();
        let mut buffer = [0_u8; 4096];
        let count = first.read(&mut buffer).unwrap();
        requests.push(
            String::from_utf8_lossy(&buffer[..count])
                .lines()
                .next()
                .unwrap_or_default()
                .to_string(),
        );
        drop(first);

        for (status, body) in responses {
            let Some((mut stream, _)) = accept() else {
                break;
            };
            let count = stream.read(&mut buffer).unwrap();
            requests.push(
                String::from_utf8_lossy(&buffer[..count])
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            );
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        requests
    });
    (format!("http://{address}"), handle)
}
