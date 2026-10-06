//! Process-owned creation-channel choice. Disk work never runs on the GPUI thread.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tokio::sync::watch;

#[derive(Clone, Debug)]
pub(crate) struct State {
    pub human_only: bool,
    pub ready: bool,
    pub generation: u64,
    pub persisted: bool,
    pub error: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            human_only: false,
            ready: true,
            generation: 0,
            persisted: true,
            error: None,
        }
    }
}
#[derive(Clone)]
pub(crate) struct Control {
    state: watch::Sender<State>,
    disk: bool,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            state: watch::channel(State::default()).0,
            disk: false,
        }
    }
}
impl Control {
    pub(crate) fn state(&self) -> State {
        self.state.borrow().clone()
    }
    pub(crate) fn subscribe(&self) -> watch::Receiver<State> {
        self.state.subscribe()
    }
    pub(crate) fn choose(&self, human_only: bool) {
        self.state.send_modify(|state| {
            if !state.ready {
                return;
            }
            state.human_only = human_only;
            state.generation += 1;
            state.persisted = !self.disk;
            state.error = None;
        });
    }
    pub(crate) fn start(services: &crate::ui::Services, path: PathBuf) -> Self {
        let state = watch::channel(State {
            ready: false,
            ..State::default()
        })
        .0;
        let control = Self {
            state: state.clone(),
            disk: true,
        };
        let account = services.user_id.clone();
        let job = services.runtime.spawn(coordinate(path, account, state));
        services.track(&job);
        control
    }
}
fn load(path: &Path) -> Result<BTreeMap<String, bool>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_| "Could not read Only mine preference".into())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(_) => Err("Could not read Only mine preference".into()),
    }
}
fn save(
    path: &Path,
    account: &str,
    choice: &State,
    state: &watch::Sender<State>,
) -> Result<(), String> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut accounts = load(path)?;
    accounts.insert(account.into(), choice.human_only);
    let bytes = serde_json::to_vec(&accounts).map_err(|_| "Could not save Only mine preference")?;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|_| "Could not save Only mine preference")?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "Could not save Only mine preference")?;
    // Serialize only the final rename with a choice change, never file reads/writes/fsync.
    // A superseded write cannot become the durable choice after a newer activation.
    let latest = state.borrow();
    if latest.generation == choice.generation {
        std::fs::rename(&tmp, path).map_err(|_| "Could not save Only mine preference")?;
    } else {
        let _removed = std::fs::remove_file(&tmp);
    }
    Ok(())
}
async fn coordinate(path: PathBuf, account: String, state: watch::Sender<State>) {
    let mut receiver = state.subscribe();
    let read_path = path.clone();
    let loaded = tokio::task::spawn_blocking(move || load(&read_path)).await;
    state.send_modify(|state| {
        match loaded {
            Ok(Ok(accounts)) => state.human_only = accounts.get(&account).copied().unwrap_or(false),
            _ => {
                state.error = Some(
                    "Could not read Only mine preference. Fix storage access and retry.".into(),
                );
                state.persisted = false;
            }
        }
        state.ready = true;
    });
    let mut handled = 0;
    loop {
        let choice = receiver.borrow_and_update().clone();
        if choice.generation == handled {
            if receiver.changed().await.is_err() {
                break;
            }
            continue;
        }
        handled = choice.generation;
        let write_path = path.clone();
        let user = account.clone();
        let latest = state.clone();
        let saving = choice.clone();
        let result =
            tokio::task::spawn_blocking(move || save(&write_path, &user, &saving, &latest)).await;
        state.send_if_modified(|current| {
            if current.generation != choice.generation {
                return false;
            }
            match result {
                Ok(Ok(())) => {
                    current.persisted = true;
                    current.error = None;
                }
                _ => {
                    current.error =
                        Some("Only mine was not saved. Fix storage access and retry.".into())
                }
            }
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn until(control: &Control, predicate: impl Fn(&State) -> bool + Send + Sync) {
        let mut receiver = control.subscribe();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if predicate(&receiver.borrow_and_update()) {
                    break;
                }
                receiver.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
    #[test]
    fn superseded_save_never_replaces_latest_choice_or_other_accounts() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.json");
        std::fs::write(&path, r#"{"other":true,"owner":false}"#).unwrap();
        let control = Control::default();
        control.choose(true);
        let stale = control.state();
        control.choose(false);
        save(&path, "owner", &stale, &control.state).unwrap();
        assert!(!load(&path).unwrap()["owner"]);
        assert!(load(&path).unwrap()["other"]);
        control.choose(true);
        save(&path, "owner", &control.state(), &control.state).unwrap();
        assert!(load(&path).unwrap()["owner"]);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[tokio::test]
    async fn default_latest_choice_restart_failure_retry_and_account_isolation() {
        let root = tempfile::tempdir().unwrap();
        let host =
            flicknote_sync::spike::SpikeHost::start(root.path(), 0, 0, std::time::Duration::ZERO)
                .await
                .unwrap();
        let host = crate::launch::Host::Synthetic(host);
        let services = host.services(tokio::runtime::Handle::current());
        let path = root.path().join("source.json");
        let organization = root.path().join("organization.json");
        std::fs::write(
            &organization,
            r#"{"owner":{"cutoff":"2026-01-01","enabled":true}}"#,
        )
        .unwrap();
        let original = std::fs::read(&organization).unwrap();
        let control = Control::start(&services, path.clone());
        until(&control, |s| s.ready).await;
        assert!(!control.state().human_only);
        for index in 0..20 {
            control.choose(index % 2 == 0);
        }
        control.choose(true);
        until(&control, |s| {
            s.persisted && s.human_only && s.generation == 21
        })
        .await;
        assert!(load(&path).unwrap()[&services.user_id]);
        assert_eq!(std::fs::read(organization).unwrap(), original);
        let restart = Control::start(&services, path.clone());
        until(&restart, |s| s.ready).await;
        assert!(restart.state().human_only);
        // The loaded map retains another account when the active account changes its choice.
        let mut accounts = load(&path).unwrap();
        accounts.insert("another".into(), true);
        std::fs::write(&path, serde_json::to_vec(&accounts).unwrap()).unwrap();
        let blocked = path.with_extension("json.tmp");
        std::fs::create_dir(&blocked).unwrap();
        restart.choose(false);
        until(&restart, |s| s.error.is_some()).await;
        assert!(!restart.state().human_only);
        assert!(!restart.state().persisted);
        assert!(
            load(&path).unwrap()[&services.user_id],
            "failed save must leave last durable value"
        );
        std::fs::remove_dir(blocked).unwrap();
        restart.choose(false);
        until(&restart, |s| s.persisted).await;
        assert!(!load(&path).unwrap()[&services.user_id]);
        assert!(load(&path).unwrap()["another"]);
        services.cancel_operations();
        host.run_until(async { Ok(()) }, || {}).await.unwrap();
    }
    #[tokio::test]
    async fn unreadable_preference_is_truthful_and_does_not_overwrite_other_state() {
        let root = tempfile::tempdir().unwrap();
        let host =
            flicknote_sync::spike::SpikeHost::start(root.path(), 0, 0, std::time::Duration::ZERO)
                .await
                .unwrap();
        let host = crate::launch::Host::Synthetic(host);
        let services = host.services(tokio::runtime::Handle::current());
        let path = root.path().join("broken.json");
        std::fs::write(&path, "broken").unwrap();
        let control = Control::start(&services, path.clone());
        until(&control, |s| s.ready).await;
        assert!(control.state().error.is_some());
        assert!(!control.state().persisted);
        control.choose(true);
        until(&control, |s| s.error.is_some()).await;
        assert!(control.state().human_only);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
        services.cancel_operations();
        host.run_until(async { Ok(()) }, || {}).await.unwrap();
    }
}
