//! Owner-active creation counts, independent of note-list bounds and lifecycle status.
use crate::today::{Period, ProjectContext, Range};
use chrono::{DateTime, Days, NaiveDate, TimeZone, Utc};
use futures_lite::StreamExt;
use powersync::PowerSyncDatabase;
use std::{collections::BTreeMap, sync::Arc};
use tokio::{sync::watch, task::JoinHandle};

#[derive(Debug, Clone)]
pub struct Day {
    pub date: NaiveDate,
    pub range: Range,
    pub counts: BTreeMap<String, u64>,
}
#[derive(Debug, Clone)]
pub struct Group {
    pub key: String,
    pub name: String,
    pub color: String,
    pub total: u64,
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub days: Vec<Day>,
    pub groups: Vec<Group>,
    pub projects: Arc<Vec<ProjectContext>>,
}
impl Snapshot {
    pub fn range(&self) -> Range {
        (self.days[0].range.0, self.days[29].range.1)
    }
    pub fn total(&self) -> u64 {
        self.groups.iter().map(|g| g.total).sum()
    }
}

/// Each day resolves local 04:00 independently, including calendar/DST transitions.
pub fn days<T: TimeZone>(now: &DateTime<T>) -> Result<Vec<Day>, String> {
    let today = Period::Day(None).date(now);
    (0..30)
        .map(|i| {
            let date = today
                .checked_sub_days(Days::new(29 - i))
                .ok_or("Calendar overflow")?;
            let range = Period::Day(Some(date)).range(now)?.expect("day range");
            Ok(Day {
                date,
                range,
                counts: BTreeMap::new(),
            })
        })
        .collect()
}

const COLORS: [u32; 12] = [
    0x4187D9, 0xE47A36, 0x59A86C, 0xB56BC5, 0xD45A6B, 0x3BA8AA, 0xB99436, 0x7584D9, 0xC46D98,
    0x5C9A4A, 0xBF7548, 0x597FA8,
];
fn color(key: &str, configured: Option<String>) -> String {
    if let Some(hex) = configured.map(|c| c.strip_prefix('#').unwrap_or(&c).to_owned())
        && hex.len() == 6
        && hex.chars().all(|c| c.is_ascii_hexdigit())
    {
        return format!("#{hex}").to_uppercase();
    }
    let hash = key.bytes().fold(14_695_981_039_346_656_037_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(1_099_511_628_211)
    });
    format!("#{:06X}", COLORS[hash as usize % COLORS.len()])
}

// SQL aggregates timestamps/project UUIDs without reading content/title/summary or imposing a
// list bound. The one-second envelope avoids SQLite's millisecond date rounding dropping an
// exact boundary candidate. Rust parses and applies the exact half-open semantic-day bounds.
const SQL: &str = "SELECT n.created_at, n.project_id, p.name, p.color, count(*) FROM notes n LEFT JOIN projects p ON p.id=n.project_id AND p.user_id=n.user_id WHERE n.user_id=?1 AND n.deleted_at IS NULL AND (?2='0' OR json_type(n.metadata,'$.created_by_ai') IS NOT 'true') AND julianday(n.created_at) >= julianday(?3)-1.0/86400 AND julianday(n.created_at) < julianday(?4)+1.0/86400 GROUP BY n.created_at,n.project_id,p.name,p.color";
fn aggregate(
    stmt: &mut rusqlite::Statement<'_>,
    params: &[String],
    mut days: Vec<Day>,
) -> rusqlite::Result<(Vec<Day>, Vec<Group>)> {
    let mut groups = BTreeMap::<String, Group>::new();
    let mut rows = stmt.query(rusqlite::params_from_iter(params))?;
    while let Some(row) = rows.next()? {
        let created: String = row.get(0)?;
        let Ok(created) = DateTime::parse_from_rfc3339(&created) else {
            continue;
        };
        let created = created.with_timezone(&Utc);
        let Some(day) = days
            .iter_mut()
            .find(|d| created >= d.range.0 && created < d.range.1)
        else {
            continue;
        };
        let project: Option<String> = row.get(1)?;
        let name: Option<String> = row.get(2)?;
        let configured = row.get(3)?;
        let key = project.map_or_else(|| "unassigned".to_owned(), |id| format!("project:{id}"));
        let count = row.get::<_, i64>(4)? as u64;
        *day.counts.entry(key.clone()).or_default() += count;
        let group = groups.entry(key.clone()).or_insert_with(|| Group {
            name: name.unwrap_or_else(|| {
                if key == "unassigned" {
                    "Unassigned"
                } else {
                    "Unknown project"
                }
                .into()
            }),
            color: if key == "unassigned" {
                "#87929B".into()
            } else {
                color(&key, configured)
            },
            key,
            total: 0,
        });
        group.total += count;
    }
    Ok((days, groups.into_values().collect()))
}

pub struct ChartWatch {
    pub receiver: watch::Receiver<Option<Result<Snapshot, String>>>,
    task: JoinHandle<()>,
}
impl Drop for ChartWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl ChartWatch {
    pub fn start(db: PowerSyncDatabase, user: String, human: bool) -> Self {
        Self::start_clock(db, user, human, chrono::Local::now)
    }
    pub(crate) fn start_clock(
        db: PowerSyncDatabase,
        user: String,
        human: bool,
        clock: impl Fn() -> DateTime<chrono::Local> + Send + 'static,
    ) -> Self {
        let (send, receiver) = watch::channel(None);
        let task = tokio::spawn(async move {
            loop {
                let days = match days(&clock()) {
                    Ok(days) => days,
                    Err(error) => {
                        send.send_replace(Some(Err(error)));
                        return;
                    }
                };
                let end = days[29].range.1;
                let params = [
                    user.clone(),
                    if human { "1" } else { "0" }.into(),
                    days[0].range.0.to_rfc3339(),
                    end.to_rfc3339(),
                ];
                let stream = db.watch_statement(SQL.into(), params, move |stmt, params| {
                    aggregate(stmt, &params, days.clone()).map_err(Into::into)
                });
                futures_lite::pin!(stream);
                let boundary = tokio::time::sleep(
                    (end - clock().with_timezone(&Utc))
                        .to_std()
                        .unwrap_or_default(),
                );
                tokio::pin!(boundary);
                loop {
                    tokio::select! {
                        _ = send.closed() => return,
                        _ = &mut boundary => break,
                        value = stream.next() => {
                            let Some(value) = value else { return; };
                            let result = match value {
                                Ok((days,groups)) => {
                                    // Rail context is independent of the chart source/range, as on other destinations.
                                    match db.reader().await {
                                        Ok(reader) => reader.prepare("SELECT id,name,color,json_extract(metadata,'$.summary') FROM projects WHERE user_id=? AND coalesce(is_archived,0)=0 ORDER BY name,id LIMIT 10000").and_then(|mut stmt| {
                                            let projects = stmt.query_map([&user],|r| Ok(ProjectContext { id:r.get(0)?,name:r.get(1)?,color:r.get(2)?,summary:r.get(3)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
                                            Ok(Snapshot { days,groups,projects:Arc::new(projects) })
                                        }).map_err(|e| e.to_string()),
                                        Err(error) => Err(error.to_string()),
                                    }
                                },
                                Err(error) => Err(error.to_string()),
                            };
                            let failed = result.is_err();
                            send.send_replace(Some(result));
                            if failed { return; }
                        }
                    }
                }
            }
        });
        Self { receiver, task }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    #[test]
    fn thirty_semantic_dates_dst_year_and_exact_boundary() {
        for (zone, date) in [
            (chrono_tz::America::New_York, (2026, 3, 8)),
            (chrono_tz::America::New_York, (2026, 11, 1)),
            (chrono_tz::Asia::Shanghai, (2027, 1, 1)),
        ] {
            let before = zone
                .with_ymd_and_hms(date.0, date.1, date.2, 3, 59, 59)
                .unwrap();
            let after = zone
                .with_ymd_and_hms(date.0, date.1, date.2, 4, 0, 0)
                .unwrap();
            let old = days(&before).unwrap();
            let current = days(&after).unwrap();
            assert_eq!(current.len(), 30);
            assert_eq!(current[29].date, after.date_naive());
            assert_eq!(old[29].date, after.date_naive().pred_opt().unwrap());
            assert_eq!(old[29].range.1, current[29].range.0);
            assert!(current.iter().all(|d| d.counts.is_empty()
                && d.range.0.with_timezone(&zone).hour() == 4
                && d.range.1.with_timezone(&zone).hour() == 4));
            assert!(
                current.windows(2).all(
                    |w| w[0].range.1 == w[1].range.0 && w[0].date.succ_opt() == Some(w[1].date)
                )
            );
            if zone == chrono_tz::America::New_York {
                let length = old[29].range.1 - old[29].range.0;
                assert_eq!(length.num_hours(), if date.1 == 3 { 23 } else { 25 });
            }
        }
    }

    #[test]
    fn configured_and_uuid_fallback_colors_are_stable() {
        assert_eq!(color("project:p", Some("abcdef".into())), "#ABCDEF");
        for value in [None, Some("#bad".into()), Some("zzzzzz".into())] {
            assert_eq!(color("project:p", value), color("project:p", None));
        }
        assert_ne!(color("project:p", None), "#87929B");
    }
}
