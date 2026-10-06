//! One bounded canonical projection; snapshots run only on the host runtime.
use chrono::{DateTime, NaiveTime, TimeZone, Utc};
use futures_lite::StreamExt;
use powersync::PowerSyncDatabase;
use std::{sync::Arc, time::Instant};
use tokio::{sync::watch, task::JoinHandle};

pub const LIMIT: usize = 10_000;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodayRow {
    pub id: i64,
    pub uuid: String,
    pub preview: String,
    pub content: String,
    pub title: Option<String>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub note_type: String,
    pub project_color: Option<String>,
}

/// Match desktop line folding while preserving spaces/tabs inside each line.
pub fn fold_preview(text: &str) -> String {
    text.split([
        '\n', '\r', '\u{000b}', '\u{000c}', '\u{0085}', '\u{2028}', '\u{2029}',
    ])
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

fn persisted_preview(content: &str, title: Option<&str>) -> String {
    fold_preview(if content.len() <= 512 {
        content
    } else {
        title.unwrap_or("Untitled note")
    })
}

pub fn bounds<T: TimeZone>(now: &DateTime<T>) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    let four = NaiveTime::from_hms_opt(4, 0, 0).expect("valid time");
    let day = if now.time() < four {
        now.date_naive().pred_opt().ok_or("Calendar underflow")?
    } else {
        now.date_naive()
    };
    let next = day.succ_opt().ok_or("Calendar overflow")?;
    let start = now
        .timezone()
        .from_local_datetime(&day.and_time(four))
        .earliest()
        .ok_or("04:00 does not exist in local calendar")?;
    let end = now
        .timezone()
        .from_local_datetime(&next.and_time(four))
        .earliest()
        .ok_or("04:00 does not exist in local calendar")?;
    Ok((start.with_timezone(&Utc), end.with_timezone(&Utc)))
}

/// Window-local workspace identity; project names and rail indices are presentation only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Destination {
    #[default]
    Home,
    Project(String),
}

#[derive(Debug, Clone)]
pub struct ProjectContext {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub rows: Arc<Vec<TodayRow>>,
    pub projects: Arc<Vec<ProjectContext>>,
    pub emission: u64,
    pub elapsed_ms: f64,
}

pub struct TodayWatch {
    pub receiver: watch::Receiver<Option<Result<Snapshot, String>>>,
    task: JoinHandle<()>,
}
impl Drop for TodayWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TodayWatch {
    #[cfg(feature = "experimental-spike")]
    pub fn start(db: PowerSyncDatabase) -> Self {
        Self::start_for_user(db, crate::spike::USER.to_string())
    }
    pub fn start_for_user(db: PowerSyncDatabase, user_id: String) -> Self {
        Self::start_destination(db, user_id, Destination::Home)
    }
    pub fn start_destination(
        db: PowerSyncDatabase,
        user_id: String,
        destination: Destination,
    ) -> Self {
        let (sender, receiver) = watch::channel(None);
        let task = tokio::spawn(async move {
            let started = Instant::now();
            let mut emission = 0;
            loop {
                let now = chrono::Local::now();
                let (start, end) = match bounds(&now) {
                    Ok(b) => b,
                    Err(error) => {
                        sender.send_replace(Some(Err(error)));
                        return;
                    }
                };
                let membership = match &destination {
                    Destination::Home => {
                        "?4 = '' AND julianday(n.created_at) >= julianday(?2) AND julianday(n.created_at) < julianday(?3)"
                    }
                    Destination::Project(_) => "n.project_id = ?4",
                };
                let sql = format!(
                    "WITH today AS (SELECT n.short_id, n.id, coalesce(n.content, '') AS content, coalesce(n.type, 'normal') AS type, p.color, n.title, p.id AS project_id, p.name AS project_name FROM notes n LEFT JOIN projects p ON p.id = n.project_id AND p.user_id = n.user_id WHERE n.user_id = ?1 AND n.deleted_at IS NULL AND n.short_id IS NOT NULL AND {membership} ORDER BY n.short_id DESC LIMIT {LIMIT}), context AS (SELECT id, name, color, json_extract(metadata, '$.summary') AS summary FROM projects WHERE user_id = ?1 AND coalesce(is_archived, 0) = 0 ORDER BY name, id LIMIT {LIMIT}) SELECT short_id, id, content, type, color, NULL AS name, title, project_id, project_name, NULL AS summary FROM today UNION ALL SELECT NULL, id, NULL, NULL, color, name, NULL, NULL, NULL, summary FROM context ORDER BY short_id DESC, name, id"
                );
                let project_id = match &destination {
                    Destination::Home => String::new(),
                    Destination::Project(id) => id.clone(),
                };
                let params = [
                    user_id.clone(),
                    start.to_rfc3339(),
                    end.to_rfc3339(),
                    project_id,
                ];
                let stream = db.watch_statement(sql, params, |stmt, params| {
                    let mut rows = Vec::new();
                    let mut projects = Vec::new();
                    let mut results = stmt.query(rusqlite::params_from_iter(params))?;
                    while let Some(r) = results.next()? {
                        if let Some(id) = r.get::<_, Option<i64>>(0)? {
                            let content: String = r.get(2)?;
                            let title: Option<String> = r.get(6)?;
                            let preview = persisted_preview(&content, title.as_deref());
                            rows.push(TodayRow {
                                id,
                                uuid: r.get(1)?,
                                preview,
                                content,
                                title,
                                project_id: r.get(7)?,
                                project_name: r.get(8)?,
                                note_type: r.get(3)?,
                                project_color: r.get(4)?,
                            });
                        } else {
                            projects.push(ProjectContext {
                                id: r.get(1)?,
                                name: r.get(5)?,
                                color: r.get(4)?,
                                summary: r.get(9)?,
                            });
                        }
                    }
                    Ok((rows, projects))
                });
                futures_lite::pin!(stream);
                let until_boundary = (end - Utc::now()).to_std().unwrap_or_default();
                let boundary = tokio::time::sleep(until_boundary);
                tokio::pin!(boundary);
                loop {
                    tokio::select! {
                        _ = sender.closed() => return,
                        _ = &mut boundary, if destination == Destination::Home => break,
                        result = stream.next() => {
                            let Some(result) = result else { return; };
                            emission += 1;
                            let snapshot = result.map(|(rows, projects)| Snapshot { rows: Arc::new(rows), projects: Arc::new(projects), emission, elapsed_ms: started.elapsed().as_secs_f64() * 1000.0 }).map_err(|e| e.to_string());
                            log::info!("today emission={emission} elapsed_ms={:.3}", started.elapsed().as_secs_f64()*1000.0);
                            sender.send_replace(Some(snapshot));
                        }
                    }
                }
            }
        });
        Self { receiver, task }
    }
}
