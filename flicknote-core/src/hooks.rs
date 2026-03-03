use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::CliError;

/// Run the on-modify hook if it exists (notification only).
///
/// Looks for an executable file at `<hooks_dir>/on-modify`.
/// If not found or not executable, returns Ok silently.
///
/// Hook contract:
/// - stdin: 2 lines — old note JSON, new note JSON
/// - args: `api:1 command:<cmd> config:<config_dir>`
/// - stdout: feedback lines (used as error message on failure, ignored on success)
/// - exit 0 = accept, non-0 = reject (aborts the modification)
/// - notification only — hook cannot modify the note
pub fn run_on_modify(
    hooks_dir: &Path,
    old_json: &str,
    new_json: &str,
    command: &str,
    config_dir: &str,
) -> Result<(), CliError> {
    let hook_path = hooks_dir.join("on-modify");

    if !hook_path.exists() || !is_executable(&hook_path) {
        return Ok(());
    }

    let mut child = Command::new(&hook_path)
        .args([
            "api:1",
            &format!("command:{command}"),
            &format!("config:{config_dir}"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            CliError::Other(format!(
                "Failed to execute hook {}: {e}",
                hook_path.display()
            ))
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        writeln!(stdin, "{old_json}")?;
        writeln!(stdin, "{new_json}")?;
    }

    let output = child.wait_with_output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let feedback: Vec<String> = stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    if !output.status.success() {
        let message = if feedback.is_empty() {
            format!("Hook 'on-modify' exited with status {}", output.status)
        } else {
            format!("Hook 'on-modify': {}", feedback.join("; "))
        };
        return Err(CliError::HookRejected { message });
    }

    Ok(())
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
        fs::write(&hook, "#!/bin/sh\ncat > /dev/null\nexit 0\n").unwrap();
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
}
