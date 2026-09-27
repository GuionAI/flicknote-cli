//! Disposable daemon-owned Tantivy projection of the canonical PowerSync note snapshot.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use flicknote_tantivy::{Fields, Note};
use futures_lite::StreamExt;
use powersync::PowerSyncDatabase;
use rusqlite::params;
use tantivy::{Index, IndexReader, IndexWriter, Term};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum State {
    Degraded,
    Rebuilding,
    Ready,
}

#[derive(Clone)]
pub(crate) struct Projection {
    state: Arc<AtomicU8>,
    count: Arc<AtomicUsize>,
    reader: Arc<RwLock<Option<(IndexReader, Fields)>>>,
}

impl Projection {
    fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(State::Rebuilding as u8)),
            count: Arc::new(AtomicUsize::new(0)),
            reader: Arc::new(RwLock::new(None)),
        }
    }
    #[cfg(test)]
    fn state(&self) -> State {
        match self.state.load(Ordering::Acquire) {
            2 => State::Ready,
            1 => State::Rebuilding,
            _ => State::Degraded,
        }
    }
    #[cfg(test)]
    fn count(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }
    fn set_state(&self, value: State) {
        self.state.store(value as u8, Ordering::Release);
    }
    #[cfg(test)]
    fn search(
        &self,
        terms: &[String],
        project: Option<&str>,
    ) -> Result<Vec<flicknote_tantivy::Hit>, String> {
        let guard = self.reader.read().map_err(|e| e.to_string())?;
        let (reader, fields) = guard.as_ref().ok_or("projection reader unavailable")?;
        flicknote_tantivy::search(reader, *fields, terms, project, 10).map_err(|e| e.to_string())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Row {
    uuid: String,
    short_id: Option<i64>,
    project: String,
    title: String,
    summary: String,
    content: String,
    updated_at: String,
    updated_sort: i64,
}

impl Row {
    fn note(&self) -> Note<'_> {
        Note {
            uuid: &self.uuid,
            short_id: self.short_id.and_then(|id| u64::try_from(id).ok()),
            project: &self.project,
            updated_at: &self.updated_at,
            updated_sort: self.updated_sort,
            text: [&self.title, &self.summary, &self.content],
        }
    }
    fn content_bytes(&self) -> usize {
        self.title.len() + self.summary.len() + self.content.len()
    }
}

type Snapshot = HashMap<String, Row>;
const SNAPSHOT_SQL: &str = "SELECT notes.id, notes.short_id, COALESCE(projects.name, ''), COALESCE(notes.title, ''), COALESCE(notes.summary, ''), COALESCE(notes.content, ''), COALESCE(notes.updated_at, ''), COALESCE(CAST(unixepoch(notes.updated_at, 'subsec') * 1000 AS INTEGER), 0) FROM notes LEFT JOIN projects ON projects.id = notes.project_id WHERE notes.user_id = ? AND notes.deleted_at IS NULL";

#[allow(clippy::needless_pass_by_value)]
fn snapshot(
    stmt: &mut rusqlite::Statement<'_>,
    parameters: [String; 1],
) -> Result<Snapshot, powersync::error::PowerSyncError> {
    let rows = stmt.query_map(params![parameters[0]], |row| {
        Ok(Row {
            uuid: row.get(0)?,
            short_id: row.get(1)?,
            project: row.get(2)?,
            title: row.get(3)?,
            summary: row.get(4)?,
            content: row.get(5)?,
            updated_at: row.get(6)?,
            updated_sort: row.get(7)?,
        })
    })?;
    let mut result = HashMap::new();
    for row in rows {
        let row = row?;
        result.insert(row.uuid.clone(), row);
    }
    Ok(result)
}

struct LiveIndex {
    _index: Index,
    writer: IndexWriter,
    reader: IndexReader,
    fields: Fields,
    path: PathBuf,
}

impl LiveIndex {
    fn rebuild(path: &Path, current: &Snapshot) -> Result<Self, String> {
        // An incomplete or corrupt prior index is never trusted. It is derived state.
        if path.exists() {
            fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        }
        fs::create_dir_all(path).map_err(|e| e.to_string())?;
        let (index, fields) = flicknote_tantivy::open_new(path).map_err(|e| e.to_string())?;
        let mut writer = index.writer(50_000_000).map_err(|e| e.to_string())?;
        for row in current.values() {
            writer
                .add_document(flicknote_tantivy::document(fields, &row.note()))
                .map_err(|e| e.to_string())?;
        }
        writer.commit().map_err(|e| e.to_string())?;
        let reader = index.reader().map_err(|e| e.to_string())?;
        reader.reload().map_err(|e| e.to_string())?;
        if reader.searcher().num_docs() != current.len() as u64 {
            return Err("Tantivy document count differs from canonical snapshot".into());
        }
        Ok(Self {
            _index: index,
            writer,
            reader,
            fields,
            path: path.to_path_buf(),
        })
    }
    fn apply(
        &mut self,
        previous: &Snapshot,
        current: &Snapshot,
    ) -> Result<(usize, usize, Duration), String> {
        let started = Instant::now();
        let mut upserts = 0;
        let mut removals = 0;
        for (uuid, old) in previous {
            if !current.contains_key(uuid) {
                self.writer
                    .delete_term(Term::from_field_text(self.fields.uuid, uuid));
                removals += 1;
            } else if current.get(uuid) != Some(old) {
                self.writer
                    .delete_term(Term::from_field_text(self.fields.uuid, uuid));
            }
        }
        for (uuid, row) in current {
            if previous.get(uuid) != Some(row) {
                self.writer
                    .add_document(flicknote_tantivy::document(self.fields, &row.note()))
                    .map_err(|e| e.to_string())?;
                upserts += 1;
            }
        }
        if upserts != 0 || removals != 0 {
            self.writer.commit().map_err(|e| e.to_string())?;
            self.reader.reload().map_err(|e| e.to_string())?;
            if self.reader.searcher().num_docs() != current.len() as u64 {
                return Err("Tantivy document count differs from canonical snapshot".into());
            }
        }
        Ok((upserts, removals, started.elapsed()))
    }
}

fn directory_bytes(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => directory_bytes(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

fn resident_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
        line.split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()?
            .checked_mul(1024)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn sample_peak_until(
    done: Arc<std::sync::atomic::AtomicBool>,
    baseline: u64,
) -> std::thread::JoinHandle<u64> {
    std::thread::spawn(move || {
        let mut peak = baseline;
        while !done.load(Ordering::Acquire) {
            peak = peak.max(resident_bytes().unwrap_or(0));
            std::thread::sleep(Duration::from_millis(10));
        }
        peak.max(resident_bytes().unwrap_or(0))
    })
}

pub(crate) fn start(
    db: PowerSyncDatabase,
    user_id: String,
    path: PathBuf,
    mut shutdown: watch::Receiver<bool>,
) -> (Projection, tokio::task::JoinHandle<()>) {
    let projection = Projection::new();
    let shared = projection.clone();
    let task = tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        'rebuild: loop {
            let stream = db.watch_statement(SNAPSHOT_SQL.to_owned(), [user_id.clone()], snapshot);
            tokio::pin!(stream);
            let mut previous = Snapshot::new();
            let mut live: Option<LiveIndex> = None;
            loop {
                tokio::select! {
                    result = stream.next() => {
                        let current = match result {
                            Some(Ok(current)) => current,
                            Some(Err(error)) => {
                                log::warn!("tantivy_projection watcher failed; retrying full rebuild: {error}");
                                shared.set_state(State::Degraded);
                                if let Ok(mut guard) = shared.reader.write() { *guard = None; }
                                tokio::time::sleep(backoff).await;
                                backoff = (backoff * 2).min(Duration::from_secs(30));
                                continue 'rebuild;
                            }
                            None => {
                                log::warn!("tantivy_projection watcher stopped; retrying full rebuild");
                                shared.set_state(State::Degraded);
                                if let Ok(mut guard) = shared.reader.write() { *guard = None; }
                                tokio::time::sleep(backoff).await;
                                backoff = (backoff * 2).min(Duration::from_secs(30));
                                continue 'rebuild;
                            }
                        };
                        let initial = live.is_none();
                        if initial { shared.set_state(State::Rebuilding); }
                        let baseline = resident_bytes();
                        let started = Instant::now();
                        let peak_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let peak_task = initial.then(|| sample_peak_until(peak_done.clone(), baseline.unwrap_or(0)));
                        let path = path.clone();
                        let old = std::mem::take(&mut live);
                        let previous_snapshot = std::mem::take(&mut previous);
                        let result = tokio::task::spawn_blocking(move || {
                            if let Some(mut index) = old {
                                let changes = index.apply(&previous_snapshot, &current)?;
                                Ok::<_, String>((index, current, Some(changes)))
                            } else {
                                let index = LiveIndex::rebuild(&path, &current)?;
                                Ok((index, current, None))
                            }
                        }).await;
                        peak_done.store(true, Ordering::Release);
                        let peak = peak_task.and_then(|task| task.join().ok());
                        match result {
                            Ok(Ok((index, current, changes))) => {
                                let count = current.len();
                                let bytes: usize = current.values().map(Row::content_bytes).sum();
                                let disk = directory_bytes(&index.path);
                                if let Ok(mut guard) = shared.reader.write() { *guard = Some((index.reader.clone(), index.fields)); }
                                shared.count.store(count, Ordering::Release);
                                shared.set_state(State::Ready);
                                if let Some((upserts, removals, elapsed)) = changes {
                                    if upserts != 0 || removals != 0 {
                                        log::info!("tantivy_projection upserts={upserts} removals={removals} notes={count} commit_reload_ms={:.1}", elapsed.as_secs_f64() * 1000.0);
                                    }
                                } else {
                                    log::info!("tantivy_projection ready notes={count} content_mib={:.2} index_mib={:.2} rebuild_ms={:.1} rss_baseline_mib={:.2} rss_peak_mib={:.2} rss_ready_writer_retained_mib={:.2} rss_delta_mib={:.2}", bytes as f64 / 1_048_576.0, disk as f64 / 1_048_576.0, started.elapsed().as_secs_f64() * 1000.0, baseline.unwrap_or(0) as f64 / 1_048_576.0, peak.unwrap_or(0) as f64 / 1_048_576.0, resident_bytes().unwrap_or(0) as f64 / 1_048_576.0, resident_bytes().unwrap_or(0).saturating_sub(baseline.unwrap_or(0)) as f64 / 1_048_576.0);
                                }
                                live = Some(index);
                                previous = current;
                                backoff = Duration::from_secs(1);
                            }
                            Ok(Err(error)) => {
                                log::warn!("tantivy_projection degraded; full rebuild required: {error}");
                                shared.set_state(State::Degraded);
                                if let Ok(mut guard) = shared.reader.write() { *guard = None; }
                                tokio::time::sleep(backoff).await;
                                backoff = (backoff * 2).min(Duration::from_secs(30));
                                // Re-open the watcher to get a fresh canonical snapshot.
                                continue 'rebuild;
                            }
                            Err(error) => {
                                log::warn!("tantivy_projection worker failed; full rebuild required: {error}");
                                shared.set_state(State::Degraded);
                                if let Ok(mut guard) = shared.reader.write() { *guard = None; }
                                tokio::time::sleep(backoff).await;
                                backoff = (backoff * 2).min(Duration::from_secs(30));
                                // Re-open the watcher to get a fresh canonical snapshot.
                                continue 'rebuild;
                            }
                        }
                    }
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { break 'rebuild; }
                    }
                }
            }
        }
        shared.set_state(State::Degraded);
    });
    (projection, task)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(uuid: &str, id: i64, content: &str) -> Row {
        Row {
            uuid: uuid.to_owned(),
            short_id: Some(id),
            project: "Alpha".into(),
            title: "Title".into(),
            summary: String::new(),
            content: content.into(),
            updated_at: "2026-09-27T00:00:00Z".into(),
            updated_sort: 1_790_467_200_000,
        }
    }

    fn hits(index: &LiveIndex, term: &str, project: Option<&str>) -> Vec<String> {
        flicknote_tantivy::search(&index.reader, index.fields, &[term.to_owned()], project, 10)
            .unwrap()
            .into_iter()
            .map(|hit| hit.uuid)
            .collect()
    }

    #[test]
    fn rebuild_and_apply_follow_canonical_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index");
        let mut empty = Snapshot::new();
        let mut index = LiveIndex::rebuild(&path, &empty).unwrap();
        assert_eq!(index.reader.searcher().num_docs(), 0);
        let projection = Projection::new();
        *projection.reader.write().unwrap() = Some((index.reader.clone(), index.fields));

        let first = row("uuid-1", 1, "searchable alpha");
        let mut current = Snapshot::from([(first.uuid.clone(), first)]);
        index.apply(&empty, &current).unwrap();
        assert_eq!(hits(&index, "alpha", None), ["uuid-1"]);
        assert_eq!(
            projection.search(&["alpha".into()], None).unwrap()[0].uuid,
            "uuid-1"
        );
        assert!(hits(&index, "alpha", Some("Other")).is_empty());
        assert_eq!(hits(&index, "alpha", Some("Alpha")), ["uuid-1"]);

        empty = current.clone();
        current.get_mut("uuid-1").unwrap().content = "searchable beta".into();
        index.apply(&empty, &current).unwrap();
        assert!(hits(&index, "alpha", None).is_empty());
        assert_eq!(hits(&index, "beta", None), ["uuid-1"]);
        assert_eq!(index.reader.searcher().num_docs(), 1);

        empty = current.clone();
        current.clear();
        index.apply(&empty, &current).unwrap();
        assert!(hits(&index, "beta", None).is_empty());
        assert_eq!(index.reader.searcher().num_docs(), 0);

        empty = current.clone();
        current.insert("uuid-1".into(), row("uuid-1", 1, "searchable gamma"));
        index.apply(&empty, &current).unwrap();
        assert_eq!(hits(&index, "gamma", None), ["uuid-1"]);
        drop(projection);
        drop(index);
        let reopened = LiveIndex::rebuild(&path, &current).unwrap();
        assert_eq!(hits(&reopened, "gamma", None), ["uuid-1"]);
    }

    #[test]
    fn incomplete_or_corrupt_previous_index_is_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("meta.json"), b"broken").unwrap();
        let current = Snapshot::from([("uuid-1".into(), row("uuid-1", 1, "recovered content"))]);
        let index = LiveIndex::rebuild(&path, &current).unwrap();
        assert_eq!(hits(&index, "recovered", None), ["uuid-1"]);
    }

    #[test]
    fn rapid_updates_commit_only_final_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index");
        let mut current = Snapshot::new();
        let mut index = LiveIndex::rebuild(&path, &current).unwrap();
        for i in 0..20 {
            let previous = current.clone();
            current.insert(
                "uuid-1".into(),
                row("uuid-1", 1, &format!("content version{i}")),
            );
            index.apply(&previous, &current).unwrap();
        }
        assert_eq!(hits(&index, "version19", None), ["uuid-1"]);
        assert!(hits(&index, "version18", None).is_empty());
        assert_eq!(index.reader.searcher().num_docs(), 1);
    }

    #[test]
    fn projection_starts_rebuilding_with_no_documents() {
        let projection = Projection::new();
        assert_eq!(projection.state(), State::Rebuilding);
        assert_eq!(projection.count(), 0);
    }
}

#[cfg(test)]
mod watcher_tests {
    use super::*;
    use crate::test_support::test_powersync_db;

    async fn wait_for(projection: &Projection, count: usize, term: &str, hits: usize) {
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                if projection.state() == State::Ready
                    && projection.count() == count
                    && let Ok(found) = projection.search(&[term.to_owned()], None)
                    && found.len() == hits
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn canonical_watcher_rebuilds_and_tracks_live_changes() {
        let (dir, db) = test_powersync_db().await;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (projection, worker) = start(
            db.clone(),
            "user-1".into(),
            dir.path().join("tantivy"),
            shutdown_rx,
        );
        wait_for(&projection, 0, "needle", 0).await;
        let writer = db.writer().await.unwrap();
        writer.execute("INSERT INTO projects (id, user_id, name, is_archived) VALUES ('project-1', 'user-1', 'Alpha', 0)", []).unwrap();
        writer.execute("INSERT INTO notes (id, short_id, user_id, type, status, title, content, project_id, updated_at) VALUES ('note-1', 1, 'user-1', 'normal', 'draft', 'Needle', 'First body', 'project-1', '2026-09-27T00:00:00Z')", []).unwrap();
        drop(writer);
        wait_for(&projection, 1, "needle", 1).await;
        assert_eq!(
            projection
                .search(&["needle".into()], Some("Alpha"))
                .unwrap()
                .len(),
            1
        );
        assert!(
            projection
                .search(&["needle".into()], Some("Other"))
                .unwrap()
                .is_empty()
        );

        let writer = db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET title = 'Changed', content = 'second body' WHERE id = 'note-1'",
                [],
            )
            .unwrap();
        drop(writer);
        wait_for(&projection, 1, "changed", 1).await;
        assert!(
            projection
                .search(&["needle".into()], None)
                .unwrap()
                .is_empty()
        );

        let writer = db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET deleted_at = '2026-09-27T01:00:00Z' WHERE id = 'note-1'",
                [],
            )
            .unwrap();
        drop(writer);
        wait_for(&projection, 0, "changed", 0).await;

        let writer = db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET deleted_at = NULL, title = 'Restored' WHERE id = 'note-1'",
                [],
            )
            .unwrap();
        drop(writer);
        wait_for(&projection, 1, "restored", 1).await;
        assert!(shutdown_tx.send(true).is_ok());
        tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap();
    }
}
