use clap::{Args, Subcommand};
use flicknote_core::error::CliError;
use serde_json::{Map, Value, json};
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const HOOK_EVENT: &str = "UserPromptSubmit";
const MCP_HOOK_TYPE: &str = "mcp_tool";
const COMMAND_HOOK_TYPE: &str = "command";
const RECALL_TOOL: &str = "note_recall";
const RECALL_TIMEOUT_SECONDS: u64 = 1;
const RECALL_COMMAND_SUFFIX: &str = " recall --hook";

#[derive(Args)]
pub(crate) struct HookArgs {
    #[command(subcommand)]
    pub(crate) command: HookCommand,
}

#[derive(Subcommand)]
pub(crate) enum HookCommand {
    /// Install a host lifecycle hook
    Install(HookInstallArgs),
}

#[derive(Args)]
pub(crate) struct HookInstallArgs {
    #[command(subcommand)]
    command: HookInstallTarget,
}

#[derive(Subcommand)]
enum HookInstallTarget {
    /// Install the read-only recall hook for Codex
    Codex(CodexInstallArgs),
}

#[derive(Args, Default)]
struct CodexInstallArgs {
    /// Write the project-local Codex hooks file
    #[arg(long, conflicts_with = "global")]
    local: bool,
    /// Write the current user's Codex hooks file
    #[arg(long, conflicts_with = "local")]
    global: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Local,
    Global,
}

#[derive(Debug, Clone)]
struct InstallContext {
    current_dir: PathBuf,
    codex_home: PathBuf,
}

#[derive(Debug, Clone)]
struct InstallPaths {
    local_hooks: PathBuf,
    local_config: PathBuf,
    global_hooks: PathBuf,
    global_config: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InstallResult {
    Installed { path: PathBuf, updated: bool },
    AlreadyConfigured { locations: Vec<String> },
}

struct ConfigSource {
    path: PathBuf,
    document: Value,
}

#[derive(Debug, Clone, Copy)]
enum HookFormat {
    Json,
    Toml,
}

#[derive(Debug, Clone)]
struct HookMatch {
    location: String,
    path: Option<PathBuf>,
}

impl InstallContext {
    fn from_environment() -> Result<Self, CliError> {
        let current_dir = std::env::current_dir()?;
        let home_dir = dirs::home_dir()
            .ok_or_else(|| CliError::Other("could not determine the user home directory".into()))?;
        let codex_home = std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir.join(".codex"));
        Ok(Self {
            current_dir,
            codex_home,
        })
    }

    fn paths(&self) -> InstallPaths {
        let project_root = find_project_root(&self.current_dir);
        let local_codex_dir = project_root.join(".codex");
        InstallPaths {
            local_hooks: local_codex_dir.join("hooks.json"),
            local_config: local_codex_dir.join("config.toml"),
            global_hooks: self.codex_home.join("hooks.json"),
            global_config: self.codex_home.join("config.toml"),
        }
    }
}

pub(crate) fn run(args: &HookArgs) -> Result<(), CliError> {
    match &args.command {
        HookCommand::Install(args) => match &args.command {
            HookInstallTarget::Codex(args) => run_codex(args),
        },
    }
}

fn run_codex(args: &CodexInstallArgs) -> Result<(), CliError> {
    let context = InstallContext::from_environment()?;
    let paths = context.paths();
    let mut stdout = io::stdout().lock();
    let mut stdin = io::stdin().lock();
    let scope = requested_scope(
        args,
        &paths,
        &mut stdin,
        &mut stdout,
        io::stdin().is_terminal(),
    )?;
    let Some(scope) = scope else {
        writeln!(stdout, "Codex hook installation cancelled")?;
        return Ok(());
    };
    let result = install_codex(scope, &paths)?;
    print_result(result, scope, &mut stdout)
}

fn requested_scope(
    args: &CodexInstallArgs,
    paths: &InstallPaths,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    interactive: bool,
) -> Result<Option<Scope>, CliError> {
    if args.local {
        return Ok(Some(Scope::Local));
    }
    if args.global {
        return Ok(Some(Scope::Global));
    }
    if !interactive {
        return Err(CliError::Other(
            "choose a Codex hook scope with --local or --global when stdin is not interactive"
                .into(),
        ));
    }

    writeln!(output, "Choose where to install the Codex recall hook:")?;
    writeln!(output, "  1) local  {}", paths.local_hooks.display())?;
    writeln!(output, "  2) global {}", paths.global_hooks.display())?;
    write!(output, "Enter 1 or 2 (blank cancels): ")?;
    output.flush()?;
    let mut answer = String::new();
    if input.read_line(&mut answer)? == 0 || answer.trim().is_empty() {
        return Ok(None);
    }
    match answer.trim() {
        "1" | "local" => Ok(Some(Scope::Local)),
        "2" | "global" => Ok(Some(Scope::Global)),
        _ => Err(CliError::Other(
            "invalid scope; enter 1 for local or 2 for global".into(),
        )),
    }
}

fn install_codex(scope: Scope, paths: &InstallPaths) -> Result<InstallResult, CliError> {
    let executable = current_executable()?;
    install_codex_with_executable(scope, paths, &executable)
}

fn install_codex_with_executable(
    scope: Scope,
    paths: &InstallPaths,
    executable: &Path,
) -> Result<InstallResult, CliError> {
    let sources = read_config_sources(paths)?;
    reject_disabled_hooks(&sources)?;

    let local_json = read_hooks_json(&paths.local_hooks)?;
    let global_json = read_hooks_json(&paths.global_hooks)?;
    let mut matches = Vec::new();
    if let Some(root) = local_json.as_ref() {
        matches.extend(find_json_matches(root, &paths.local_hooks)?);
    }
    if let Some(root) = global_json.as_ref() {
        matches.extend(find_json_matches(root, &paths.global_hooks)?);
    }
    for source in &sources {
        matches.extend(find_inline_matches(&source.document, source)?);
    }

    let target_path = match scope {
        Scope::Local => &paths.local_hooks,
        Scope::Global => &paths.global_hooks,
    };
    let target_file_matches = matches
        .iter()
        .filter(|item| item.path.as_ref() == Some(target_path))
        .count();
    let other_matches = matches
        .iter()
        .filter(|item| item.path.as_ref() != Some(target_path))
        .map(|item| item.location.clone())
        .collect::<Vec<_>>();

    if !other_matches.is_empty() {
        return Ok(InstallResult::AlreadyConfigured {
            locations: matches.into_iter().map(|item| item.location).collect(),
        });
    }

    let mut root = match scope {
        Scope::Local => local_json.unwrap_or_else(|| json!({})),
        Scope::Global => global_json.unwrap_or_else(|| json!({})),
    };
    let original_root = root.clone();
    let updated = if target_file_matches > 0 {
        update_existing_json_matches(&mut root, executable)?;
        true
    } else {
        add_json_hook(&mut root, executable)?;
        false
    };

    let bytes = serde_json::to_vec_pretty(&root)?;
    let _: Value = serde_json::from_slice(&bytes)?;
    if root != original_root {
        atomic_write(target_path, &bytes)?;
    }
    Ok(InstallResult::Installed {
        path: target_path.clone(),
        updated,
    })
}

fn print_result(
    result: InstallResult,
    scope: Scope,
    output: &mut dyn Write,
) -> Result<(), CliError> {
    match result {
        InstallResult::Installed { path, updated } => {
            let action = if updated { "Updated" } else { "Installed" };
            writeln!(output, "{action} Codex recall hook in {}", path.display())?;
        }
        InstallResult::AlreadyConfigured { locations } => {
            writeln!(
                output,
                "Codex recall hook is already configured; no file was changed."
            )?;
            for location in locations {
                writeln!(output, "  {location}")?;
            }
        }
    }
    writeln!(
        output,
        "Installation uses the FlickNote CLI command and does not require an MCP registration or a running daemon."
    )?;
    writeln!(
        output,
        "Review and trust the hook in Codex with /hooks{}.",
        if scope == Scope::Local {
            "; project-local hooks also require a trusted project"
        } else {
            ""
        }
    )?;
    Ok(())
}

fn current_executable() -> Result<PathBuf, CliError> {
    let executable = std::env::current_exe()?;
    if executable.is_absolute() {
        return Ok(executable);
    }
    fs::canonicalize(executable).map_err(CliError::Io)
}

fn find_project_root(current_dir: &Path) -> PathBuf {
    let mut candidate = current_dir;
    loop {
        if candidate.join(".git").exists() {
            return candidate.to_path_buf();
        }
        let Some(parent) = candidate.parent() else {
            return current_dir.to_path_buf();
        };
        candidate = parent;
    }
}

fn read_config_sources(paths: &InstallPaths) -> Result<Vec<ConfigSource>, CliError> {
    let mut sources = Vec::new();
    for path in [&paths.global_config, &paths.local_config] {
        if !path.exists() {
            continue;
        }
        let raw = fs::read_to_string(path)?;
        let document = raw.parse::<toml::Value>().map_err(|error| {
            CliError::Other(format!(
                "invalid Codex TOML configuration {}: {error}",
                path.display()
            ))
        })?;
        sources.push(ConfigSource {
            path: path.to_path_buf(),
            document: toml_to_json(document),
        });
    }
    Ok(sources)
}

fn toml_to_json(value: toml::Value) -> Value {
    serde_json::to_value(value).expect("TOML values are JSON-compatible")
}

fn reject_disabled_hooks(sources: &[ConfigSource]) -> Result<(), CliError> {
    for source in sources {
        if source
            .document
            .get("allow_managed_hooks_only")
            .and_then(Value::as_bool)
            == Some(true)
        {
            return Err(CliError::Other(format!(
                "Codex config {} allows managed hooks only; review that policy before installing a user hook",
                source.path.display()
            )));
        }
        let Some(features) = source.document.get("features") else {
            continue;
        };
        let Some(features) = features.as_object() else {
            return Err(CliError::Other(format!(
                "invalid Codex features table in {}",
                source.path.display()
            )));
        };
        for key in ["hooks", "codex_hooks"] {
            if features.get(key).and_then(Value::as_bool) == Some(false) {
                return Err(CliError::Other(format!(
                    "Codex hooks are explicitly disabled by {} [features].{key}; no hook was written",
                    source.path.display()
                )));
            }
            if features.contains_key(key) && !features[key].is_boolean() {
                return Err(CliError::Other(format!(
                    "invalid Codex [features].{key} value in {}",
                    source.path.display()
                )));
            }
        }
    }
    Ok(())
}

fn read_hooks_json(path: &Path) -> Result<Option<Value>, CliError> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&raw).map_err(|error| {
        CliError::Other(format!(
            "invalid Codex hooks JSON {}: {error}",
            path.display()
        ))
    })?;
    if !value.is_object() {
        return Err(CliError::Other(format!(
            "Codex hooks file {} must have an object root",
            path.display()
        )));
    }
    Ok(Some(value))
}

fn find_json_matches(root: &Value, path: &Path) -> Result<Vec<HookMatch>, CliError> {
    find_hook_matches(
        root,
        &path.display().to_string(),
        Some(path),
        HookFormat::Json,
    )
}

fn find_inline_matches(
    document: &Value,
    source: &ConfigSource,
) -> Result<Vec<HookMatch>, CliError> {
    find_hook_matches(
        document,
        &source.path.display().to_string(),
        None,
        HookFormat::Toml,
    )
}

fn find_hook_matches(
    root: &Value,
    source_label: &str,
    path: Option<&Path>,
    format: HookFormat,
) -> Result<Vec<HookMatch>, CliError> {
    let Some(hooks) = root.get("hooks") else {
        return Ok(Vec::new());
    };
    let Some(hooks) = hooks.as_object() else {
        return Err(CliError::Other(format!(
            "Codex {} in {} must be an {}",
            hooks_name(format),
            source_label,
            object_name(format)
        )));
    };
    let Some(groups) = hooks.get(HOOK_EVENT) else {
        return Ok(Vec::new());
    };
    let Some(groups) = groups.as_array() else {
        return Err(CliError::Other(format!(
            "Codex {}.{HOOK_EVENT} in {} must be an array",
            hooks_name(format),
            source_label
        )));
    };
    let mut matches = Vec::new();
    for (group_index, group) in groups.iter().enumerate() {
        let Some(group) = group.as_object() else {
            return Err(CliError::Other(format!(
                "Codex {}.{HOOK_EVENT}[{group_index}] in {} must be an {}",
                hooks_name(format),
                source_label,
                object_name(format)
            )));
        };
        let Some(handlers) = group.get("hooks") else {
            return Err(CliError::Other(format!(
                "Codex {}.{HOOK_EVENT}[{group_index}] in {} has no hooks array",
                hooks_name(format),
                source_label
            )));
        };
        let Some(handlers) = handlers.as_array() else {
            return Err(CliError::Other(format!(
                "Codex {}.{HOOK_EVENT}[{group_index}].hooks in {} must be an array",
                hooks_name(format),
                source_label
            )));
        };
        for (handler_index, handler) in handlers.iter().enumerate() {
            let Some(handler) = handler.as_object() else {
                return Err(CliError::Other(format!(
                    "Codex hook handler in {} group {group_index} item {handler_index} must be an {}",
                    source_label,
                    object_name(format)
                )));
            };
            if matches_handler(handler) {
                if handler_disabled(handler) {
                    return Err(CliError::Other(format!(
                        "FlickNote recall hook at {} is explicitly disabled; no file was changed",
                        source_label
                    )));
                }
                matches.push(HookMatch {
                    location: format!(
                        "{}{} hooks.{HOOK_EVENT}[{group_index}].hooks[{handler_index}]",
                        source_label,
                        if matches!(format, HookFormat::Toml) {
                            " inline"
                        } else {
                            ""
                        }
                    ),
                    path: path.map(Path::to_path_buf),
                });
            }
        }
    }
    Ok(matches)
}

fn hooks_name(format: HookFormat) -> &'static str {
    match format {
        HookFormat::Json => "hooks",
        HookFormat::Toml => "[hooks]",
    }
}

fn object_name(format: HookFormat) -> &'static str {
    match format {
        HookFormat::Json => "object",
        HookFormat::Toml => "table",
    }
}

fn matches_handler(handler: &Map<String, Value>) -> bool {
    match handler.get("type").and_then(Value::as_str) {
        Some(MCP_HOOK_TYPE) => handler.get("tool").and_then(Value::as_str) == Some(RECALL_TOOL),
        Some(COMMAND_HOOK_TYPE) => handler
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_recall_command),
        _ => false,
    }
}

fn is_recall_command(command: &str) -> bool {
    let Some(executable) = command
        .strip_suffix(RECALL_COMMAND_SUFFIX)
        .map(str::trim_end)
    else {
        return false;
    };
    let Some(executable) = shell_unquote(executable) else {
        return false;
    };
    Path::new(&executable)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "flicknote" || name == "flicknote.exe")
}

fn shell_quote(path: &Path) -> Result<String, CliError> {
    let path = path.to_str().ok_or_else(|| {
        CliError::Other(format!(
            "current executable path is not valid UTF-8: {}",
            path.display()
        ))
    })?;
    Ok(format!("'{}'", path.replace('\'', "'\\''")))
}

fn shell_unquote(value: &str) -> Option<String> {
    if value.len() < 2 || !value.starts_with('\'') || !value.ends_with('\'') {
        return if value.chars().any(char::is_whitespace) {
            None
        } else {
            Some(value.to_string())
        };
    }
    let inner = &value[1..value.len() - 1];
    let escaped_quote = "'\\''";
    let mut remaining = inner;
    let mut decoded = String::with_capacity(inner.len());
    while let Some(index) = remaining.find(escaped_quote) {
        let prefix = &remaining[..index];
        if prefix.contains('\'') {
            return None;
        }
        decoded.push_str(prefix);
        decoded.push('\'');
        remaining = &remaining[index + escaped_quote.len()..];
    }
    if remaining.contains('\'') {
        return None;
    }
    decoded.push_str(remaining);
    Some(decoded)
}

fn handler_disabled(handler: &Map<String, Value>) -> bool {
    handler.get("enabled").and_then(Value::as_bool) == Some(false)
        || handler.get("disabled").and_then(Value::as_bool) == Some(true)
}

fn desired_handler(executable: &Path) -> Result<Value, CliError> {
    Ok(json!({
        "type": COMMAND_HOOK_TYPE,
        "command": format!("{}{}", shell_quote(executable)?, RECALL_COMMAND_SUFFIX),
        "timeout": RECALL_TIMEOUT_SECONDS,
    }))
}

fn add_json_hook(root: &mut Value, executable: &Path) -> Result<(), CliError> {
    let object = root_object_mut(root)?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks.as_object_mut().ok_or_else(|| {
        CliError::Other("Codex hooks must be an object; no file was changed".into())
    })?;
    let events = hooks
        .entry(HOOK_EVENT)
        .or_insert_with(|| Value::Array(Vec::new()));
    let events = events.as_array_mut().ok_or_else(|| {
        CliError::Other(format!(
            "Codex hooks.{HOOK_EVENT} must be an array; no file was changed"
        ))
    })?;
    events.push(json!({ "hooks": [desired_handler(executable)?] }));
    Ok(())
}

fn update_existing_json_matches(root: &mut Value, executable: &Path) -> Result<(), CliError> {
    let object = root_object_mut(root)?;
    let hooks = object
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            CliError::Other("Codex hooks must be an object; no file was changed".into())
        })?;
    let events = hooks
        .get_mut(HOOK_EVENT)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            CliError::Other(format!(
                "Codex hooks.{HOOK_EVENT} must be an array; no file was changed"
            ))
        })?;
    let desired = desired_handler(executable)?;
    let mut replaced = false;
    for group in events {
        let Some(group) = group.as_object_mut() else {
            continue;
        };
        let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        let mut index = 0;
        while index < handlers.len() {
            let is_match = handlers[index].as_object().is_some_and(matches_handler);
            if !is_match {
                index += 1;
                continue;
            }
            if replaced {
                handlers.remove(index);
                continue;
            }
            handlers[index] = desired.clone();
            replaced = true;
            index += 1;
        }
    }
    if replaced {
        return Ok(());
    }
    Err(CliError::Other(
        "could not update the identified Codex recall hook; no file was changed".into(),
    ))
}

fn root_object_mut(root: &mut Value) -> Result<&mut Map<String, Value>, CliError> {
    root.as_object_mut().ok_or_else(|| {
        CliError::Other("Codex hooks must have an object root; no file was changed".into())
    })
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let parent = path.parent().ok_or_else(|| {
        CliError::Other(format!(
            "Codex hooks path has no parent: {}",
            path.display()
        ))
    })?;
    fs::create_dir_all(parent)?;
    let existing_permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    if let Some(permissions) = existing_permissions {
        temporary.as_file().set_permissions(permissions)?;
    }
    temporary
        .persist(path)
        .map_err(|error| CliError::Io(error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn context(root: &Path) -> InstallContext {
        InstallContext {
            current_dir: root.join("repo").join("nested"),
            codex_home: root.join("home").join(".codex"),
        }
    }

    fn setup_context(temp: &tempfile::TempDir) -> InstallContext {
        let root = temp.path();
        fs::create_dir_all(root.join("repo/.git")).unwrap();
        fs::create_dir_all(root.join("repo/nested")).unwrap();
        context(root)
    }

    fn test_executable(root: &Path) -> PathBuf {
        root.join("bin with spaces").join("flicknote")
    }

    fn installed_handler(root: &Value) -> &Value {
        &root["hooks"][HOOK_EVENT][0]["hooks"][0]
    }

    #[test]
    fn local_install_needs_no_mcp_registration_and_preserves_other_hooks() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        fs::create_dir_all(paths.local_hooks.parent().unwrap()).unwrap();
        fs::write(
            &paths.local_hooks,
            r#"{"description":"keep","hooks":{"Stop":[{"hooks":[{"type":"command","command":"keep"}]}]}}"#,
        )
        .unwrap();
        let executable = test_executable(temp.path());

        let result = install_codex_with_executable(Scope::Local, &paths, &executable).unwrap();
        assert_eq!(
            result,
            InstallResult::Installed {
                path: paths.local_hooks.clone(),
                updated: false,
            }
        );
        let installed: Value =
            serde_json::from_str(&fs::read_to_string(&paths.local_hooks).unwrap()).unwrap();
        assert_eq!(installed["description"], "keep");
        assert_eq!(installed["hooks"]["Stop"][0]["hooks"][0]["command"], "keep");
        assert_eq!(installed_handler(&installed)["type"], COMMAND_HOOK_TYPE);
        assert_eq!(
            installed_handler(&installed)["command"],
            format!("{} recall --hook", shell_quote(&executable).unwrap())
        );
        assert_eq!(
            installed_handler(&installed)["timeout"],
            RECALL_TIMEOUT_SECONDS
        );
        assert!(installed_handler(&installed).get("input").is_none());
        assert!(installed_handler(&installed).get("server").is_none());
    }

    #[test]
    fn repeated_install_replaces_mcp_and_command_matches_and_coalesces_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        fs::create_dir_all(paths.local_hooks.parent().unwrap()).unwrap();
        let old_command = format!(
            "{} recall --hook",
            shell_quote(Path::new("/old path/flicknote")).unwrap()
        );
        fs::write(
            &paths.local_hooks,
            json!({
                "hooks": {HOOK_EVENT: [{
                    "matcher": "keep",
                    "hooks": [
                        {"type": MCP_HOOK_TYPE, "server": "flicknote", "tool": RECALL_TOOL, "input": {"prompt": "old"}},
                        {"type": COMMAND_HOOK_TYPE, "command": old_command, "timeout": 99},
                        {"type": "command", "command": "unrelated"}
                    ]
                }]}
            })
            .to_string(),
        )
        .unwrap();
        let executable = test_executable(temp.path());

        let result = install_codex_with_executable(Scope::Local, &paths, &executable).unwrap();
        assert_eq!(
            result,
            InstallResult::Installed {
                path: paths.local_hooks.clone(),
                updated: true,
            }
        );
        let before = fs::read_to_string(&paths.local_hooks).unwrap();
        let installed: Value = serde_json::from_str(&before).unwrap();
        let handlers = installed["hooks"][HOOK_EVENT][0]["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(handlers.len(), 2);
        assert_eq!(handlers[0], desired_handler(&executable).unwrap());
        assert_eq!(handlers[1]["command"], "unrelated");

        let result = install_codex_with_executable(Scope::Local, &paths, &executable).unwrap();
        assert_eq!(
            result,
            InstallResult::Installed {
                path: paths.local_hooks.clone(),
                updated: true,
            }
        );
        assert_eq!(before, fs::read_to_string(&paths.local_hooks).unwrap());
    }

    #[test]
    fn another_scope_is_reported_without_writing_the_target() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        fs::create_dir_all(paths.global_hooks.parent().unwrap()).unwrap();
        fs::write(
            &paths.global_hooks,
            json!({"hooks": {HOOK_EVENT: [{"hooks": [desired_handler(&test_executable(temp.path())).unwrap()]}]}})
                .to_string(),
        )
        .unwrap();

        let result =
            install_codex_with_executable(Scope::Local, &paths, &test_executable(temp.path()))
                .unwrap();
        assert!(matches!(result, InstallResult::AlreadyConfigured { .. }));
        assert!(!paths.local_hooks.exists());
    }

    #[test]
    fn inline_recall_handler_is_reported_without_an_mcp_registration() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        fs::create_dir_all(paths.global_config.parent().unwrap()).unwrap();
        fs::write(
            &paths.global_config,
            r#"
[[hooks.UserPromptSubmit]]
[[hooks.UserPromptSubmit.hooks]]
type = "mcp_tool"
tool = "note_recall"
"#,
        )
        .unwrap();

        let result =
            install_codex_with_executable(Scope::Local, &paths, &test_executable(temp.path()))
                .unwrap();
        assert!(matches!(result, InstallResult::AlreadyConfigured { .. }));
        assert!(!paths.local_hooks.exists());
    }

    #[test]
    fn invalid_target_json_is_not_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        fs::create_dir_all(paths.local_hooks.parent().unwrap()).unwrap();
        fs::write(&paths.local_hooks, "not json").unwrap();
        let error =
            install_codex_with_executable(Scope::Local, &paths, &test_executable(temp.path()))
                .unwrap_err();
        assert!(error.to_string().contains("invalid Codex hooks JSON"));
        assert_eq!(fs::read_to_string(&paths.local_hooks).unwrap(), "not json");
    }

    #[test]
    fn invalid_inline_configuration_is_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        fs::create_dir_all(paths.global_config.parent().unwrap()).unwrap();
        fs::write(&paths.global_config, "[hooks\n").unwrap();
        let error =
            install_codex_with_executable(Scope::Local, &paths, &test_executable(temp.path()))
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("invalid Codex TOML configuration")
        );
        assert!(!paths.local_hooks.exists());
    }

    #[test]
    fn interactive_scope_shows_actual_paths_and_accepts_local() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        let mut input = Cursor::new("1\n");
        let mut output = Vec::new();
        let choice = requested_scope(
            &CodexInstallArgs::default(),
            &paths,
            &mut input,
            &mut output,
            true,
        )
        .unwrap();
        assert_eq!(choice, Some(Scope::Local));
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(&paths.local_hooks.display().to_string()));
        assert!(output.contains(&paths.global_hooks.display().to_string()));
    }

    #[test]
    fn blank_interactive_input_cancels_without_a_choice() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        let mut input = Cursor::new("\n");
        let mut output = Vec::new();
        assert_eq!(
            requested_scope(
                &CodexInstallArgs::default(),
                &paths,
                &mut input,
                &mut output,
                true,
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn noninteractive_install_requires_an_explicit_scope() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let error = requested_scope(
            &CodexInstallArgs::default(),
            &paths,
            &mut input,
            &mut output,
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("--local or --global"));
        assert!(output.is_empty());
        assert!(!paths.local_hooks.exists());
        assert!(!paths.global_hooks.exists());
    }

    #[test]
    fn shell_quote_matches_flicknote_commands_with_spaces_and_quotes() {
        let executable = Path::new("/tmp/with spaces/neil's/flicknote");
        let command = format!("{} recall --hook", shell_quote(executable).unwrap());
        assert!(is_recall_command(&command));
        assert_eq!(
            command,
            "'/tmp/with spaces/neil'\\''s/flicknote' recall --hook"
        );
    }

    #[cfg(unix)]
    #[test]
    fn generated_command_delivers_adversarial_stdin_without_shell_interpolation() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};

        let temp = tempfile::tempdir().unwrap();
        let executable = test_executable(temp.path());
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(
            &executable,
            "#!/bin/sh\ncat > \"$FLICKNOTE_TEST_CAPTURE\"\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).unwrap();
        let capture = temp.path().join("captured.json");
        let prompt = r#"{"hook_event_name":"UserPromptSubmit","prompt":"$(touch SHOULD_NOT_EXIST); `touch ALSO_NOT`; \"quoted\""}"#;
        let command = desired_handler(&executable).unwrap()["command"]
            .as_str()
            .unwrap()
            .to_string();

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .env("FLICKNOTE_TEST_CAPTURE", &capture)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(prompt.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(fs::read_to_string(capture).unwrap(), prompt);
        assert!(!temp.path().join("SHOULD_NOT_EXIST").exists());
        assert!(!temp.path().join("ALSO_NOT").exists());
    }
}
