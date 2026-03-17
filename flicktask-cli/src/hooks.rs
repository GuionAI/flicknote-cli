//! Taskwarrior-compatible hook execution for flicktask.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// Returns the hooks directory: `~/.config/flicktask/hooks/`.
pub fn hooks_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("flicktask").join("hooks"))
}

/// Run all `on-add-*` hooks. Returns (possibly modified) task JSON.
pub fn run_on_add(task_json: &Value) -> Result<Value> {
    match hooks_dir() {
        Some(dir) => run_on_add_with_dir(task_json, &dir),
        None => Ok(task_json.clone()),
    }
}

/// Run all `on-modify-*` hooks. Returns (possibly modified) task JSON.
pub fn run_on_modify(original: &Value, modified: &Value) -> Result<Value> {
    match hooks_dir() {
        Some(dir) => run_on_modify_with_dir(original, modified, &dir),
        None => Ok(modified.clone()),
    }
}

pub(crate) fn run_on_add_with_dir(task_json: &Value, hooks_dir: &Path) -> Result<Value> {
    let mut current = task_json.clone();
    for hook in discover_hooks(hooks_dir, "on-add-") {
        current = run_hook_single(&hook, &current)?;
    }
    Ok(current)
}

pub(crate) fn run_on_modify_with_dir(
    original: &Value,
    modified: &Value,
    hooks_dir: &Path,
) -> Result<Value> {
    let mut current = modified.clone();
    for hook in discover_hooks(hooks_dir, "on-modify-") {
        current = run_hook_modify(&hook, original, &current)?;
    }
    Ok(current)
}

/// Discover executable files with `prefix` in `dir`, sorted alphabetically.
fn discover_hooks(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut hooks: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            name_str.starts_with(prefix) && is_executable(&e.path())
        })
        .map(|e| e.path())
        .collect();
    hooks.sort();
    hooks
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(m) => m.permissions().mode() & 0o111 != 0,
        Err(err) => {
            eprintln!("flicktask: warning: could not check hook {:?}: {err}", path);
            false
        }
    }
}

fn run_hook_single(hook: &Path, task_json: &Value) -> Result<Value> {
    let input = serde_json::to_string(task_json)?;
    // Don't wrap with context — hook rejection stderr should be the top-level error message
    let output = run_hook_process(hook, &input)?;
    parse_hook_output(hook, &output)
}

fn run_hook_modify(hook: &Path, original: &Value, modified: &Value) -> Result<Value> {
    let input = format!(
        "{}\n{}",
        serde_json::to_string(original)?,
        serde_json::to_string(modified)?
    );
    // Don't wrap with context — hook rejection stderr should be the top-level error message
    let output = run_hook_process(hook, &input)?;
    parse_hook_output(hook, &output)
}

fn parse_hook_output(hook: &Path, output: &str) -> Result<Value> {
    if output.is_empty() {
        anyhow::bail!(
            "Hook {} produced no output — hooks must print the task JSON to stdout",
            hook.display()
        );
    }
    serde_json::from_str(output)
        .with_context(|| format!("Hook {} produced invalid JSON: {}", hook.display(), output))
}

fn run_hook_process(hook: &Path, input: &str) -> Result<String> {
    let mut child = Command::new(hook)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn hook: {}", hook.display()))?;

    {
        let mut stdin = child.stdin.take().unwrap();
        // Ignore write errors — BrokenPipe means the hook rejected early without
        // consuming stdin. Any real failure will surface via the non-zero exit status.
        stdin.write_all(input.as_bytes()).ok();
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim();
        if msg.is_empty() {
            bail!(
                "Hook {} exited with status {}",
                hook.display(),
                output.status
            );
        } else {
            bail!("{}", msg);
        }
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_hook(dir: &std::path::Path, name: &str, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, script).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }

    #[test]
    fn test_no_hooks_dir_is_ok() {
        let tmp = TempDir::new().unwrap();
        let nonexistent = tmp.path().join("nope");
        let task_json = serde_json::json!({"uuid": "abc", "status": "pending", "description": "t"});
        let result = run_on_add_with_dir(&task_json, &nonexistent).unwrap();
        assert_eq!(result["uuid"], "abc");
    }

    #[test]
    fn test_on_add_hook_passthrough() {
        let tmp = TempDir::new().unwrap();
        write_hook(tmp.path(), "on-add-ttal", "#!/bin/bash\ncat\n");
        let task_json = serde_json::json!({"uuid": "abc", "status": "pending", "description": "t"});
        let result = run_on_add_with_dir(&task_json, tmp.path()).unwrap();
        assert_eq!(result["uuid"], "abc");
    }

    #[test]
    fn test_on_add_hook_enriches() {
        let tmp = TempDir::new().unwrap();
        write_hook(
            tmp.path(),
            "on-add-enrich",
            "#!/bin/bash\npython3 -c \"import sys,json; t=json.load(sys.stdin); t['branch']='worker/enriched'; print(json.dumps(t))\"\n",
        );
        let task_json = serde_json::json!({"uuid": "abc", "status": "pending", "description": "t"});
        let result = run_on_add_with_dir(&task_json, tmp.path()).unwrap();
        assert_eq!(result["branch"], "worker/enriched");
    }

    #[test]
    fn test_on_add_hook_reject() {
        let tmp = TempDir::new().unwrap();
        write_hook(
            tmp.path(),
            "on-add-reject",
            "#!/bin/bash\necho 'bad project' >&2\nexit 1\n",
        );
        let task_json = serde_json::json!({"uuid": "abc", "status": "pending", "description": "t"});
        let err = run_on_add_with_dir(&task_json, tmp.path()).unwrap_err();
        assert!(err.to_string().contains("bad project"));
    }

    #[test]
    fn test_on_modify_hook_passthrough() {
        let tmp = TempDir::new().unwrap();
        write_hook(tmp.path(), "on-modify-ttal", "#!/bin/bash\ntail -n 1\n");
        let orig = serde_json::json!({"uuid": "abc", "status": "pending", "description": "old"});
        let modified =
            serde_json::json!({"uuid": "abc", "status": "completed", "description": "old"});
        let result = run_on_modify_with_dir(&orig, &modified, tmp.path()).unwrap();
        assert_eq!(result["status"], "completed");
    }

    #[test]
    fn test_hooks_run_alphabetically() {
        let tmp = TempDir::new().unwrap();
        write_hook(
            tmp.path(),
            "on-add-a-first",
            "#!/bin/bash\npython3 -c \"import sys,json; t=json.load(sys.stdin); t['branch']='worker/a'; print(json.dumps(t))\"\n",
        );
        write_hook(
            tmp.path(),
            "on-add-z-last",
            "#!/bin/bash\npython3 -c \"import sys,json; t=json.load(sys.stdin); t['branch']='worker/z'; print(json.dumps(t))\"\n",
        );
        let task_json = serde_json::json!({"uuid": "abc", "status": "pending", "description": "t"});
        let result = run_on_add_with_dir(&task_json, tmp.path()).unwrap();
        assert_eq!(result["branch"], "worker/z");
    }
}
