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

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub rows: Arc<Vec<TodayRow>>,
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
    pub fn start(db: PowerSyncDatabase) -> Self {
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
                let sql = format!(
                    "SELECT short_id, id, coalesce(content, '') FROM notes WHERE user_id = ? AND deleted_at IS NULL AND short_id IS NOT NULL AND julianday(created_at) >= julianday(?) AND julianday(created_at) < julianday(?) ORDER BY short_id DESC LIMIT {LIMIT}"
                );
                let params = [
                    super::fixture::USER.to_string(),
                    start.to_rfc3339(),
                    end.to_rfc3339(),
                ];
                let stream = db.watch_statement(sql, params, |stmt, params| {
                    let rows = stmt
                        .query_map(rusqlite::params_from_iter(params), |r| {
                            let content: String = r.get(2)?;
                            let preview = content.split_whitespace().collect::<Vec<_>>().join(" ");
                            Ok(TodayRow {
                                id: r.get(0)?,
                                uuid: r.get(1)?,
                                preview,
                                content,
                            })
                        })?
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                });
                futures_lite::pin!(stream);
                let until_boundary = (end - Utc::now()).to_std().unwrap_or_default();
                let boundary = tokio::time::sleep(until_boundary);
                tokio::pin!(boundary);
                loop {
                    tokio::select! {
                        _ = sender.closed() => return,
                        _ = &mut boundary => break,
                        result = stream.next() => {
                            let Some(result) = result else { return; };
                            emission += 1;
                            let snapshot = result.map(|rows| Snapshot { rows: Arc::new(rows), emission, elapsed_ms: started.elapsed().as_secs_f64() * 1000.0 }).map_err(|e| e.to_string());
                            log::info!("spike today emission={emission} elapsed_ms={:.3}", started.elapsed().as_secs_f64()*1000.0);
                            sender.send_replace(Some(snapshot));
                        }
                    }
                }
            }
        });
        Self { receiver, task }
    }
}
