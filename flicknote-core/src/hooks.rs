//! # FlickNote Hook System
//!
//! Hooks are executable files in `<config_dir>/hooks/`. They fire on CLI lifecycle events.
//!
//! ## Available hooks
//!
//! | Hook | File | Stdin | Can reject? | Blocking? |
//! |------|------|-------|-------------|-----------|
//! | on-add | `hooks/on-add` | 1 line: new note JSON | Yes | Yes |
//! | on-modify | `hooks/on-modify` | 2 lines: old JSON, new JSON | Yes | Yes |
//! | on-archive | `hooks/on-archive` | 2 lines: old JSON, new JSON | Yes | Yes |
//! | on-get | `hooks/on-get` | 1 line: note JSON | No | Yes (ignores exit) |
//!
//! ## Common args
//!
//! All hooks receive: `api:1 command:<cmd> config:<config_dir>`
//!
//! ## Exit codes
//!
//! - `0` = accept (for rejectable hooks: proceed with operation)
//! - non-0 = reject (for rejectable hooks: abort with HookRejected error)
//! - on-get ignores exit codes
//!
//! ## Hook files
//!
//! Must be executable. Non-existent or non-executable files are silently skipped.
//!
//! ## ⚠ Blocking behaviour
//!
//! All hooks run synchronously: the CLI blocks until the hook process exits. Hook scripts
//! that fork background processes without closing inherited file descriptors (stdout/stderr)
//! will cause `wait_with_output` to hang indefinitely — there is currently no timeout or
//! kill mechanism. Keep hooks short-lived; redirect or close file descriptors in scripts
//! that need to daemonise.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::CliError;

/// Internal: run a hook by name with the given stdin and args.
/// If `check_exit` is true, non-zero exit returns HookRejected.
/// If `check_exit` is false, exit code is ignored (fire-and-forget).
fn run_hook(
    hooks_dir: &Path,
    hook_name: &str,
    stdin_data: &str,
    command: &str,
    config_dir: &str,
    check_exit: bool,
) -> Result<(), CliError> {
    let hook_path = hooks_dir.join(hook_name);

    if !hook_path.exists() || !is_executable(&hook_path) {
        return Ok(());
    }

    // Retry on ETXTBSY (os error 26): transient race between file write and exec
    // on some kernels/filesystems. Resolves within milliseconds.
    let mut child = {
        let mut last_err = None;
        let mut spawned = None;
        for attempt in 0u32..3 {
            match Command::new(&hook_path)
                .args([
                    "api:1",
                    &format!("command:{command}"),
                    &format!("config:{config_dir}"),
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(child) => {
                    spawned = Some(child);
                    break;
                }
                Err(e) if e.raw_os_error() == Some(26) && attempt < 2 => {
                    std::thread::sleep(std::time::Duration::from_millis(10 * (attempt as u64 + 1)));
                }
                Err(e) => {
                    last_err = Some(e);
                    break;
                }
            }
        }
        spawned.ok_or_else(|| {
            last_err
                .map(|e| {
                    CliError::Other(format!(
                        "Failed to execute hook {}: {e}",
                        hook_path.display()
                    ))
                })
                .unwrap_or_else(|| {
                    CliError::Other(format!("Failed to execute hook {}", hook_path.display()))
                })
        })?
    };

    if let Some(mut stdin) = child.stdin.take() {
        write!(stdin, "{stdin_data}")?;
    }

    let output = child.wait_with_output()?;

    if check_exit && !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let feedback: Vec<String> = stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        let message = if feedback.is_empty() {
            format!("Hook '{hook_name}' exited with status {}", output.status)
        } else {
            format!("Hook '{hook_name}': {}", feedback.join("; "))
        };
        return Err(CliError::HookRejected { message });
    }

    Ok(())
}

/// Runs the on-add hook when a new note is created.
///
/// # Hook contract
/// - **stdin**: 1 line — new note JSON
/// - **args**: `api:1 command:add config:<dir>`
/// - **exit 0**: accept, **non-0**: reject (aborts creation)
pub fn run_on_add(hooks_dir: &Path, new_json: &str, config_dir: &str) -> Result<(), CliError> {
    let stdin_data = format!("{new_json}\n");
    run_hook(hooks_dir, "on-add", &stdin_data, "add", config_dir, true)
}

/// Runs the on-get hook when a note is retrieved. Synchronous but
/// exit-code-ignoring: the hook runs to completion, but cannot reject
/// the read operation regardless of exit code. Infrastructure errors
/// (spawn failure, broken pipe) are printed as warnings rather than
/// silently discarded.
///
/// # Hook contract
/// - **stdin**: 1 line — note JSON
/// - **args**: `api:1 command:get config:<dir>`
/// - **exit code**: ignored (read-only hook, cannot reject)
/// - **scope**: fires only on full-note retrieval (`flicknote get <id>`);
///   structural queries (`--tree`, `--section`) do not fire this hook
///   because they do not fetch the full `Note` record.
#[allow(clippy::print_stderr)] // infrastructure warnings should surface to the user
pub fn run_on_get(hooks_dir: &Path, note_json: &str, config_dir: &str) {
    if let Err(e) = run_hook(
        hooks_dir,
        "on-get",
        &format!("{note_json}\n"),
        "get",
        config_dir,
        false,
    ) {
        match e {
            CliError::HookRejected { .. } => {} // intentionally ignored for on-get
            other => eprintln!("warning: on-get hook failed: {other}"),
        }
    }
}

/// Runs the on-archive hook when a note is archived or unarchived.
///
/// # Hook contract
/// - **stdin**: 2 lines — old note JSON, new note JSON
/// - **args**: `api:1 command:archive|unarchive config:<dir>`
/// - **exit 0**: accept, **non-0**: reject (aborts archive/unarchive)
///
/// Note: this is a separate hook file from on-modify because archive is a
/// lifecycle transition (soft-delete), not a content change. Agents may want
/// to subscribe to one but not the other.
pub fn run_on_archive(
    hooks_dir: &Path,
    old_json: &str,
    new_json: &str,
    command: &str,
    config_dir: &str,
) -> Result<(), CliError> {
    let stdin_data = format!("{old_json}\n{new_json}\n");
    run_hook(
        hooks_dir,
        "on-archive",
        &stdin_data,
        command,
        config_dir,
        true,
    )
}

/// Run the on-modify hook if it exists.
///
/// Hook contract:
/// - stdin: 2 lines — old note JSON, new note JSON
/// - args: `api:1 command:<cmd> config:<config_dir>`
/// - exit 0 = accept, non-0 = reject (aborts the modification)
pub fn run_on_modify(
    hooks_dir: &Path,
    old_json: &str,
    new_json: &str,
    command: &str,
    config_dir: &str,
) -> Result<(), CliError> {
    let stdin_data = format!("{old_json}\n{new_json}\n");
    run_hook(
        hooks_dir,
        "on-modify",
        &stdin_data,
        command,
        config_dir,
        true,
    )
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn temp_hooks_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write_hook(dir: &std::path::Path, name: &str, script: &str) {
        let hook = dir.join(name);
        fs::write(&hook, script).unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn no_hook_file_is_noop() {
        let dir = temp_hooks_dir();
        let old = r#"{"id":"abc","content":"old"}"#;
        let new = r#"{"id":"abc","content":"new"}"#;
        let result = run_on_modify(dir.path(), old, new, "edit", "/tmp");
        assert!(result.is_ok());
    }

    #[test]
    fn non_executable_hook_is_noop() {
        let dir = temp_hooks_dir();
        fs::write(dir.path().join("on-modify"), "#!/bin/sh\n").unwrap();

        let old = r#"{"id":"abc","content":"old"}"#;
        let new = r#"{"id":"abc","content":"new"}"#;
        let result = run_on_modify(dir.path(), old, new, "edit", "/tmp");
        assert!(result.is_ok());
    }

    #[test]
    fn hook_exit_0_succeeds() {
        let dir = temp_hooks_dir();
        let hook = dir.path().join("on-modify");
        let out = dir.path().join("out");
        let script = format!("#!/bin/sh\ncat > {out}\nexit 0\n", out = out.display());
        fs::write(&hook, script).unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

        let old = r#"{"id":"abc","content":"old"}"#;
        let new = r#"{"id":"abc","content":"new"}"#;
        let result = run_on_modify(dir.path(), old, new, "edit", "/tmp");
        assert!(result.is_ok());
    }

    #[test]
    fn hook_exit_nonzero_returns_error() {
        let dir = temp_hooks_dir();
        let hook = dir.path().join("on-modify");
        fs::write(
            &hook,
            "#!/bin/sh\ncat > /dev/null\necho 'Rejected by hook'\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

        let old = r#"{"id":"abc","content":"old"}"#;
        let new = r#"{"id":"abc","content":"new"}"#;
        let result = run_on_modify(dir.path(), old, new, "edit", "/tmp");
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CliError::HookRejected { message } => {
                assert!(message.contains("Rejected by hook"));
            }
            other => panic!("Expected HookRejected, got: {other}"),
        }
    }

    #[test]
    fn hook_receives_stdin_data() {
        let dir = temp_hooks_dir();
        let log_file = dir.path().join("hook.log");
        let hook = dir.path().join("on-modify");
        let script = format!("#!/bin/sh\ncat > {log}\n", log = log_file.display());
        fs::write(&hook, script).unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

        let old = r#"{"id":"abc","content":"old"}"#;
        let new = r#"{"id":"abc","content":"new"}"#;
        run_on_modify(dir.path(), old, new, "edit", "/tmp").unwrap();

        let log = fs::read_to_string(&log_file).unwrap();
        assert!(log.contains(r#""content":"old""#));
        assert!(log.contains(r#""content":"new""#));
    }

    // ── on-get tests ─────────────────────────────────────────────────────────

    #[test]
    fn on_get_no_hook_is_noop() {
        let dir = temp_hooks_dir();
        run_on_get(dir.path(), r#"{"id":"abc"}"#, "/tmp");
    }

    #[test]
    fn on_get_non_executable_is_noop() {
        let dir = temp_hooks_dir();
        fs::write(dir.path().join("on-get"), "#!/bin/sh\n").unwrap();
        run_on_get(dir.path(), r#"{"id":"abc"}"#, "/tmp");
    }

    #[test]
    fn on_get_exit_0_no_panic() {
        let dir = temp_hooks_dir();
        write_hook(dir.path(), "on-get", "#!/bin/sh\ncat > /dev/null\nexit 0\n");
        run_on_get(dir.path(), r#"{"id":"abc"}"#, "/tmp");
    }

    #[test]
    fn on_get_exit_nonzero_no_panic() {
        let dir = temp_hooks_dir();
        write_hook(dir.path(), "on-get", "#!/bin/sh\ncat > /dev/null\nexit 1\n");
        // Must not panic — exit code is ignored
        run_on_get(dir.path(), r#"{"id":"abc"}"#, "/tmp");
    }

    #[test]
    fn on_get_hook_executed_with_note_json() {
        let dir = temp_hooks_dir();
        let log_file = dir.path().join("get.log");
        let script = format!("#!/bin/sh\ncat > {log}\n", log = log_file.display());
        write_hook(dir.path(), "on-get", &script);

        run_on_get(dir.path(), r#"{"id":"abc","content":"test"}"#, "/tmp");

        let log = fs::read_to_string(&log_file).unwrap();
        assert!(log.contains(r#""content":"test""#));
    }

    #[test]
    fn on_get_config_dir_arg_passed() {
        let dir = temp_hooks_dir();
        let log_file = dir.path().join("get_args.log");
        let script = format!("#!/bin/sh\necho \"$@\" > {log}\n", log = log_file.display());
        write_hook(dir.path(), "on-get", &script);

        run_on_get(dir.path(), r#"{"id":"abc"}"#, "/my/config");

        let log = fs::read_to_string(&log_file).unwrap();
        assert!(log.contains("config:/my/config"));
    }

    // ── on-archive tests ─────────────────────────────────────────────────────

    #[test]
    fn on_archive_no_hook_is_noop() {
        let dir = temp_hooks_dir();
        let old = r#"{"id":"abc","deleted_at":null}"#;
        let new = r#"{"id":"abc","deleted_at":"2026-01-01T00:00:00Z"}"#;
        assert!(run_on_archive(dir.path(), old, new, "archive", "/tmp").is_ok());
    }

    #[test]
    fn on_archive_non_executable_is_noop() {
        let dir = temp_hooks_dir();
        fs::write(dir.path().join("on-archive"), "#!/bin/sh\n").unwrap();
        let old = r#"{"id":"abc","deleted_at":null}"#;
        let new = r#"{"id":"abc","deleted_at":"2026-01-01T00:00:00Z"}"#;
        assert!(run_on_archive(dir.path(), old, new, "archive", "/tmp").is_ok());
    }

    #[test]
    fn on_archive_exit_0_succeeds() {
        let dir = temp_hooks_dir();
        write_hook(
            dir.path(),
            "on-archive",
            "#!/bin/sh\ncat > /dev/null\nexit 0\n",
        );

        let old = r#"{"id":"abc","deleted_at":null}"#;
        let new = r#"{"id":"abc","deleted_at":"2026-01-01T00:00:00Z"}"#;
        assert!(run_on_archive(dir.path(), old, new, "archive", "/tmp").is_ok());
    }

    #[test]
    fn on_archive_exit_nonzero_returns_error() {
        let dir = temp_hooks_dir();
        write_hook(
            dir.path(),
            "on-archive",
            "#!/bin/sh\ncat > /dev/null\necho 'Blocked'\nexit 1\n",
        );

        let old = r#"{"id":"abc","deleted_at":null}"#;
        let new = r#"{"id":"abc","deleted_at":"2026-01-01T00:00:00Z"}"#;
        let result = run_on_archive(dir.path(), old, new, "archive", "/tmp");
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CliError::HookRejected { message } => {
                assert!(message.contains("Blocked"));
            }
            other => panic!("Expected HookRejected, got: {other}"),
        }
    }

    #[test]
    fn on_archive_receives_two_lines_of_json() {
        let dir = temp_hooks_dir();
        let log_file = dir.path().join("archive.log");
        let script = format!("#!/bin/sh\ncat > {log}\n", log = log_file.display());
        write_hook(dir.path(), "on-archive", &script);

        let old = r#"{"id":"abc","status":"active"}"#;
        let new = r#"{"id":"abc","status":"archived"}"#;
        run_on_archive(dir.path(), old, new, "archive", "/tmp").unwrap();

        let log = fs::read_to_string(&log_file).unwrap();
        let non_empty: Vec<&str> = log.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(non_empty.len(), 2);
        assert!(log.contains(r#""status":"active""#));
        assert!(log.contains(r#""status":"archived""#));
    }

    #[test]
    fn on_archive_unarchive_command_arg() {
        let dir = temp_hooks_dir();
        let log_file = dir.path().join("unarchive.log");
        let script = format!("#!/bin/sh\necho \"$@\" > {log}\n", log = log_file.display());
        write_hook(dir.path(), "on-archive", &script);

        let old = r#"{"id":"abc"}"#;
        let new = r#"{"id":"abc"}"#;
        run_on_archive(dir.path(), old, new, "unarchive", "/tmp").unwrap();

        let log = fs::read_to_string(&log_file).unwrap();
        assert!(log.contains("command:unarchive"));
    }

    // ── on-add tests ─────────────────────────────────────────────────────────

    #[test]
    fn on_add_no_hook_is_noop() {
        let dir = temp_hooks_dir();
        let new = r#"{"id":"abc","content":"hello"}"#;
        assert!(run_on_add(dir.path(), new, "/tmp").is_ok());
    }

    #[test]
    fn on_add_non_executable_is_noop() {
        let dir = temp_hooks_dir();
        fs::write(dir.path().join("on-add"), "#!/bin/sh\n").unwrap();
        let new = r#"{"id":"abc","content":"hello"}"#;
        assert!(run_on_add(dir.path(), new, "/tmp").is_ok());
    }

    #[test]
    fn on_add_exit_0_succeeds() {
        let dir = temp_hooks_dir();
        write_hook(dir.path(), "on-add", "#!/bin/sh\ncat > /dev/null\nexit 0\n");
        let new = r#"{"id":"abc","content":"hello"}"#;
        assert!(run_on_add(dir.path(), new, "/tmp").is_ok());
    }

    #[test]
    fn on_add_exit_nonzero_returns_error() {
        let dir = temp_hooks_dir();
        write_hook(
            dir.path(),
            "on-add",
            "#!/bin/sh\ncat > /dev/null\necho 'Rejected'\nexit 1\n",
        );

        let new = r#"{"id":"abc","content":"hello"}"#;
        let result = run_on_add(dir.path(), new, "/tmp");
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::CliError::HookRejected { message } => {
                assert!(message.contains("Rejected"));
            }
            other => panic!("Expected HookRejected, got: {other}"),
        }
    }

    #[test]
    fn on_add_receives_one_line_of_json() {
        let dir = temp_hooks_dir();
        let log_file = dir.path().join("add.log");
        let script = format!("#!/bin/sh\ncat > {log}\n", log = log_file.display());
        write_hook(dir.path(), "on-add", &script);

        let new = r#"{"id":"abc","content":"hello"}"#;
        run_on_add(dir.path(), new, "/tmp").unwrap();

        let log = fs::read_to_string(&log_file).unwrap();
        assert!(log.contains(r#""content":"hello""#));
        let non_empty: Vec<&str> = log.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(non_empty.len(), 1);
    }

    #[test]
    fn on_add_config_dir_arg_passed() {
        let dir = temp_hooks_dir();
        let log_file = dir.path().join("add_args.log");
        let script = format!("#!/bin/sh\necho \"$@\" > {log}\n", log = log_file.display());
        write_hook(dir.path(), "on-add", &script);

        run_on_add(dir.path(), r#"{"id":"abc"}"#, "/my/config").unwrap();

        let log = fs::read_to_string(&log_file).unwrap();
        assert!(log.contains("config:/my/config"));
    }
}
