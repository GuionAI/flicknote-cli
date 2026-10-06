//! Account-scoped, non-secret GUI preference and isolated Keychain adapter.
use flicknote_sync::organization::{
    CatchPhase, CatchProgress, CatchUp, Credential, RoutingControl,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{mpsc, oneshot, watch};

const SERVICE: &str = "ai.guion.flicknote.gpui.automatic-organization.v1";
pub(crate) trait SecretStore: Send + Sync {
    fn read(&self, account: &str) -> Result<Option<String>, String>;
    fn save(&self, account: &str, key: &str) -> Result<(), String>;
    fn remove(&self, account: &str) -> Result<(), String>;
}
struct Keychain;
impl SecretStore for Keychain {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        match security_framework::passwords::get_generic_password(SERVICE, account) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| "Stored organization key is invalid. Replace it.".into()),
            Err(e) if e.code() == -25300 => Ok(None),
            Err(_) => Err("Could not read the organization key. Check Keychain access.".into()),
        }
    }
    fn save(&self, account: &str, key: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(SERVICE, account, key.as_bytes())
            .map_err(|_| "Could not save the organization key. Check Keychain access.".into())
    }
    fn remove(&self, account: &str) -> Result<(), String> {
        match security_framework::passwords::delete_generic_password(SERVICE, account) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == -25300 => Ok(()),
            Err(_) => Err("Could not remove the organization key. Check Keychain access.".into()),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Preference {
    cutoff: String,
    enabled: bool,
}
fn load(
    path: &Path,
    account: &str,
    started: &str,
) -> Result<(BTreeMap<String, Preference>, Preference), String> {
    let mut prefs: BTreeMap<String, Preference> = match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_| "Could not read organization preferences")?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(_) => return Err("Could not read organization preferences".into()),
    };
    if let Some(p) = prefs.get(account) {
        chrono::DateTime::parse_from_rfc3339(&p.cutoff)
            .map_err(|_| "Invalid organization cutoff")?;
        return Ok((prefs.clone(), p.clone()));
    }
    let p = Preference {
        cutoff: started.into(),
        enabled: false,
    };
    prefs.insert(account.into(), p.clone());
    persist(path, &prefs)?;
    Ok((prefs, p))
}
fn persist(path: &Path, prefs: &BTreeMap<String, Preference>) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(prefs).map_err(|_| "Could not save organization preferences")?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|_| "Could not save organization preferences")?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "Could not save organization preferences")?;
    std::fs::rename(tmp, path).map_err(|_| "Could not save organization preferences".into())
}
#[derive(Clone, Default)]
pub(crate) struct State {
    pub enabled: bool,
    pub has_key: bool,
    pub error: Option<String>,
    pub ready: bool,
}
pub(crate) enum Change {
    Save(String),
    Enable(bool),
    Remove,
    Preview(u32),
    Start(u32),
    Stop,
}
struct Request {
    change: Change,
    reply: oneshot::Sender<Result<(), String>>,
}
#[derive(Clone)]
pub(crate) struct Control {
    pub opened: watch::Sender<bool>,
    commands: mpsc::Sender<Request>,
    pub state: watch::Receiver<State>,
    pub routing_error: watch::Receiver<Option<String>>,
    pub catch_up: watch::Receiver<CatchProgress>,
}
impl Control {
    pub(crate) async fn change(&self, change: Change) -> Result<(), String> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Request { change, reply })
            .await
            .map_err(|_| "Organization stopped")?;
        result.await.map_err(|_| "Organization stopped")?
    }
}
pub(crate) fn start(services: &crate::ui::Services, root: &Path) -> Control {
    start_with_store(
        services,
        root.join("gui-organization.json"),
        Arc::new(Keychain),
        chrono::Utc::now().to_rfc3339(),
    )
}
pub(crate) fn start_with_store(
    services: &crate::ui::Services,
    path: PathBuf,
    store: Arc<dyn SecretStore>,
    started: String,
) -> Control {
    start_with_provider(
        services,
        path,
        store,
        started,
        flicknote_sync::organization::ENDPOINT.into(),
    )
}
pub(crate) fn start_with_provider(
    services: &crate::ui::Services,
    path: PathBuf,
    store: Arc<dyn SecretStore>,
    started: String,
    endpoint: String,
) -> Control {
    let (commands, requests) = mpsc::channel::<Request>(8);
    let (state, state_receiver) = watch::channel(State::default());
    let (status, routing_error) = watch::channel(None);
    let (progress, catch_up) = watch::channel(CatchProgress::default());
    let (credentials, credential) = watch::channel(Credential::default());
    let app = services.app.clone();
    let db = services.db.clone();
    let account = services.user_id.clone();
    let job = services.runtime.spawn(async move {
        coordinate(
            Setup {
                path,
                store,
                account,
                started,
                endpoint,
            },
            requests,
            state,
            credentials,
            Routing {
                db,
                app,
                credential,
                status,
                progress,
            },
        )
        .await;
    });
    services.track(&job);
    Control {
        opened: watch::channel(false).0,
        commands,
        state: state_receiver,
        routing_error,
        catch_up,
    }
}

struct Setup {
    path: PathBuf,
    store: Arc<dyn SecretStore>,
    account: String,
    started: String,
    endpoint: String,
}
struct Cancel(tokio::task::AbortHandle);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct Routing {
    db: flicknote_sync::PowerSyncDatabase,
    app: Arc<flicknote_sync::app::Application>,
    credential: watch::Receiver<Credential>,
    status: watch::Sender<Option<String>>,
    progress: watch::Sender<CatchProgress>,
}
fn manual_range(
    change: &Change,
    usable: bool,
    credentials: &watch::Sender<Credential>,
    progress: &watch::Sender<CatchProgress>,
) -> Result<Option<CatchUp>, String> {
    match change {
        Change::Preview(_) if progress.borrow().phase == CatchPhase::Running => {
            Err("Stop the current Catch up first".into())
        }
        Change::Preview(days) => CatchUp::recent(&chrono::Local::now(), *days, false).map(Some),
        Change::Start(_) if !usable => Err("Enable organization with a saved key first".into()),
        Change::Start(_)
            if progress.borrow().phase == CatchPhase::Running
                || credentials
                    .borrow()
                    .catch_up
                    .as_ref()
                    .is_some_and(|r| r.running)
                    && progress.borrow().phase == CatchPhase::Preview =>
        {
            Err("Catch up is already running".into())
        }
        Change::Start(days) => CatchUp::recent(&chrono::Local::now(), *days, true).map(Some),
        Change::Stop => Ok(None),
        _ => unreachable!(),
    }
}
fn spawn_router(routing: Routing, account: String, cutoff: String, endpoint: String) -> Cancel {
    let task = tokio::spawn(async move {
        flicknote_sync::organization::run_with_endpoint(
            routing.db,
            routing.app,
            account,
            cutoff,
            RoutingControl {
                credential: routing.credential,
                status: routing.status,
                catch_up: routing.progress,
            },
            (&endpoint, std::time::Duration::from_secs(2)),
        )
        .await;
    });
    Cancel(task.abort_handle())
}
async fn read_key(store: Arc<dyn SecretStore>, user: String) -> (Option<String>, Option<String>) {
    match tokio::task::spawn_blocking(move || store.read(&user)).await {
        Ok(Ok(k)) => (k, None),
        _ => (
            None,
            Some("Could not read organization key. Replace it to recover.".into()),
        ),
    }
}
async fn persist_change(
    setup: &Setup,
    current: BTreeMap<String, Preference>,
    old: Preference,
    existing_key: Option<String>,
    change: Change,
) -> Result<Saved, String> {
    let store = setup.store.clone();
    let user = setup.account.clone();
    let path = setup.path.clone();
    tokio::task::spawn_blocking(move || {
        apply(&path, &user, &*store, current, old, existing_key, change)
    })
    .await
    .map_err(|_| "Could not change organization configuration".to_owned())?
}
async fn coordinate(
    setup: Setup,
    mut requests: mpsc::Receiver<Request>,
    state: watch::Sender<State>,
    credentials: watch::Sender<Credential>,
    routing: Routing,
) {
    let path = setup.path.clone();
    let account = setup.account.clone();
    let started = setup.started.clone();
    let loaded = tokio::task::spawn_blocking(move || load(&path, &account, &started)).await;
    let (mut prefs, mut preference) = match loaded {
        Ok(Ok(p)) => p,
        _ => {
            state.send_replace(State { error: Some("Could not initialize organization preferences. Reopen after fixing storage access.".into()), ..State::default() });
            return;
        }
    };
    let (mut key, error) = read_key(setup.store.clone(), setup.account.clone()).await;
    let mut generation = 0;
    state.send_replace(State {
        ready: true,
        enabled: preference.enabled,
        has_key: key.is_some(),
        error,
    });
    credentials.send_replace(Credential {
        key: preference.enabled.then(|| key.clone()).flatten(),
        generation,
        catch_up: None,
    });
    let progress = routing.progress.clone();
    let _cancel = spawn_router(
        routing,
        setup.account.clone(),
        preference.cutoff.clone(),
        setup.endpoint.clone(),
    );
    while let Some(request) = requests.recv().await {
        generation += 1;
        if matches!(
            request.change,
            Change::Preview(_) | Change::Start(_) | Change::Stop
        ) {
            let range = manual_range(
                &request.change,
                preference.enabled && key.is_some(),
                &credentials,
                &progress,
            );
            let result = range.map(|catch_up| {
                credentials.send_replace(Credential {
                    key: preference.enabled.then(|| key.clone()).flatten(),
                    generation,
                    catch_up,
                });
            });
            let _sent = request.reply.send(result);
            continue;
        }
        credentials.send_replace(Credential {
            key: None,
            generation,
            catch_up: None,
        });
        let result = persist_change(
            &setup,
            prefs.clone(),
            preference.clone(),
            key.clone(),
            request.change,
        )
        .await;
        match result {
            Ok((updated, p, k)) => {
                prefs = updated;
                preference = p;
                key = k;
                state.send_replace(State {
                    ready: true,
                    enabled: preference.enabled,
                    has_key: key.is_some(),
                    error: None,
                });
                credentials.send_replace(Credential {
                    key: preference.enabled.then(|| key.clone()).flatten(),
                    generation,
                    catch_up: None,
                });
                let _sent = request.reply.send(Ok(()));
            }
            Err(error) => {
                // Keychain may have changed before a preference write failed. Never reactivate cached secrets.
                key = None;
                state.send_replace(State {
                    ready: true,
                    enabled: false,
                    has_key: false,
                    error: Some(error.clone()),
                });
                let _sent = request.reply.send(Err(error));
            }
        }
    }
}
type Saved = (BTreeMap<String, Preference>, Preference, Option<String>);
fn apply(
    path: &Path,
    user: &str,
    store: &dyn SecretStore,
    mut prefs: BTreeMap<String, Preference>,
    mut preference: Preference,
    mut key: Option<String>,
    change: Change,
) -> Result<Saved, String> {
    match change {
        Change::Save(value) => {
            let value = value.trim();
            if value.is_empty() {
                return Err("Enter an OpenRouter key".into());
            }
            // A failed replacement must remain disabled on restart, even after Keychain changed.
            preference.enabled = false;
            prefs.insert(user.into(), preference.clone());
            persist(path, &prefs)?;
            store.save(user, value)?;
            key = Some(value.to_owned());
            preference.enabled = true;
        }
        Change::Remove => {
            store.remove(user)?;
            key = None;
            preference.enabled = false;
        }
        Change::Preview(_) | Change::Start(_) | Change::Stop => {
            unreachable!("manual run does not persist preferences")
        }
        Change::Enable(enabled) => {
            if enabled && key.is_none() {
                return Err("Save an OpenRouter key first".into());
            }
            preference.enabled = enabled;
        }
    }
    prefs.insert(user.into(), preference.clone());
    persist(path, &prefs)?;
    Ok((prefs, preference, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    #[derive(Default)]
    struct FakeStore {
        keys: Mutex<BTreeMap<String, String>>,
        fail: AtomicBool,
        block_after_save: Mutex<Option<PathBuf>>,
    }
    impl SecretStore for FakeStore {
        fn read(&self, account: &str) -> Result<Option<String>, String> {
            Ok(self.keys.lock().unwrap().get(account).cloned())
        }
        fn save(&self, account: &str, key: &str) -> Result<(), String> {
            if self.fail.load(Ordering::SeqCst) {
                return Err("Fake save failure".into());
            }
            self.keys.lock().unwrap().insert(account.into(), key.into());
            if let Some(path) = self.block_after_save.lock().unwrap().take() {
                std::fs::create_dir(path).unwrap();
            }
            Ok(())
        }
        fn remove(&self, account: &str) -> Result<(), String> {
            self.keys.lock().unwrap().remove(account);
            Ok(())
        }
    }
    #[test]
    fn cutoff_survives_restart_missing_key_and_separates_accounts() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("prefs.json");
        let (_, first) = load(&path, "account-a", "2026-10-06T00:00:00Z").unwrap();
        assert!(!first.enabled);
        let (_, restart) = load(&path, "account-a", "2026-11-06T00:00:00Z").unwrap();
        assert_eq!(restart.cutoff, first.cutoff);
        let (_, other) = load(&path, "account-b", "2026-11-06T00:00:00Z").unwrap();
        assert_ne!(first.cutoff, other.cutoff);
        let (_, again) = load(&path, "account-a", "2026-12-06T00:00:00Z").unwrap();
        assert_eq!(again.cutoff, first.cutoff);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[tokio::test]
    async fn fake_secret_failure_save_disable_remove_and_restart() {
        let root = tempfile::tempdir().unwrap();
        let host =
            flicknote_sync::spike::SpikeHost::start(root.path(), 0, 0, std::time::Duration::ZERO)
                .await
                .unwrap();
        let host = crate::launch::Host::Synthetic(host);
        let services = host.services(tokio::runtime::Handle::current());
        let path = root.path().join("organization.json");
        let store = Arc::new(FakeStore::default());
        let control = start_with_store(
            &services,
            path.clone(),
            store.clone(),
            "2026-10-06T00:00:00Z".into(),
        );
        let mut state = control.state.clone();
        ready(&mut state).await;
        assert!(!state.borrow().enabled);
        assert!(!state.borrow().has_key);
        assert!(control.change(Change::Enable(true)).await.is_err());
        store.fail.store(true, Ordering::SeqCst);
        assert!(
            control
                .change(Change::Save("fixture-secret".into()))
                .await
                .is_err()
        );
        assert!(!state.borrow().enabled);
        store.fail.store(false, Ordering::SeqCst);
        control
            .change(Change::Save("fixture-secret".into()))
            .await
            .unwrap();
        assert!(state.borrow().enabled);
        assert!(state.borrow().has_key);
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("fixture-secret")
        );
        drop(control);
        services.cancel_operations();
        let control = start_with_store(
            &services,
            path.clone(),
            store.clone(),
            "2026-11-06T00:00:00Z".into(),
        );
        let mut state = control.state.clone();
        ready(&mut state).await;
        assert!(state.borrow().enabled);
        assert!(state.borrow().has_key);
        assert_eq!(
            load(&path, &services.user_id, "2027-01-01T00:00:00Z")
                .unwrap()
                .1
                .cutoff,
            "2026-10-06T00:00:00Z"
        );
        control.change(Change::Enable(false)).await.unwrap();
        assert!(!state.borrow().enabled);
        assert!(state.borrow().has_key);
        let blocked = path.with_extension("json.tmp");
        std::fs::create_dir(&blocked).unwrap();
        assert!(
            control
                .change(Change::Save("replacement-fixture-secret".into()))
                .await
                .is_err()
        );
        assert!(!state.borrow().enabled);
        assert!(!state.borrow().has_key);
        assert!(
            !state
                .borrow()
                .error
                .as_ref()
                .unwrap()
                .contains("replacement-fixture-secret")
        );
        std::fs::remove_dir(blocked).unwrap();
        control
            .change(Change::Save("fixture-secret".into()))
            .await
            .unwrap();
        control.change(Change::Enable(true)).await.unwrap();
        assert!(state.borrow().enabled);
        control.change(Change::Remove).await.unwrap();
        assert!(!state.borrow().enabled);
        assert!(!state.borrow().has_key);
        assert!(store.keys.lock().unwrap().is_empty());
        drop(control);
        services.cancel_operations();
        host.run_until(async { Ok(()) }, || {}).await.unwrap();
    }
    async fn ready(state: &mut watch::Receiver<State>) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !state.borrow().ready {
                state.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }

    #[test]
    fn replacement_final_write_failure_stays_disabled_on_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("prefs.json");
        let store = FakeStore::default();
        let (prefs, preference) = load(&path, "account", "2026-10-06T00:00:00Z").unwrap();
        let cutoff = preference.cutoff.clone();
        let (prefs, preference, key) = apply(
            &path,
            "account",
            &store,
            prefs,
            preference,
            None,
            Change::Save("original-fixture-secret".into()),
        )
        .unwrap();
        assert!(
            load(&path, "account", "2026-11-06T00:00:00Z")
                .unwrap()
                .1
                .enabled
        );
        let blocked = path.with_extension("json.tmp");
        *store.block_after_save.lock().unwrap() = Some(blocked.clone());
        assert!(
            apply(
                &path,
                "account",
                &store,
                prefs,
                preference,
                key,
                Change::Save("replacement-fixture-secret".into())
            )
            .is_err()
        );
        assert_eq!(
            store.read("account").unwrap().as_deref(),
            Some("replacement-fixture-secret")
        );
        std::fs::remove_dir(blocked).unwrap();
        let (_, restarted) = load(&path, "account", "2027-01-01T00:00:00Z").unwrap();
        assert_eq!(restarted.cutoff, cutoff);
        assert!(
            !restarted.enabled,
            "failed Save must not enable the replacement on restart"
        );
        assert!(
            !std::fs::read_to_string(path)
                .unwrap()
                .contains("fixture-secret")
        );
    }

    #[test]
    fn replacement_aborts_when_durable_disable_fails() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("prefs.json");
        let store = FakeStore::default();
        let (prefs, preference) = load(&path, "account", "2026-10-06T00:00:00Z").unwrap();
        let (prefs, preference, key) = apply(
            &path,
            "account",
            &store,
            prefs,
            preference,
            None,
            Change::Save("original-fixture-secret".into()),
        )
        .unwrap();
        std::fs::create_dir(path.with_extension("json.tmp")).unwrap();
        assert!(
            apply(
                &path,
                "account",
                &store,
                prefs,
                preference,
                key,
                Change::Save("replacement-fixture-secret".into())
            )
            .is_err()
        );
        assert_eq!(
            store.read("account").unwrap().as_deref(),
            Some("original-fixture-secret")
        );
    }
}
