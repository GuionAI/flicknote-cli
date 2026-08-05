use std::io::Write;
use std::process::{Command, Stdio};

fn write_session(config_root: &std::path::Path) {
    let directory = config_root.join("flicknote");
    std::fs::create_dir_all(&directory).unwrap();
    let session = serde_json::json!({
        "access_token": "test-token",
        "refresh_token": "test-refresh",
        "expires_at": null,
        "user": { "id": "test-user", "email": null }
    });
    let wrapper = serde_json::json!({
        "sb-test-auth-token": serde_json::to_string(&session).unwrap()
    });
    std::fs::write(
        directory.join("session.json"),
        serde_json::to_vec(&wrapper).unwrap(),
    )
    .unwrap();
}

#[test]
fn mcp_binary_keeps_stdout_as_json_rpc_frames() {
    let directory = tempfile::tempdir().unwrap();
    let config_root = directory.path().join("config");
    let data_root = directory.path().join("data");
    write_session(&config_root);

    let mut child = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .arg("mcp")
        .env("XDG_CONFIG_HOME", &config_root)
        .env("XDG_DATA_HOME", &data_root)
        .env_remove("DATABASE_URL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-11-25","capabilities":{{}},"clientInfo":{{"name":"integration-test","version":"0"}}}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#
    )
    .unwrap();
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let frames = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["id"], 1);
    assert_eq!(frames[1]["id"], 2);
    assert_eq!(frames[0]["result"]["serverInfo"]["name"], "flicknote");
    assert!(frames[1]["result"]["tools"].as_array().unwrap().len() > 20);
}

#[test]
fn managed_workspace_rejects_mcp_before_protocol_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_flicknote"))
        .arg("mcp")
        .env("DATABASE_URL", "postgres://unused")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("`flicknote mcp` is not available in managed workspaces")
    );
}
