//! One bounded canonical projection; snapshots run only on the host runtime.
use chrono::{DateTime, Datelike, Days, NaiveDate, NaiveTime, TimeZone, Utc};
use flicknote_core::backend::FailedStage;
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
    pub archived: bool,
    pub draft: bool,
    pub shared: bool,
    pub failed_stage: Option<FailedStage>,
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
    Period::Day(None).range(now).map(|r| r.expect("day range"))
}

/// None follows the current local calendar; a date anchors a historical period.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Period {
    #[default]
    All,
    Day(Option<NaiveDate>),
    Week(Option<NaiveDate>),
}
pub type Range = (DateTime<Utc>, DateTime<Utc>);
impl Period {
    pub fn date<T: TimeZone>(&self, now: &DateTime<T>) -> NaiveDate {
        match self {
            Self::Day(Some(day)) | Self::Week(Some(day)) => *day,
            Self::Day(None) if now.time() < NaiveTime::from_hms_opt(4, 0, 0).unwrap() => {
                now.date_naive().pred_opt().expect("calendar day")
            }
            Self::Week(None) => now
                .date_naive()
                .checked_sub_days(Days::new(u64::from(now.weekday().num_days_from_monday())))
                .expect("calendar week"),
            _ => now.date_naive(),
        }
    }
    pub fn range<T: TimeZone>(&self, now: &DateTime<T>) -> Result<Option<Range>, String> {
        if *self == Self::All {
            return Ok(None);
        }
        let day = self.date(now);
        let (hour, days) = if matches!(self, Self::Day(_)) {
            (4, 1)
        } else {
            (0, 7)
        };
        let next = day
            .checked_add_days(Days::new(days))
            .ok_or("Calendar overflow")?;
        let time = NaiveTime::from_hms_opt(hour, 0, 0).expect("valid boundary");
        let local = |date: NaiveDate| {
            now.timezone()
                .from_local_datetime(&date.and_time(time))
                .earliest()
                .map(|d| d.with_timezone(&Utc))
                .ok_or_else(|| "Boundary does not exist in local calendar".to_string())
        };
        Ok(Some((local(day)?, local(next)?)))
    }
    pub fn follows_clock(&self) -> bool {
        matches!(self, Self::Day(None) | Self::Week(None))
    }
    pub fn shifted<T: TimeZone>(&self, next: bool, now: &DateTime<T>) -> Option<Self> {
        let current = match self {
            Self::Day(_) => Self::Day(None),
            Self::Week(_) => Self::Week(None),
            Self::All => return None,
        };
        let days = Days::new(if matches!(self, Self::Day(_)) { 1 } else { 7 });
        let day = if next {
            self.date(now).checked_add_days(days)?
        } else {
            self.date(now).checked_sub_days(days)?
        };
        let today = current.date(now);
        if day > today {
            return None;
        }
        if day == today {
            return Some(current);
        }
        Some(if matches!(self, Self::Day(_)) {
            Self::Day(Some(day))
        } else {
            Self::Week(Some(day))
        })
    }
}

/// Window-local workspace identity; project names and rail indices are presentation only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Destination {
    #[default]
    Home,
    Project(String),
    Failed,
    Shared,
    Archive,
    Charts,
}

impl Destination {
    /// Utility collections include every creation channel, including during global search.
    pub fn effective_human_only(&self, saved: bool) -> bool {
        saved && matches!(self, Self::Home | Self::Project(_) | Self::Charts)
    }
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
    pub range: Option<Range>,
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
        Self::start_destination(db, user_id, Destination::Home, false)
    }
    /// Creation-channel predicate belongs to the bounded SQL projection, never direct access.
    pub fn start_destination(
        db: PowerSyncDatabase,
        user_id: String,
        destination: Destination,
        human_only: bool,
    ) -> Self {
        let period = if destination == Destination::Home {
            Period::Day(None)
        } else {
            Period::All
        };
        Self::start_period(db, user_id, destination, human_only, period)
    }
    pub fn start_period(
        db: PowerSyncDatabase,
        user_id: String,
        destination: Destination,
        human_only: bool,
        period: Period,
    ) -> Self {
        Self::start_clock(
            db,
            user_id,
            destination,
            human_only,
            period,
            chrono::Local::now,
        )
    }
    /// Private clock seam lets owned-host tests cross real watch timers without wall-clock waits.
    pub(crate) fn start_clock(
        db: PowerSyncDatabase,
        user_id: String,
        destination: Destination,
        human_only: bool,
        period: Period,
        clock: impl Fn() -> DateTime<chrono::Local> + Send + 'static,
    ) -> Self {
        let (sender, receiver) = watch::channel(None);
        let task = tokio::spawn(async move {
            let started = Instant::now();
            let mut emission = 0;
            loop {
                let now = clock();
                let range = match period.range(&now) {
                    Ok(b) => b,
                    Err(error) => {
                        sender.send_replace(Some(Err(error)));
                        return;
                    }
                };
                let membership = match &destination {
                    Destination::Charts => "0",
                    Destination::Home => "?4 = ''",
                    Destination::Project(_) => "n.project_id = ?4",
                    Destination::Failed => "n.status IN ('ai_failed', 'source_failed')",
                    Destination::Shared => "shared = 1",
                    Destination::Archive => "n.deleted_at IS NOT NULL",
                };
                let sql = format!(
                    "WITH today AS (SELECT n.short_id, n.id, coalesce(n.content, '') AS content, coalesce(n.type, 'normal') AS type, p.color, n.title, p.id AS project_id, p.name AS project_name, n.deleted_at IS NOT NULL AS archived, coalesce(n.status, '') = 'draft' AS draft, coalesce(n.status, '') AS status, EXISTS (SELECT 1 FROM note_shares share WHERE share.id = n.id AND share.user_id = n.user_id AND (share.expires_at IS NULL OR julianday(share.expires_at) > julianday('now'))) AS shared FROM notes n LEFT JOIN projects p ON p.id = n.project_id AND p.user_id = n.user_id WHERE n.user_id = ?1 AND (n.deleted_at IS NOT NULL) = CAST(?6 AS INTEGER) AND n.short_id IS NOT NULL AND {membership} AND (?2 = '' OR (julianday(n.created_at) >= julianday(?2) AND julianday(n.created_at) < julianday(?3))) AND (?5 = '0' OR json_type(n.metadata, '$.created_by_ai') IS NOT 'true') ORDER BY n.short_id DESC LIMIT {LIMIT}), context AS (SELECT id, name, color, json_extract(metadata, '$.summary') AS summary FROM projects WHERE user_id = ?1 AND coalesce(is_archived, 0) = 0 ORDER BY name, id LIMIT {LIMIT}) SELECT short_id, id, content, type, color, NULL AS name, title, project_id, project_name, NULL AS summary, archived, draft, shared, status FROM today UNION ALL SELECT NULL, id, NULL, NULL, color, name, NULL, NULL, NULL, summary, NULL, NULL, NULL, NULL FROM context ORDER BY short_id DESC, name, id"
                );
                let project_id = match &destination {
                    Destination::Project(id) => id.clone(),
                    _ => String::new(),
                };
                let params = [
                    user_id.clone(),
                    range.map_or_else(String::new, |r| r.0.to_rfc3339()),
                    range.map_or_else(String::new, |r| r.1.to_rfc3339()),
                    project_id,
                    u8::from(destination.effective_human_only(human_only)).to_string(),
                    if destination == Destination::Archive {
                        "1"
                    } else {
                        "0"
                    }
                    .to_string(),
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
                                archived: r.get(10)?,
                                draft: r.get(11)?,
                                shared: r.get(12)?,
                                failed_stage: FailedStage::from_status(&r.get::<_, String>(13)?),
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
                let until_boundary = range.map_or(std::time::Duration::ZERO, |r| {
                    (r.1 - clock().with_timezone(&Utc))
                        .to_std()
                        .unwrap_or_default()
                });
                let boundary = tokio::time::sleep(until_boundary);
                tokio::pin!(boundary);
                loop {
                    tokio::select! {
                        _ = sender.closed() => return,
                        _ = &mut boundary, if period.follows_clock() => break,
                        result = stream.next() => {
                            let Some(result) = result else { return; };
                            emission += 1;
                            let snapshot = result.map(|(rows, projects)| Snapshot { rows: Arc::new(rows), projects: Arc::new(projects), emission, elapsed_ms: started.elapsed().as_secs_f64() * 1000.0, range }).map_err(|e| e.to_string());
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
