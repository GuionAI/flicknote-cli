//! Daemon-owned, disposable Meilisearch projection of canonical PowerSync notes.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::time::Duration;

use futures_lite::StreamExt;
use powersync::PowerSyncDatabase;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use tokio::sync::{Notify, watch};

const INDEX: &str = "flicknote_notes";
const DEFAULT_PORT: u16 = 7702;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum SearchState {
    Degraded = 0,
    Starting = 1,
    Ready = 2,
}

impl SearchState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Degraded => "degraded/unavailable",
            Self::Starting => "starting/rebuilding",
            Self::Ready => "ready",
        }
    }
}

#[derive(Clone)]
pub(crate) struct SearchProjection {
    state: Arc<AtomicU8>,
    documents: Arc<AtomicUsize>,
    retry: Arc<Notify>,
    client: Arc<MeiliClient>,
}

impl SearchProjection {
    pub(crate) fn new(port: u16, key: String) -> Self {
        Self {
            state: Arc::new(AtomicU8::new(SearchState::Starting as u8)),
            documents: Arc::new(AtomicUsize::new(0)),
            retry: Arc::new(Notify::new()),
            client: Arc::new(MeiliClient {
                base: format!("http://127.0.0.1:{port}"),
                key,
                http: reqwest::Client::builder()
                    .timeout(REQUEST_TIMEOUT)
                    .build()
                    .expect("valid local HTTP client"),
            }),
        }
    }

    pub(crate) fn state(&self) -> SearchState {
        match self.state.load(Ordering::Acquire) {
            1 => SearchState::Starting,
            2 => SearchState::Ready,
            _ => SearchState::Degraded,
        }
    }

    pub(crate) fn document_count(&self) -> usize {
        self.documents.load(Ordering::Acquire)
    }

    pub(crate) fn ready_document_count(&self) -> Option<usize> {
        (self.state() == SearchState::Ready).then(|| self.document_count())
    }

    fn set_state(&self, state: SearchState) {
        self.state.store(state as u8, Ordering::Release);
    }

    pub(crate) async fn search(&self, keywords: &[String], limit: u32) -> Option<Vec<String>> {
        if self.state() != SearchState::Ready {
            return None;
        }
        match self.client.search(keywords, limit).await {
            Ok(ids) => Some(ids),
            Err(error) => {
                log::warn!("Meilisearch query failed; using SQLite fallback: {error}");
                self.set_state(SearchState::Degraded);
                self.retry.notify_one();
                None
            }
        }
    }
}

pub(crate) fn port_from_env(value: Option<&str>) -> Result<u16, String> {
    match value {
        None => Ok(DEFAULT_PORT),
        Some(value) => value
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| format!("invalid FLICKNOTE_MEILI_PORT: {value:?}; expected 1..=65535")),
    }
}

pub(crate) fn discover_binary(path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = path {
        candidates.extend(std::env::split_paths(path).map(|dir| dir.join("meilisearch")));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/meilisearch"),
        PathBuf::from("/usr/local/bin/meilisearch"),
    ]);
    candidates
        .into_iter()
        .find(|candidate| executable(candidate))
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

fn private_key(data_dir: &Path) -> Result<String, String> {
    fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
    let path = data_dir.join("meilisearch.key");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            let key = format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            );
            file.write_all(key.as_bytes())
                .map_err(|error| error.to_string())?;
            Ok(key)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let mut key = String::new();
            fs::File::open(path)
                .and_then(|mut file| file.read_to_string(&mut key))
                .map_err(|error| error.to_string())?;
            if key.len() < 32 {
                return Err("Meilisearch private key is invalid".to_string());
            }
            Ok(key)
        }
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn start(
    db: PowerSyncDatabase,
    user_id: String,
    data_dir: PathBuf,
    shutdown: watch::Receiver<bool>,
) -> Option<(SearchProjection, tokio::task::JoinHandle<()>)> {
    let port = match port_from_env(std::env::var("FLICKNOTE_MEILI_PORT").ok().as_deref()) {
        Ok(port) => port,
        Err(error) => {
            log::warn!("Meilisearch unavailable; SQLite fallback: {error}");
            return None;
        }
    };
    let Some(binary) = discover_binary(std::env::var_os("PATH").as_deref()) else {
        log::warn!("Meilisearch executable not found; SQLite search fallback is active");
        return None;
    };
    let key = match private_key(&data_dir) {
        Ok(key) => key,
        Err(error) => {
            log::warn!("Meilisearch state unavailable; SQLite fallback: {error}");
            return None;
        }
    };
    Some(start_with_binary(
        db, user_id, data_dir, shutdown, port, binary, key,
    ))
}

fn start_with_binary(
    db: PowerSyncDatabase,
    user_id: String,
    data_dir: PathBuf,
    shutdown: watch::Receiver<bool>,
    port: u16,
    binary: PathBuf,
    key: String,
) -> (SearchProjection, tokio::task::JoinHandle<()>) {
    let projection = SearchProjection::new(port, key);
    let worker_projection = projection.clone();
    let worker = tokio::spawn(async move {
        let mut shutdown = shutdown;
        let mut backoff = Duration::from_secs(1);
        loop {
            if *shutdown.borrow() {
                break;
            }
            worker_projection.set_state(SearchState::Starting);
            worker_projection.documents.store(0, Ordering::Release);
            let result = run_worker(
                &worker_projection,
                &db,
                &user_id,
                &data_dir,
                &binary,
                shutdown.clone(),
            )
            .await;
            worker_projection.set_state(SearchState::Degraded);
            if *shutdown.borrow() || result.is_ok() {
                break;
            }
            log::warn!(
                "Meilisearch projection degraded; SQLite fallback; retrying: {}",
                result.unwrap_err()
            );
            tokio::select! {
                () = tokio::time::sleep(backoff) => {},
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
        worker_projection.set_state(SearchState::Degraded);
    });
    (projection, worker)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SearchDocument {
    uuid: String,
    short_id: Option<i64>,
    user_id: String,
    #[serde(rename = "type")]
    note_type: String,
    title: Option<String>,
    summary: Option<String>,
    content: Option<String>,
    project_id: Option<String>,
    updated_at: Option<String>,
    draft: bool,
}

const SNAPSHOT_SQL: &str = "SELECT id, short_id, user_id, type, title, summary, content, project_id, updated_at, status FROM notes WHERE user_id = ? AND deleted_at IS NULL";

// The PowerSync watcher callback requires ownership of its cloned Params.
#[allow(clippy::needless_pass_by_value)]
fn snapshot(
    stmt: &mut rusqlite::Statement<'_>,
    parameters: [String; 1],
) -> Result<HashMap<String, SearchDocument>, powersync::error::PowerSyncError> {
    let rows = stmt.query_map(params![parameters[0]], |row| {
        Ok(SearchDocument {
            uuid: row.get(0)?,
            short_id: row.get(1)?,
            user_id: row.get(2)?,
            note_type: row.get(3)?,
            title: row.get(4)?,
            summary: row.get(5)?,
            content: row.get(6)?,
            project_id: row.get(7)?,
            updated_at: row.get(8)?,
            draft: row.get::<_, String>(9)? == "draft",
        })
    })?;
    let mut documents = HashMap::new();
    for document in rows {
        let document = document?;
        documents.insert(document.uuid.clone(), document);
    }
    Ok(documents)
}

async fn run_worker(
    projection: &SearchProjection,
    db: &PowerSyncDatabase,
    user_id: &str,
    data_dir: &Path,
    binary: &Path,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let mut child = Command::new(binary)
        .arg("--http-addr")
        .arg(projection.client.base.trim_start_matches("http://"))
        .arg("--db-path")
        .arg(data_dir.join("db"))
        .arg("--no-analytics")
        .env("MEILI_ENV", "production")
        .env("MEILI_MASTER_KEY", &projection.client.key)
        .env("MEILI_LOG_LEVEL", "WARN")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("could not launch {}: {error}", binary.display()))?;
    log::info!("Meilisearch child started (pid {:?})", child.id());
    let result = project_until_shutdown(projection, db, user_id, &mut child, &mut shutdown).await;
    if result.is_err() {
        projection.set_state(SearchState::Degraded);
    }
    stop_child(&mut child).await;
    result
}

async fn stop_child(child: &mut Child) {
    match child.try_wait() {
        Ok(Some(status)) => {
            log::info!("Meilisearch child already stopped: {status}");
            return;
        }
        Ok(None) => {}
        Err(error) => log::warn!("Meilisearch child state check failed: {error}"),
    }
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        #[allow(unsafe_code)]
        let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if result != 0 {
            log::debug!(
                "Meilisearch SIGTERM request failed: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = child.start_kill() {
        log::debug!("Meilisearch child termination request: {error}");
    }
    match tokio::time::timeout(Duration::from_secs(4), child.wait()).await {
        Ok(Ok(status)) => log::info!("Meilisearch child stopped: {status}"),
        Ok(Err(error)) => log::warn!("Meilisearch child wait failed: {error}"),
        Err(_) => {
            log::warn!("Meilisearch child did not exit after SIGTERM; forcing stop");
            if let Err(error) = child.start_kill() {
                log::warn!("Meilisearch forced stop failed: {error}");
            }
            if let Err(error) = child.wait().await {
                log::warn!("Meilisearch child wait failed after forced stop: {error}");
            }
        }
    }
}

async fn project_until_shutdown(
    projection: &SearchProjection,
    db: &PowerSyncDatabase,
    user_id: &str,
    child: &mut Child,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let healthy = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if *shutdown.borrow() {
                return Ok(false);
            }
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!("child exited during startup: {status}"));
            }
            if projection.client.healthy().await {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| "health check timed out".to_string())??;
    if !healthy {
        return Ok(());
    }
    projection.client.configure().await?;
    // Rebuild on every startup: a previous child may have exited before its
    // last task completed, and index settings can change across versions.
    projection.client.clear().await?;
    let stream = db.watch_statement(SNAPSHOT_SQL.to_string(), [user_id.to_string()], snapshot);
    tokio::pin!(stream);
    let mut previous = HashMap::new();
    let mut initial = true;
    loop {
        tokio::select! {
            result = stream.next() => {
                let Some(result) = result else { return Err("canonical note watcher stopped".to_string()); };
                let current = result.map_err(|error| format!("canonical note watcher failed: {error}"))?;
                projection.set_state(SearchState::Starting);
                let changes = projection.client.apply_diff(&previous, &current).await
                    .map_err(|error| format!("projection update failed: {error}"))?;
                let documents = current.len();
                projection.documents.store(documents, Ordering::Release);
                if initial {
                    log::info!("meili_projection ready documents={documents}");
                    initial = false;
                } else if changes.upserts != 0 || changes.removals != 0 {
                    log::info!("meili_projection upserts={} removals={} documents={documents}", changes.upserts, changes.removals);
                }
                previous = current;
                projection.set_state(SearchState::Ready);
            }
            result = child.wait() => {
                return Err(format!("Meilisearch child exited unexpectedly: {:?}", result));
            }
            () = projection.retry.notified() => {
                return Err("query failed; rebuilding search projection".to_string());
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
        }
    }
}

struct MeiliClient {
    base: String,
    key: String,
    http: reqwest::Client,
}

struct ProjectionChanges {
    upserts: usize,
    removals: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskAccepted {
    task_uid: u64,
}

#[derive(Deserialize)]
struct TaskStatus {
    status: String,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct SearchHits {
    hits: Vec<SearchHit>,
}

#[derive(Deserialize)]
struct SearchHit {
    uuid: String,
}

impl MeiliClient {
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response, String> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.key);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("HTTP {status}: {body}"));
        }
        Ok(response)
    }

    async fn task(&self, response: reqwest::Response) -> Result<(), String> {
        let accepted: TaskAccepted = response.json().await.map_err(|error| error.to_string())?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            let response = self
                .request(
                    reqwest::Method::GET,
                    &format!("/tasks/{}", accepted.task_uid),
                    None,
                )
                .await?;
            let task: TaskStatus = response.json().await.map_err(|error| error.to_string())?;
            match task.status.as_str() {
                "succeeded" => return Ok(()),
                "failed" | "canceled" => {
                    return Err(format!("task {}: {:?}", task.status, task.error));
                }
                _ => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
        Err("Meilisearch task timed out".to_string())
    }

    async fn healthy(&self) -> bool {
        self.request(reqwest::Method::GET, "/health", None)
            .await
            .is_ok()
    }

    async fn configure(&self) -> Result<(), String> {
        let index_path = format!("/indexes/{INDEX}");
        let response = self
            .http
            .get(format!("{}{}", self.base, index_path))
            .bearer_auth(&self.key)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            let response = self
                .request(
                    reqwest::Method::POST,
                    "/indexes",
                    Some(serde_json::json!({"uid": INDEX, "primaryKey": "uuid"})),
                )
                .await?;
            self.task(response).await?;
        } else if !response.status().is_success() {
            return Err(format!("index lookup failed: {}", response.status()));
        }
        let response = self.request(reqwest::Method::PATCH, &format!("{index_path}/settings"), Some(serde_json::json!({
            "searchableAttributes": ["title", "summary", "content"],
            "rankingRules": ["words", "typo", "proximity", "attributeRank", "sort", "exactness"],
            "typoTolerance": {"disableOnNumbers": true}
        }))).await?;
        self.task(response).await
    }

    async fn clear(&self) -> Result<(), String> {
        let response = self
            .request(
                reqwest::Method::DELETE,
                &format!("/indexes/{INDEX}/documents"),
                None,
            )
            .await?;
        self.task(response).await
    }

    async fn apply_diff(
        &self,
        previous: &HashMap<String, SearchDocument>,
        current: &HashMap<String, SearchDocument>,
    ) -> Result<ProjectionChanges, String> {
        let upserts: Vec<_> = current
            .iter()
            .filter(|(id, document)| previous.get(*id) != Some(*document))
            .map(|(_, document)| document)
            .collect();
        let removals: Vec<_> = previous
            .keys()
            .filter(|id| !current.contains_key(*id))
            .collect();
        if !upserts.is_empty() {
            let response = self
                .request(
                    reqwest::Method::POST,
                    &format!("/indexes/{INDEX}/documents"),
                    Some(serde_json::to_value(&upserts).map_err(|error| error.to_string())?),
                )
                .await?;
            self.task(response).await?;
        }
        if !removals.is_empty() {
            let response = self
                .request(
                    reqwest::Method::POST,
                    &format!("/indexes/{INDEX}/documents/delete-batch"),
                    Some(serde_json::to_value(&removals).map_err(|error| error.to_string())?),
                )
                .await?;
            self.task(response).await?;
        }
        Ok(ProjectionChanges {
            upserts: upserts.len(),
            removals: removals.len(),
        })
    }

    async fn search(&self, keywords: &[String], limit: u32) -> Result<Vec<String>, String> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/indexes/{INDEX}/search"),
                Some(serde_json::json!({
                    "q": keywords.join(" "),
                    "limit": limit,
                    "attributesToRetrieve": ["uuid"]
                })),
            )
            .await?;
        let hits: SearchHits = response.json().await.map_err(|error| error.to_string())?;
        Ok(hits.hits.into_iter().map(|hit| hit.uuid).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Application;
    use crate::ipc::{AppRequest, AppResponse};
    use crate::test_support::{search_log_cursor, search_logs_since, test_powersync_db};
    use async_trait::async_trait;
    use flicknote_core::backend::LocalPowerSyncBackend;
    use flicknote_core::services::dto::{ExtractionFilterDto, NoteFindInput};
    use flicknote_core::services::error::ServiceError;
    use flicknote_core::services::ports::{
        CreateNote, CreatedNote, NoteCreator, ShareGateway, ShareResource,
    };
    use std::net::TcpListener;

    struct UnusedWritePorts;

    #[async_trait]
    impl NoteCreator for UnusedWritePorts {
        async fn create(&self, _request: CreateNote) -> Result<CreatedNote, ServiceError> {
            unreachable!("search test does not create notes through the application")
        }
    }

    #[async_trait]
    impl ShareGateway for UnusedWritePorts {
        async fn share(&self, _resource: ShareResource, _id: &str) -> Result<String, ServiceError> {
            unreachable!("search test does not share notes")
        }

        async fn unshare(&self, _resource: ShareResource, _id: &str) -> Result<(), ServiceError> {
            unreachable!("search test does not unshare notes")
        }
    }

    async fn check_find_backend_logs(projection: &SearchProjection, db: &PowerSyncDatabase) {
        let backend = Arc::new(LocalPowerSyncBackend::new(db.clone(), "user-1".to_string()));
        let app = Application::new(
            backend,
            Arc::new(UnusedWritePorts),
            Arc::new(UnusedWritePorts),
        )
        .with_search(Some(projection.clone()));
        let input = NoteFindInput {
            keywords: vec!["aurora".to_string()],
            extractions: Vec::new(),
            project: None,
            archived: false,
            limit: 20,
        };
        let cursor = search_log_cursor();
        let response = app
            .handle(AppRequest::NoteFind(input.clone()))
            .await
            .unwrap();
        let AppResponse::NoteListItems(items) = response else {
            panic!("expected note discovery items")
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content_bytes, "中文 English aurora".len() as u64);

        let mut structured = input.clone();
        structured.extractions.push(ExtractionFilterDto {
            key: "::topic".to_string(),
            value: "private-value".to_string(),
        });
        app.handle(AppRequest::NoteFind(structured)).await.unwrap();
        let mut archived = input.clone();
        archived.archived = true;
        app.handle(AppRequest::NoteFind(archived)).await.unwrap();
        let mut project = input.clone();
        project.project = Some("missing-project".to_string());
        assert!(app.handle(AppRequest::NoteFind(project)).await.is_err());

        projection.set_state(SearchState::Degraded);
        let fallback = app.handle(AppRequest::NoteFind(input)).await.unwrap();
        projection.set_state(SearchState::Ready);
        let AppResponse::NoteListItems(items) = fallback else {
            panic!("expected SQLite discovery items")
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content_bytes, "中文 English aurora".len() as u64);

        let logs = search_logs_since(cursor);
        assert!(
            logs.iter()
                .any(|line| line.starts_with("note_find backend=meili hits=1 latency_ms="))
        );
        for reason in [
            "structured_query",
            "project_filter",
            "archived_filter",
            "meili_unavailable",
        ] {
            assert!(
                logs.iter()
                    .any(|line| line.contains(&format!("backend=sqlite reason={reason}")))
            );
        }
        assert!(
            logs.iter()
                .all(|line| !line.contains("aurora") && !line.contains("private-"))
        );
    }

    async fn wait_ready(projection: &SearchProjection) {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if projection.state() == SearchState::Ready {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("projection did not become ready");
    }

    async fn seed_search_note(db: &PowerSyncDatabase) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let writer = db.writer().await.unwrap();
        writer.execute(
            "INSERT INTO notes (id, user_id, type, status, content) VALUES (?, 'user-1', 'normal', 'ready', 'Seed document')",
            params![id],
        ).unwrap();
        id
    }

    fn assert_projection_logs(cursor: usize) {
        let logs = search_logs_since(cursor);
        assert!(
            logs.iter()
                .any(|line| line == "meili_projection ready documents=1")
        );
        assert!(
            logs.iter()
                .any(|line| line == "meili_projection upserts=1 removals=0 documents=2")
        );
        assert!(
            logs.iter()
                .any(|line| line == "meili_projection upserts=0 removals=1 documents=1")
        );
    }

    async fn wait_degraded(projection: &SearchProjection) {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if projection.state() == SearchState::Degraded {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("projection did not degrade");
    }

    async fn wait_count(projection: &SearchProjection, expected: usize) {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if projection.state() == SearchState::Ready
                    && projection.document_count() == expected
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("projection document count did not converge");
    }

    async fn wait_hit(projection: &SearchProjection, query: &str, expected: &[&str]) {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let Some(actual) = projection.search(&[query.to_string()], 20).await
                    && actual == expected
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("projection did not converge");
    }

    async fn check_ranking_and_limit(projection: &SearchProjection, db: &PowerSyncDatabase) {
        let title_id = uuid::Uuid::new_v4().to_string();
        let body_id = uuid::Uuid::new_v4().to_string();
        let summary_id = uuid::Uuid::new_v4().to_string();
        let writer = db.writer().await.unwrap();
        for (note_id, title, summary, content) in [
            (&title_id, "comet", "none", "ordinary"),
            (&body_id, "ordinary", "none", "comet nebula"),
            (&summary_id, "ordinary", "nebula", "ordinary"),
        ] {
            writer.execute(
                "INSERT INTO notes (id, user_id, type, status, title, summary, content) VALUES (?, 'user-1', 'normal', 'ready', ?, ?, ?)",
                params![note_id, title, summary, content],
            ).unwrap();
        }
        drop(writer);
        wait_hit(projection, "comet", &[&title_id, &body_id]).await;
        wait_hit(projection, "nebula", &[&summary_id, &body_id]).await;
        assert_eq!(
            projection.search(&["comet".to_string()], 1).await.unwrap(),
            vec![title_id]
        );
    }

    async fn projected_document(projection: &SearchProjection, id: &str) -> serde_json::Value {
        projection
            .client
            .request(
                reqwest::Method::GET,
                &format!("/indexes/{INDEX}/documents/{id}"),
                None,
            )
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn check_rebuild_after_update_failure(
        projection: &SearchProjection,
        db: &PowerSyncDatabase,
        id: &str,
    ) {
        let response = projection
            .client
            .request(reqwest::Method::DELETE, &format!("/indexes/{INDEX}"), None)
            .await
            .unwrap();
        projection.client.task(response).await.unwrap();
        let writer = db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET deleted_at = '2026-01-03' WHERE id = ?",
                params![id],
            )
            .unwrap();
        drop(writer);
        wait_degraded(projection).await;
        assert!(
            projection
                .search(&["aurora".to_string()], 10)
                .await
                .is_none()
        );
        wait_ready(projection).await;
        wait_hit(projection, "aurora", &[]).await;
        let writer = db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET deleted_at = NULL WHERE id = ?",
                params![id],
            )
            .unwrap();
        drop(writer);
        wait_hit(projection, "aurora", &[id]).await;
    }

    async fn check_rebuild_after_query_failure(projection: &SearchProjection, id: &str) {
        let response = projection
            .client
            .request(reqwest::Method::DELETE, &format!("/indexes/{INDEX}"), None)
            .await
            .unwrap();
        projection.client.task(response).await.unwrap();
        assert!(
            projection
                .search(&["aurora".to_string()], 10)
                .await
                .is_none()
        );
        wait_degraded(projection).await;
        wait_ready(projection).await;
        wait_hit(projection, "aurora", &[id]).await;
    }

    #[test]
    fn port_contract() {
        assert_eq!(port_from_env(None).unwrap(), 7702);
        assert_eq!(port_from_env(Some("44123")).unwrap(), 44123);
        assert!(port_from_env(Some("0")).is_err());
        assert!(port_from_env(Some("oops")).is_err());
    }

    #[test]
    fn status_count_is_unavailable_until_ready_and_after_degradation() {
        let projection = SearchProjection::new(1, "test-key".to_string());
        projection.documents.store(3, Ordering::Release);
        assert_eq!(projection.ready_document_count(), None);
        projection.set_state(SearchState::Ready);
        assert_eq!(projection.ready_document_count(), Some(3));
        projection.set_state(SearchState::Degraded);
        assert_eq!(projection.ready_document_count(), None);
    }

    #[test]
    fn binary_discovery_searches_path() {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("meilisearch");
        fs::write(&binary, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(
            discover_binary(Some(directory.path().as_os_str())),
            Some(binary)
        );
    }

    #[tokio::test]
    async fn canonical_watcher_indexes_and_removes_notes() {
        let Some(binary) = discover_binary(std::env::var_os("PATH").as_deref()) else {
            return;
        };
        let log_cursor = search_log_cursor();
        let (directory, db) = test_powersync_db().await;
        seed_search_note(&db).await;
        let port_guard = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = port_guard.local_addr().unwrap().port();
        drop(port_guard);
        let data_dir = directory.path().join("meilisearch");
        let key = private_key(&data_dir).unwrap();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (projection, worker) = start_with_binary(
            db.clone(),
            "user-1".to_string(),
            data_dir,
            shutdown_rx,
            port,
            binary,
            key,
        );
        wait_ready(&projection).await;
        assert_eq!(projection.document_count(), 1);

        let id = uuid::Uuid::new_v4().to_string();
        let writer = db.writer().await.unwrap();
        writer.execute(
            "INSERT INTO notes (id, short_id, user_id, type, status, title, summary, content, created_at, updated_at) VALUES (?, 41, 'user-1', 'normal', 'draft', 'Needle title', 'Summary', 'Body', '2026-01-01', '2026-01-01')",
            params![id],
        ).unwrap();
        drop(writer);
        wait_hit(&projection, "Needle", &[&id]).await;
        wait_count(&projection, 2).await;

        let writer = db.writer().await.unwrap();
        writer.execute("UPDATE notes SET title = 'Other', summary = 'Another', content = '中文 English aurora', project_id = 'project-1' WHERE id = ?", params![id]).unwrap();
        drop(writer);
        wait_hit(&projection, "Needle", &[]).await;
        wait_hit(&projection, "aurora", &[&id]).await;
        wait_hit(&projection, "中文", &[&id]).await;
        assert_eq!(projection.document_count(), 2);
        check_find_backend_logs(&projection, &db).await;
        let document = projected_document(&projection, &id).await;
        assert_eq!(document["content"], "中文 English aurora");
        assert_eq!(document["project_id"], "project-1");
        assert_eq!(document["draft"], true);
        assert!(document.get("deleted").is_none());

        let writer = db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET status = 'ai_queued' WHERE id = ?",
                params![id],
            )
            .unwrap();
        drop(writer);
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let value = projected_document(&projection, &id).await;
                if value["draft"] == false {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();

        let writer = db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET deleted_at = '2026-01-02' WHERE id = ?",
                params![id],
            )
            .unwrap();
        drop(writer);
        wait_hit(&projection, "aurora", &[]).await;
        wait_count(&projection, 1).await;

        let writer = db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET deleted_at = NULL WHERE id = ?",
                params![id],
            )
            .unwrap();
        drop(writer);
        wait_hit(&projection, "aurora", &[&id]).await;
        wait_count(&projection, 2).await;

        check_ranking_and_limit(&projection, &db).await;
        wait_count(&projection, 5).await;

        check_rebuild_after_update_failure(&projection, &db, &id).await;
        check_rebuild_after_query_failure(&projection, &id).await;

        assert_projection_logs(log_cursor);

        shutdown_tx.send(true).unwrap();
        worker.await.unwrap();
        assert!(!projection.client.healthy().await);
    }

    #[tokio::test]
    async fn unexpected_child_exit_degrades_without_touching_canonical_db() {
        let (directory, db) = test_powersync_db().await;
        let binary = directory.path().join("meilisearch");
        fs::write(&binary, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let data_dir = directory.path().join("meilisearch-state");
        let key = private_key(&data_dir).unwrap();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (projection, worker) = start_with_binary(
            db.clone(),
            "user-1".to_string(),
            data_dir,
            shutdown_rx,
            port,
            binary,
            key,
        );

        wait_degraded(&projection).await;
        assert_eq!(projection.state(), SearchState::Degraded);
        assert!(
            projection
                .search(&["anything".to_string()], 5)
                .await
                .is_none()
        );
        assert!(db.reader().await.is_ok());
        shutdown_tx.send(true).unwrap();
        worker.await.unwrap();
    }
}
