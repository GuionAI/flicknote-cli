//! Isolated normal Config/port resolution, without a native window or live state.
use super::*;
use flicknote_auth::{
    client::{AuthSession, AuthUser},
    session::save_session,
};
use flicknote_core::config::Config;

#[test]
fn normal_launch_in_isolated_xdg_subprocess() {
    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "launch::normal_host_tests::isolated_normal_host",
            "--ignored",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("FLICKNOTE_MCP_PORT", "0")
        .env("FLICKNOTE_SUPABASE_KEY", "owned-override")
        .env("FN_OWNED_NORMAL_TEST", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "runs only inside the test-owned XDG subprocess"]
async fn isolated_normal_host() {
    assert_eq!(std::env::var("FN_OWNED_NORMAL_TEST").unwrap(), "1");
    tokio::time::timeout(Duration::from_secs(30), normal_host_fixture())
        .await
        .unwrap();
}

async fn owned_config() -> (Config, tokio::task::JoinHandle<()>) {
    let config_home = PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap());
    let data_home = PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap());
    let initial = Config::load().unwrap();
    assert_eq!(initial.paths.config_dir, config_home.join("flicknote"));
    assert_eq!(initial.paths.data_dir, data_home.join("flicknote"));
    assert_eq!(initial.supabase_url, "https://dev-auth.flicknote.app");
    assert_eq!(initial.supabase_anon_key, "owned-override");
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router =
        axum::Router::new().fallback(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE });
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    std::fs::write(
        &initial.paths.config_file,
        serde_json::to_vec(&serde_json::json!({
            "supabaseUrl":origin, "supabaseAnonKey":"saved-key", "powersyncUrl":origin,
            "apiUrl":origin, "gatewayUrl":origin
        }))
        .unwrap(),
    )
    .unwrap();
    let config = Config::load().unwrap();
    assert_eq!(config.supabase_url, origin);
    assert_eq!(config.powersync_url, origin);
    assert_eq!(config.api_url, origin);
    assert_eq!(config.gateway_url, origin);
    assert_eq!(config.supabase_anon_key, "owned-override");
    (config, server)
}

async fn reject_incumbent_and_cancel_login(config: &Config) {
    let options = Options::try_parse_from(["flicknote-gpui"]).unwrap();
    let (quit, cancel) = tokio::sync::watch::channel(false);
    let incumbent =
        flicknote_sync::ownership::DataDirectoryLock::acquire(&config.paths.data_dir).unwrap();
    let socket = config.paths.data_dir.join("daemon.sock");
    let incumbent_socket = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let inode = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&socket).unwrap());
    let error = options
        .start(cancel.clone(), |_| panic!("incumbent login"))
        .await
        .err()
        .unwrap();
    assert!(error.contains("Quit the current host"));
    assert!(!config.paths.session_file.exists());
    assert!(!config.paths.log_file.exists());
    assert_eq!(
        std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&socket).unwrap()),
        inode
    );
    drop(incumbent_socket);
    std::fs::remove_file(&socket).unwrap();
    drop(incumbent);
    // The no-session pane is offered only while ownership is held. Cancel before any OTP.
    let (shown, pane) = tokio::sync::oneshot::channel();
    let startup = tokio::spawn(async move {
        options
            .start(cancel, move |handle| {
                assert!(shown.send(handle).is_ok());
            })
            .await
    });
    let _pane = pane.await.unwrap();
    assert!(flicknote_sync::ownership::DataDirectoryLock::acquire(&config.paths.data_dir).is_err());
    quit.send_replace(true);
    assert!(startup.await.unwrap().is_err());
    assert!(!config.paths.session_file.exists());
    assert!(!socket.exists());
}

async fn normal_host_fixture() {
    let (config, server) = owned_config().await;
    reject_incumbent_and_cancel_login(&config).await;
    let socket = config.paths.data_dir.join("daemon.sock");
    save_session(
        &config.paths.session_file,
        &AuthSession {
            access_token: "owned-access".into(),
            refresh_token: "owned-refresh".into(),
            expires_at: Some(u64::MAX),
            user: AuthUser {
                id: "owned-normal-user".into(),
                email: None,
            },
        },
    )
    .unwrap();
    let session = std::fs::read(&config.paths.session_file).unwrap();
    let options = Options::try_parse_from(["flicknote-gpui"]).unwrap();
    let (_quit, cancel) = tokio::sync::watch::channel(false);
    let host = options
        .start(cancel.clone(), |_| panic!("cached session login"))
        .await
        .unwrap();
    let Host::Real(ref real) = host else {
        panic!("synthetic fallback")
    };
    assert_eq!(real.socket, socket);
    assert_ne!(real.mcp_port, 0);
    assert_ne!(real.mcp_port, 37789);
    assert!(
        flicknote_client::DaemonClient::new(&socket)
            .health()
            .await
            .is_ok()
    );
    assert_eq!(std::fs::read(&config.paths.session_file).unwrap(), session);
    host.run_until(async { Ok(()) }, || {}).await.unwrap();
    assert!(!socket.exists());
    // A genuine occupied MCP port removes only this startup's IPC socket and releases its lock.
    let occupied = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = occupied.local_addr().unwrap().port();
    let error = options
        .start_normal(config.clone(), Some(port), cancel, |_| {
            panic!("cached login")
        })
        .await
        .err()
        .unwrap();
    assert!(error.contains("Address already in use"), "{error}");
    assert!(!socket.exists());
    assert_eq!(std::fs::read(&config.paths.session_file).unwrap(), session);
    assert!(flicknote_sync::ownership::DataDirectoryLock::acquire(&config.paths.data_dir).is_ok());
    assert!(
        tokio::net::TcpStream::connect(occupied.local_addr().unwrap())
            .await
            .is_ok()
    );
    server.abort();
}
