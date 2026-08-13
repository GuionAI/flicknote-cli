#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use flicknote_sync::ipc::{DaemonRequest, DaemonResponse, PROTOCOL_VERSION};
use serde_json::json;
use tempfile::TempDir;

struct DaemonProcess {
    _directory: TempDir,
    config_home: PathBuf,
    data_home: PathBuf,
    child: Child,
}

struct DaemonExit {
    status: std::process::ExitStatus,
    logs: String,
}

impl DaemonExit {
    fn success(&self) -> bool {
        self.status.success()
    }
}

impl DaemonProcess {
    fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let config_home = directory.path().join("config");
        let data_home = directory.path().join("data");
        let config_dir = config_home.join("flicknote");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(data_home.join("flicknote")).unwrap();
        let session = json!({
            "sb-test-auth-token": serde_json::to_string(&json!({
                "access_token": "test-token",
                "refresh_token": "test-refresh",
                "expires_at": 4102444800_u64,
                "user": { "id": "daemon-process-test-user", "email": null }
            })).unwrap()
        });
        std::fs::write(
            config_dir.join("session.json"),
            serde_json::to_vec(&session).unwrap(),
        )
        .unwrap();

        let child = Command::new(env!("CARGO_BIN_EXE_flicknote"))
            .args(["daemon", "run"])
            .env("XDG_CONFIG_HOME", &config_home)
            .env("XDG_DATA_HOME", &data_home)
            .env("FLICKNOTE_ENV", "dev")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            _directory: directory,
            config_home,
            data_home,
            child,
        }
    }

    fn socket(&self) -> PathBuf {
        socket_path_for(&self.data_home)
    }

    fn wait_ready(&mut self) {
        for _ in 0..200 {
            if self.health() {
                return;
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("daemon exited before readiness: {status}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("daemon did not become IPC-ready");
    }

    fn health(&self) -> bool {
        let Ok(mut stream) = UnixStream::connect(self.socket()) else {
            return false;
        };
        let request = serde_json::to_vec(&DaemonRequest::Health {
            protocol: PROTOCOL_VERSION,
        })
        .unwrap();
        if stream.write_all(&request).is_err()
            || stream.shutdown(std::net::Shutdown::Write).is_err()
        {
            return false;
        }
        let mut response = Vec::new();
        if stream.read_to_end(&mut response).is_err() {
            return false;
        }
        matches!(
            serde_json::from_slice::<DaemonResponse>(&response),
            Ok(DaemonResponse::ServerInfo(info)) if info.protocol == PROTOCOL_VERSION
        )
    }

    #[allow(unsafe_code)]
    fn signal(&mut self, signal: libc::c_int) -> DaemonExit {
        let result = unsafe { libc::kill(self.child.id() as libc::pid_t, signal) };
        assert_eq!(result, 0, "failed to signal daemon process");
        let status = wait_for_exit(&mut self.child, Duration::from_secs(12))
            .unwrap_or_else(|| panic!("daemon did not exit within the shutdown budget"));
        let mut logs = String::new();
        if let Some(mut stderr) = self.child.stderr.take() {
            stderr.read_to_string(&mut logs).unwrap();
        }
        DaemonExit { status, logs }
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            #[allow(unsafe_code)]
            unsafe {
                libc::kill(self.child.id() as libc::pid_t, libc::SIGKILL);
            }
            #[allow(clippy::let_underscore_untyped)]
            let _ = wait_for_exit(&mut self.child, Duration::from_secs(2));
        }
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn socket_path_for(data_home: &std::path::Path) -> PathBuf {
    data_home.join("flicknote").join("daemon.sock")
}

fn start_with_roots(process: &DaemonProcess) -> DaemonProcess {
    let child = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args(["daemon", "run"])
        .env("XDG_CONFIG_HOME", &process.config_home)
        .env("XDG_DATA_HOME", &process.data_home)
        .env("FLICKNOTE_ENV", "dev")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    DaemonProcess {
        _directory: tempfile::tempdir().unwrap(),
        config_home: process.config_home.clone(),
        data_home: process.data_home.clone(),
        child,
    }
}

#[test]
fn foreground_run_rejects_missing_auth_before_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let config_home = directory.path().join("config");
    let data_home = directory.path().join("data");
    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args(["daemon", "run"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_DATA_HOME", &data_home)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Not authenticated"));
    assert!(!data_home.join("flicknote").join("daemon.lock").exists());
}

#[test]
fn managed_foreground_permanent_auth_failure_exits_without_restarting() {
    let directory = tempfile::tempdir().unwrap();
    let config_home = directory.path().join("config");
    let data_home = directory.path().join("data");
    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args(["daemon", "run"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_DATA_HOME", &data_home)
        .env("FLICKNOTE_DAEMON_MANAGED", "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!data_home.join("flicknote").join("daemon.lock").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn managed_daemon_redirects_main_error_output_to_its_log() {
    let directory = tempfile::tempdir().unwrap();
    let config_home = directory.path().join("config");
    let data_home = directory.path().join("data");
    let config_dir = config_home.join("flicknote");
    let data_dir = data_home.join("flicknote");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    let session = json!({
        "sb-test-auth-token": serde_json::to_string(&json!({
            "access_token": "test-token",
            "refresh_token": "test-refresh",
            "expires_at": 4102444800_u64,
            "user": { "id": "daemon-log-test-user", "email": null }
        })).unwrap()
    });
    std::fs::write(
        config_dir.join("session.json"),
        serde_json::to_vec(&session).unwrap(),
    )
    .unwrap();
    std::fs::create_dir(data_dir.join("flicknote.db")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args(["daemon", "run"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_DATA_HOME", &data_home)
        .env("FLICKNOTE_ENV", "dev")
        .env("FLICKNOTE_DAEMON_MANAGED", "1")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let logs = std::fs::read_to_string(data_dir.join("flicknote.log")).unwrap();
    assert!(logs.contains("Error: FlickNote daemon failed"));
}

#[test]
fn second_foreground_daemon_cannot_take_ownership_or_remove_the_socket() {
    let mut first = DaemonProcess::start();
    first.wait_ready();
    let second = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args(["daemon", "run"])
        .env("XDG_CONFIG_HOME", &first.config_home)
        .env("XDG_DATA_HOME", &first.data_home)
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("already owns"));

    let managed = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .args(["daemon", "run"])
        .env("XDG_CONFIG_HOME", &first.config_home)
        .env("XDG_DATA_HOME", &first.data_home)
        .env("FLICKNOTE_DAEMON_MANAGED", "1")
        .output()
        .unwrap();
    assert!(
        !managed.status.success(),
        "a managed ownership conflict must request an OS restart"
    );
    let mut managed_diagnostics = String::from_utf8_lossy(&managed.stderr).into_owned();
    let managed_log = first.data_home.join("flicknote").join("flicknote.log");
    if managed_log.exists() {
        managed_diagnostics.push_str(&std::fs::read_to_string(managed_log).unwrap());
    }
    assert!(managed_diagnostics.contains("already owns"));

    assert!(first.socket().exists());
    assert!(first.signal(libc::SIGTERM).success());
    assert!(!first.socket().exists());
}

#[test]
fn both_signals_use_graceful_shutdown_and_allow_restart() {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let mut process = DaemonProcess::start();
        process.wait_ready();
        let exit = process.signal(signal);
        assert!(exit.success());
        assert!(
            exit.logs
                .contains("Shutdown stage: stop accepting and drain IPC")
        );
        assert!(exit.logs.contains("Shutdown stage: disconnect PowerSync"));
        assert!(exit.logs.contains("Shutdown stage: truncate WAL"));
        assert!(exit.logs.contains("Daemon shutdown coordinator finished"));
        assert!(!process.socket().exists());
        let mut restarted = start_with_roots(&process);
        restarted.wait_ready();
        assert!(restarted.signal(libc::SIGINT).success());
    }
}

#[test]
fn forced_termination_releases_lock_and_stale_socket_is_reclaimed() {
    let mut first = DaemonProcess::start();
    first.wait_ready();
    assert!(!first.signal(libc::SIGKILL).success());
    assert!(first.socket().exists());

    let mut restarted = start_with_roots(&first);
    restarted.wait_ready();
    assert!(restarted.signal(libc::SIGINT).success());
}

#[test]
fn different_data_directories_can_run_concurrently() {
    let mut first = DaemonProcess::start();
    let mut second = DaemonProcess::start();
    first.wait_ready();
    second.wait_ready();
    assert!(first.signal(libc::SIGINT).success());
    assert!(second.signal(libc::SIGINT).success());
}
