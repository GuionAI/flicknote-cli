//! GUI-host-only compact Decisions routing. No actor is started by headless hosts.
use crate::{PowerSyncDatabase, app::Application};
use flicknote_client::dto::NoteRouteProjectInput;
use futures_lite::StreamExt;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::watch;

pub const ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";
const BODY_LIMIT: usize = 64 * 1024;
#[derive(Clone, Default)]
pub struct Credential {
    pub key: Option<String>,
    pub generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Project {
    id: String,
    name: String,
    summary: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Note {
    id: i64,
    uuid: String,
    text: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Snapshot {
    projects: Vec<Project>,
    notes: Vec<Note>,
}

fn text(summary: Option<String>, content: &str) -> String {
    if let Some(summary) = summary.filter(|s| !s.trim().is_empty()) {
        return summary.trim().to_owned();
    }
    let content = content.trim();
    let mut end = content.len().min(2048);
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    content[..end].to_owned()
}
fn sql() -> &'static str {
    "SELECT 'p', id, name, json_extract(metadata, '$.summary'), NULL FROM projects WHERE user_id = ?1 AND coalesce(is_archived, 0) = 0 UNION ALL SELECT 'n', id, short_id, summary, coalesce(content, '') FROM notes WHERE user_id = ?1 AND deleted_at IS NULL AND status = 'ready' AND project_id IS NULL AND short_id > 0 AND julianday(created_at) >= julianday(?2) AND coalesce(json_type(metadata, '$.project_routing.routed'), '') != 'true' ORDER BY 1, 2"
}
fn read(stmt: &mut rusqlite::Statement<'_>, params: &[String]) -> rusqlite::Result<Snapshot> {
    let mut snapshot = Snapshot::default();
    let mut rows = stmt.query(rusqlite::params_from_iter(params))?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(0)? == "p" {
            snapshot.projects.push(Project {
                id: row.get(1)?,
                name: row.get(2)?,
                summary: row.get(3)?,
            });
        } else {
            let text = text(row.get(3)?, &row.get::<_, String>(4)?);
            if !text.trim().is_empty() {
                snapshot.notes.push(Note {
                    uuid: row.get(1)?,
                    id: row.get(2)?,
                    text,
                });
            }
        }
    }
    Ok(snapshot)
}
struct Batch {
    snapshot: Snapshot,
    body: Vec<u8>,
    signature: String,
}
impl Batch {
    fn new(projects: &[Project], notes: &[Note]) -> Result<Self, &'static str> {
        let mut state = BTreeMap::new();
        let mut criteria = BTreeMap::new();
        for (i, project) in projects.iter().enumerate() {
            let alias = format!("project_{i}");
            criteria.insert(alias.clone(), format!("Use projects.{alias}"));
            state.insert(
                alias,
                json!({"name":project.name,"summary":project.summary}),
            );
        }
        criteria.insert("none".into(), "No active project is a good fit".into());
        let questions: BTreeMap<_, _> = notes.iter().enumerate().map(|(i,n)| (format!("note_{i}"),json!({
            "type":"choice", "criteria":criteria,
            "instructions":format!("Choose the best active project for this note, or none. Judge independently.\n{}",n.text)
        }))).collect();
        let body = serde_json::to_vec(
            &json!({"model":"typesafe/jev-1.13","state":{"projects":state},"questions":questions}),
        )
        .map_err(|_| "Could not prepare organization request")?;
        if body.len() > BODY_LIMIT {
            return Err(
                "Project summaries or note input are too large. Shorten them to resume organization.",
            );
        }
        // Canonical identity participates only in the local retry/staleness key.
        let signature = format!("{:?}:{:?}", projects, notes);
        Ok(Self {
            snapshot: Snapshot {
                projects: projects.to_vec(),
                notes: notes.to_vec(),
            },
            body,
            signature,
        })
    }
    fn routes(&self, response: &Value) -> Result<Vec<NoteRouteProjectInput>, &'static str> {
        let answers = response
            .get("answers")
            .and_then(Value::as_object)
            .ok_or("Invalid organization response")?;
        if answers.len() != self.snapshot.notes.len() {
            return Err("Invalid organization response");
        }
        self.snapshot
            .notes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let a = answers
                    .get(&format!("note_{i}"))
                    .ok_or("Invalid organization response")?;
                if a.get("type").and_then(Value::as_str) != Some("choice") {
                    return Err("Invalid organization response");
                }
                let choice = a
                    .get("choice")
                    .and_then(Value::as_str)
                    .ok_or("Invalid organization response")?;
                let project_id = if choice == "none" {
                    None
                } else {
                    self.snapshot
                        .projects
                        .iter()
                        .enumerate()
                        .find(|(j, _)| choice == format!("project_{j}"))
                        .map(|(_, p)| Some(p.id.clone()))
                        .ok_or("Invalid organization response")?
                };
                let probability = a
                    .get("probabilities")
                    .and_then(|p| p.get(choice))
                    .and_then(Value::as_f64)
                    .filter(|p| p.is_finite() && (0.0..=1.0).contains(p))
                    .ok_or("Invalid organization response")?;
                Ok(NoteRouteProjectInput {
                    note_id: n.id,
                    project_id,
                    probability,
                })
            })
            .collect()
    }
    fn current(&self, snapshot: &Snapshot) -> bool {
        snapshot.projects == self.snapshot.projects
            && self
                .snapshot
                .notes
                .iter()
                .all(|n| snapshot.notes.contains(n))
    }
}
struct Failure {
    message: String,
    pause: bool,
}
async fn decide(
    client: &reqwest::Client,
    endpoint: &str,
    key: &str,
    batch: &Batch,
) -> Result<Vec<NoteRouteProjectInput>, Failure> {
    let fail = |message: &str| Failure {
        message: message.into(),
        pause: false,
    };
    let mut response = client
        .post(endpoint)
        .bearer_auth(key)
        .header("Content-Type", "application/json")
        .body(batch.body.clone())
        .send()
        .await
        .map_err(|_| fail("Organization request failed. Will retry."))?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        return Err(Failure {
            message: format!(
                "Organization provider returned HTTP {status}. Check your key and credits."
            ),
            pause: matches!(status, 401..=403),
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| fail("Organization response failed"))?
    {
        if bytes.len() + chunk.len() > BODY_LIMIT {
            return Err(fail("Organization response is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    batch
        .routes(&serde_json::from_slice(&bytes).map_err(|_| fail("Invalid organization response"))?)
        .map_err(fail)
}

/// Run in a GUI-owned task. Dropping/aborting it cancels HTTP, watch and retry futures.
pub async fn run(
    db: PowerSyncDatabase,
    app: Arc<Application>,
    user: String,
    cutoff: String,
    credential: watch::Receiver<Credential>,
    status: watch::Sender<Option<String>>,
) {
    run_with_endpoint(
        db,
        app,
        user,
        cutoff,
        credential,
        status,
        (ENDPOINT, Duration::from_secs(2)),
    )
    .await;
}
type Failures = BTreeMap<String, (u32, tokio::time::Instant)>;
fn choose(
    snapshot: &Snapshot,
    failures: &mut Failures,
    status: &watch::Sender<Option<String>>,
) -> (Option<Batch>, Option<tokio::time::Instant>) {
    if snapshot.projects.is_empty() {
        return (None, None);
    }
    if let Err(e) = Batch::new(&snapshot.projects, &[]) {
        status.send_replace(Some(e.into()));
        return (None, None);
    }
    let mut offset = 0;
    let mut next_retry = None;
    let mut signatures = std::collections::BTreeSet::new();
    let mut chosen = None;
    while offset < snapshot.notes.len() {
        let group = &snapshot.notes[offset..];
        let mut size = group.len().min(8);
        let batch = loop {
            match Batch::new(&snapshot.projects, &group[..size]) {
                Ok(batch) => break Some(batch),
                Err(e) if size == 1 => {
                    status.send_replace(Some(e.into()));
                    break None;
                }
                Err(_) => size -= 1,
            }
        };
        offset += size;
        let Some(batch) = batch else {
            continue;
        };
        signatures.insert(batch.signature.clone());
        if let Some((attempts, until)) = failures.get(&batch.signature) {
            if *attempts >= 3 {
                continue;
            }
            if *until > tokio::time::Instant::now() {
                next_retry =
                    Some(next_retry.map_or(*until, |old: tokio::time::Instant| old.min(*until)));
                continue;
            }
        }
        if chosen.is_none() {
            chosen = Some(batch);
        }
    }
    failures.retain(|signature, _| signatures.contains(signature));
    (chosen, next_retry)
}
async fn apply_response(
    db: &PowerSyncDatabase,
    app: &Application,
    params: &[String],
    batch: &Batch,
    validity: (u64, &watch::Receiver<Credential>),
    result: Result<Vec<NoteRouteProjectInput>, Failure>,
    #[cfg(test)] gap: Option<(&tokio::sync::Notify, &tokio::sync::Notify)>,
) -> Result<(Snapshot, bool), Failure> {
    // Re-read once after HTTP: no timer/polling; catches writes queued with the response.
    let fail = || Failure {
        message: "Could not verify or apply organization. Changes will be checked before retry."
            .into(),
        pause: false,
    };
    let writer = db.writer().await.map_err(|_| fail())?;
    let mut current = writer
        .prepare(sql())
        .and_then(|mut stmt| read(&mut stmt, params))
        .map_err(|_| fail())?;
    drop(writer);
    if !batch.current(&current) || validity.1.borrow().generation != validity.0 {
        return Ok((current, false));
    }
    let routes = result?;
    #[cfg(test)]
    if let Some((entered, release)) = gap {
        entered.notify_one();
        release.notified().await;
    }
    let local = flicknote_core::backend::LocalPowerSyncBackend::new(db.clone(), params[0].clone());
    let applied = app
        .route_project_locally_guarded(routes, &local, |transaction| {
            current = transaction
                .prepare(sql())
                .and_then(|mut stmt| read(&mut stmt, params))?;
            Ok(batch.current(&current) && validity.1.borrow().generation == validity.0)
        })
        .await
        .map_err(|_| fail())?;
    if applied {
        current.notes.retain(|n| !batch.snapshot.notes.contains(n));
    }
    Ok((current, applied))
}
fn record_failure(
    failures: &mut Failures,
    batch: &Batch,
    backoff: Duration,
    failure: Failure,
    status: &watch::Sender<Option<String>>,
) {
    let entry = failures
        .entry(batch.signature.clone())
        .or_insert((0, tokio::time::Instant::now()));
    entry.0 += 1;
    entry.1 = tokio::time::Instant::now() + backoff * entry.0;
    status.send_replace(Some(if entry.0 >= 3 {
        format!(
            "{} Automatic retries paused; edit input or configuration to resume.",
            failure.message
        )
    } else {
        failure.message
    }));
}
pub async fn run_with_endpoint(
    db: PowerSyncDatabase,
    app: Arc<Application>,
    user: String,
    cutoff: String,
    mut credential: watch::Receiver<Credential>,
    status: watch::Sender<Option<String>>,
    provider: (&str, Duration),
) {
    let (endpoint, backoff) = provider;
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            status.send_replace(Some("Could not initialize organization".into()));
            return;
        }
    };
    let params = [user, cutoff];
    let stream = db.watch_statement(sql().to_owned(), params.clone(), |stmt, params| {
        read(stmt, &params).map_err(Into::into)
    });
    futures_lite::pin!(stream);
    let mut snapshot = Snapshot::default();
    let mut failures: BTreeMap<String, (u32, tokio::time::Instant)> = BTreeMap::new();
    let mut paused = false;
    let mut generation = credential.borrow().generation;
    loop {
        let config = credential.borrow_and_update().clone();
        if generation != config.generation {
            failures.clear();
            paused = false;
            status.send_replace(None);
            generation = config.generation;
        }
        let (chosen, next_retry) = if config.key.is_some() && !paused {
            choose(&snapshot, &mut failures, &status)
        } else {
            (None, None)
        };
        if let Some(batch) = chosen {
            let key = config.key.as_deref().expect("enabled credential");
            let request = decide(&client, endpoint, key, &batch);
            tokio::pin!(request);
            let result = loop {
                tokio::select! {
                    biased;
                    _ = credential.changed() => break None,
                    result = &mut request => break Some(result),
                    updated = stream.next() => {
                        match updated { Some(Ok(s))=>snapshot=s, Some(Err(_))=> { status.send_replace(Some("Could not watch organization inputs".into())); break None; }, None=>return }
                        if !batch.current(&snapshot) { break None; }
                    }
                }
            };
            let Some(result) = result else {
                continue;
            };
            let validation = credential.clone();
            let application = tokio::select! { biased;
                _=credential.changed()=>continue,
                result=apply_response(
                &db,
                &app,
                &params,
                &batch,
                (generation, &validation),
                result,
                #[cfg(test)]
                None,
            ) => result,
            };
            match application {
                Ok((current, applied)) => {
                    snapshot = current;
                    if applied {
                        status.send_replace(None);
                        failures.remove(&batch.signature);
                    }
                }
                Err(f) => {
                    paused = f.pause;
                    record_failure(&mut failures, &batch, backoff, f, &status);
                }
            }
            continue;
        }
        tokio::select! {
            _ = credential.changed()=>{},
            _ = async { match next_retry { Some(at)=>tokio::time::sleep_until(at).await, None=>std::future::pending().await } }=>{},
            updated=stream.next()=>match updated { Some(Ok(s))=>snapshot=s, Some(Err(_))=>{status.send_replace(Some("Could not watch organization inputs".into()));}, None=>return }
        }
    }
}

#[cfg(test)]
#[path = "organization_tests.rs"]
mod tests;
