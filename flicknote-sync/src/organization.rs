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
    pub catch_up: Option<CatchUp>,
}
/// A manual, process-local interval. Previewing never enables credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatchUp {
    pub days: u32,
    pub start: String,
    pub end: String,
    pub running: bool,
}
impl CatchUp {
    pub fn recent<T: chrono::TimeZone>(
        now: &chrono::DateTime<T>,
        days: u32,
        running: bool,
    ) -> Result<Self, String> {
        if !matches!(days, 3 | 7) {
            return Err("Choose 3 or 7 days".into());
        }
        let (today, _) = crate::today::bounds(now)?;
        let day = today
            .with_timezone(&now.timezone())
            .date_naive()
            .checked_sub_days(chrono::Days::new(u64::from(days - 1)))
            .ok_or("Calendar underflow")?;
        let start = now
            .timezone()
            .from_local_datetime(&day.and_hms_opt(4, 0, 0).expect("valid time"))
            .earliest()
            .ok_or("04:00 does not exist in local calendar")?;
        Ok(Self {
            days,
            start: start.with_timezone(&chrono::Utc).to_rfc3339(),
            end: now.with_timezone(&chrono::Utc).to_rfc3339(),
            running,
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CatchPhase {
    #[default]
    Preview,
    Running,
    Complete,
    Stopped,
    Unfinished,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatchProgress {
    pub days: u32,
    pub phase: CatchPhase,
    pub initial: usize,
    pub processed: usize,
    pub remaining: usize,
    pub failed: usize,
    pub skipped: usize,
}
pub struct RoutingControl {
    pub credential: watch::Receiver<Credential>,
    pub status: watch::Sender<Option<String>>,
    pub catch_up: watch::Sender<CatchProgress>,
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
    regular: bool,
    catch_up: bool,
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
    "SELECT 'p', id, name, json_extract(metadata, '$.summary'), NULL, NULL, NULL FROM projects WHERE user_id = ?1 AND coalesce(is_archived, 0) = 0 UNION ALL SELECT 'n', id, short_id, summary, coalesce(content, ''), julianday(created_at) >= julianday(?2), julianday(created_at) >= julianday(?3) AND julianday(created_at) <= julianday(?4) FROM notes WHERE user_id = ?1 AND deleted_at IS NULL AND status = 'ready' AND project_id IS NULL AND short_id > 0 AND (julianday(created_at) >= julianday(?2) OR (julianday(created_at) >= julianday(?3) AND julianday(created_at) <= julianday(?4))) AND coalesce(json_type(metadata, '$.project_routing.routed'), '') != 'true' ORDER BY 1, 2"
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
                    regular: row.get(5)?,
                    catch_up: row.get::<_, Option<bool>>(6)?.unwrap_or(false),
                });
            }
        }
    }
    snapshot.notes.sort_by_key(|n| (!n.regular, n.uuid.clone()));
    Ok(snapshot)
}
struct Batch {
    snapshot: Snapshot,
    body: Vec<u8>,
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
        Ok(Self {
            snapshot: Snapshot {
                projects: projects.to_vec(),
                notes: notes.to_vec(),
            },
            body,
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

type Failures = BTreeMap<String, (u32, tokio::time::Instant, Note)>;
fn failure_key(projects: &[Project], note: &Note) -> String {
    // Eligibility/priority and positional batch membership are not canonical input.
    format!("{projects:?}:{:?}:{:?}", note.uuid, note.text)
}
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
    let mut signatures = std::collections::BTreeSet::new();
    let mut notes = Vec::new();
    let mut next_retry = None;
    for note in &snapshot.notes {
        let signature = failure_key(&snapshot.projects, note);
        signatures.insert(signature.clone());
        if let Some((attempts, until, _)) = failures.get(&signature) {
            if *attempts >= 3 {
                continue;
            }
            if *until > tokio::time::Instant::now() {
                next_retry =
                    Some(next_retry.map_or(*until, |old: tokio::time::Instant| old.min(*until)));
                continue;
            }
        }
        notes.push(note.clone());
    }
    failures.retain(|signature, _| signatures.contains(signature));
    let mut offset = 0;
    let mut chosen = None;
    while offset < notes.len() {
        let group = &notes[offset..];
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
        if chosen.is_none() {
            chosen = Some(batch);
        }
    }
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
    let mut exhausted = false;
    for note in &batch.snapshot.notes {
        let entry = failures
            .entry(failure_key(&batch.snapshot.projects, note))
            .or_insert((0, tokio::time::Instant::now(), note.clone()));
        entry.0 += 1;
        entry.1 = tokio::time::Instant::now() + backoff * entry.0;
        exhausted |= entry.0 >= 3;
    }
    status.send_replace(Some(if exhausted {
        format!(
            "{} Automatic retries paused; edit input or configuration to resume.",
            failure.message
        )
    } else {
        failure.message
    }));
}
#[derive(Default)]
struct Coordinator {
    params: [String; 4],
    snapshot: Snapshot,
    failures: Failures,
    paused: bool,
    generation: Option<u64>,
    loaded: bool,
    active: bool,
    initialized: bool,
    progress: CatchProgress,
    seen: std::collections::BTreeSet<i64>,
    last_start: Option<tokio::time::Instant>,
}
impl Coordinator {
    fn configure(
        &mut self,
        config: &Credential,
        user: &str,
        cutoff: &str,
        status: &watch::Sender<Option<String>>,
        catch_up: &watch::Sender<CatchProgress>,
    ) -> bool {
        if self.generation == Some(config.generation) {
            return false;
        }
        self.failures.clear();
        self.paused = false;
        status.send_replace(None);
        self.generation = Some(config.generation);
        if let Some(range) = &config.catch_up {
            self.params = [
                user.to_owned(),
                cutoff.to_owned(),
                range.start.clone(),
                range.end.clone(),
            ];
            self.active = range.running && config.key.is_some();
            self.initialized = false;
            self.seen.clear();
            self.progress = CatchProgress {
                days: range.days,
                phase: if self.active {
                    CatchPhase::Running
                } else {
                    CatchPhase::Preview
                },
                ..Default::default()
            };
        } else {
            self.params = [
                user.to_owned(),
                cutoff.to_owned(),
                String::new(),
                String::new(),
            ];
            if self.active {
                self.progress.phase = CatchPhase::Stopped;
            }
            self.active = false;
        }
        self.snapshot = Snapshot::default();
        self.loaded = false;
        catch_up.send_replace(self.progress.clone());
        true
    }
    fn choose(
        &mut self,
        config: &Credential,
        status: &watch::Sender<Option<String>>,
        catch_up: &watch::Sender<CatchProgress>,
    ) -> (Option<Batch>, Option<tokio::time::Instant>) {
        let current: std::collections::BTreeSet<_> = self
            .snapshot
            .notes
            .iter()
            .filter(|n| n.catch_up)
            .map(|n| n.id)
            .collect();
        if self.loaded
            && config.catch_up.is_some()
            && (!self.initialized || self.active || self.progress.phase == CatchPhase::Preview)
        {
            if !self.initialized {
                self.progress.initial = current.len();
                self.initialized = true;
            }
            self.progress.skipped += self.seen.difference(&current).count();
            self.seen = current.clone();
            self.progress.remaining = current.len();
            self.progress.failed = self
                .failures
                .values()
                .filter(|(attempts, _, _)| *attempts >= 3 || self.paused)
                .map(|(_, _, note)| note)
                .filter(|n| n.catch_up && current.contains(&n.id))
                .map(|n| n.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            catch_up.send_replace(self.progress.clone());
        }
        let candidates = Snapshot {
            projects: self.snapshot.projects.clone(),
            notes: self
                .snapshot
                .notes
                .iter()
                .filter(|n| n.regular || self.active)
                .cloned()
                .collect(),
        };
        let (chosen, next_retry) = if self.loaded && config.key.is_some() && !self.paused {
            choose(&candidates, &mut self.failures, status)
        } else {
            (None, None)
        };
        if self.loaded
            && self.active
            && (self.progress.remaining == 0 || (chosen.is_none() && next_retry.is_none()))
        {
            self.active = false;
            self.progress.phase = if self.progress.remaining == 0 {
                CatchPhase::Complete
            } else {
                CatchPhase::Unfinished
            };
            catch_up.send_replace(self.progress.clone());
        }
        (chosen, next_retry)
    }
    fn cadence_deadline(&self) -> Option<tokio::time::Instant> {
        self.last_start
            .filter(|_| self.active)
            .map(|at| at + Duration::from_secs(10))
            .filter(|until| *until > tokio::time::Instant::now())
    }
    fn applied(
        &mut self,
        result: Result<(Snapshot, bool), Failure>,
        batch: &Batch,
        backoff: Duration,
        status: &watch::Sender<Option<String>>,
    ) {
        match result {
            Ok((current, applied)) => {
                self.snapshot = current;
                if applied {
                    if self.active {
                        for note in batch.snapshot.notes.iter().filter(|n| n.catch_up) {
                            self.progress.processed += 1;
                            self.seen.remove(&note.id);
                        }
                    }
                    status.send_replace(None);
                    for note in &batch.snapshot.notes {
                        self.failures
                            .remove(&failure_key(&batch.snapshot.projects, note));
                    }
                }
            }
            Err(f) => {
                self.paused = f.pause;
                record_failure(&mut self.failures, batch, backoff, f, status);
            }
        }
    }
}
fn provider_client(status: &watch::Sender<Option<String>>) -> Option<reqwest::Client> {
    match reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => Some(c),
        Err(_) => {
            status.send_replace(Some("Could not initialize organization".into()));
            None
        }
    }
}
/// GUI-owned singleflight actor. Aborting it cancels HTTP, watch and retry futures.
pub async fn run_with_endpoint(
    db: PowerSyncDatabase,
    app: Arc<Application>,
    user: String,
    cutoff: String,
    control: RoutingControl,
    provider: (&str, Duration),
) {
    let RoutingControl {
        mut credential,
        status,
        catch_up,
    } = control;
    let (endpoint, backoff) = provider;
    let Some(client) = provider_client(&status) else {
        return;
    };
    let subscribe = |params: [String; 4]| {
        db.watch_statement(sql().to_owned(), params, |stmt, params| {
            read(stmt, &params).map_err(Into::into)
        })
    };
    let mut state = Coordinator::default();
    let mut stream = Box::pin(subscribe(state.params.clone()));
    loop {
        let config = credential.borrow_and_update().clone();
        if state.configure(&config, &user, &cutoff, &status, &catch_up) {
            stream = Box::pin(subscribe(state.params.clone()));
        }
        let (chosen, next_retry) = state.choose(&config, &status, &catch_up);
        if let Some(batch) = chosen {
            if let Some(until) = state.cadence_deadline() {
                tokio::select! { biased;
                    _ = credential.changed() => {},
                    updated = stream.next() => match updated { Some(Ok(s)) => { state.snapshot = s; state.loaded = true; }, Some(Err(_)) => { status.send_replace(Some("Could not watch organization inputs".into())); }, None => return },
                    () = tokio::time::sleep_until(until) => {},
                }
                continue;
            }
            state.last_start = Some(tokio::time::Instant::now());
            let key = config.key.as_deref().expect("enabled credential");
            let request = decide(&client, endpoint, key, &batch);
            tokio::pin!(request);
            let result = loop {
                tokio::select! {
                    biased;
                    _ = credential.changed() => break None,
                    result = &mut request => break Some(result),
                    updated = stream.next() => {
                        match updated { Some(Ok(s))=>{state.snapshot=s; state.loaded=true;}, Some(Err(_))=> { status.send_replace(Some("Could not watch organization inputs".into())); break None; }, None=>return }
                        if !batch.current(&state.snapshot) { break None; }
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
                &state.params,
                &batch,
                (config.generation, &validation),
                result,
                #[cfg(test)]
                None,
            ) => result,
            };
            state.applied(application, &batch, backoff, &status);
            continue;
        }
        tokio::select! {
            _ = credential.changed()=>{},
            _ = async { match next_retry { Some(at)=>tokio::time::sleep_until(at).await, None=>std::future::pending().await } }=>{},
            updated=stream.next()=>match updated { Some(Ok(s))=>{state.snapshot=s; state.loaded=true;}, Some(Err(_))=>{status.send_replace(Some("Could not watch organization inputs".into()));}, None=>return }
        }
    }
}

#[cfg(test)]
#[path = "organization_tests.rs"]
mod tests;
