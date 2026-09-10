use clap::{Args, Subcommand};
use flicknote_core::error::CliError;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const HOOK_EVENT: &str = "UserPromptSubmit";
const HOOK_TYPE: &str = "mcp_tool";
const RECALL_TOOL: &str = "note_recall";
const RECALL_TIMEOUT_SECONDS: u64 = 1;
const MCP_SERVERS_KEYS: [&str; 2] = ["mcp_servers", "mcpServers"];

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
    scope: Scope,
    document: Value,
    servers: RawServerMap,
}

type ServerMap = BTreeMap<String, ServerDefinition>;
type RawServerMap = BTreeMap<String, RawServerDefinition>;

#[derive(Debug, Clone, Default)]
struct RawServerDefinition {
    command: Option<String>,
    args: Option<Vec<String>>,
    enabled: Option<bool>,
    enabled_tools: Option<Vec<String>>,
    disabled_tools: Option<Vec<String>>,
}

impl RawServerDefinition {
    fn merged_with(&self, overrides: &Self) -> Self {
        Self {
            command: overrides.command.clone().or_else(|| self.command.clone()),
            args: overrides.args.clone().or_else(|| self.args.clone()),
            enabled: overrides.enabled.or(self.enabled),
            enabled_tools: overrides
                .enabled_tools
                .clone()
                .or_else(|| self.enabled_tools.clone()),
            disabled_tools: overrides
                .disabled_tools
                .clone()
                .or_else(|| self.disabled_tools.clone()),
        }
    }

    fn effective(&self) -> ServerDefinition {
        let flicknote = match (&self.command, &self.args) {
            (Some(command), Some(args)) => is_flicknote_command(command, args),
            _ => false,
        };
        let note_recall_available = self
            .enabled_tools
            .as_ref()
            .is_none_or(|tools| tools.iter().any(|tool| tool == RECALL_TOOL))
            && self
                .disabled_tools
                .as_ref()
                .is_none_or(|tools| !tools.iter().any(|tool| tool == RECALL_TOOL));
        ServerDefinition {
            flicknote,
            enabled: self.enabled.unwrap_or(true),
            note_recall_available,
        }
    }
}

#[derive(Debug, Clone)]
struct ServerDefinition {
    flicknote: bool,
    enabled: bool,
    note_recall_available: bool,
}

struct ServerResolution {
    selected: String,
    global: ServerMap,
    local: ServerMap,
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
    let sources = read_config_sources(paths)?;
    reject_disabled_hooks(&sources)?;
    let resolution = resolve_server(scope, &sources)?;

    let local_json = read_hooks_json(&paths.local_hooks)?;
    let global_json = read_hooks_json(&paths.global_hooks)?;
    let mut matches = Vec::new();
    if let Some(root) = local_json.as_ref() {
        matches.extend(find_json_matches(
            root,
            &paths.local_hooks,
            &resolution.local,
        )?);
    }
    if let Some(root) = global_json.as_ref() {
        let servers = if scope == Scope::Local {
            &resolution.local
        } else {
            &resolution.global
        };
        matches.extend(find_json_matches(root, &paths.global_hooks, servers)?);
    }
    for source in &sources {
        let servers = if scope == Scope::Local {
            &resolution.local
        } else {
            match source.scope {
                Scope::Global => &resolution.global,
                Scope::Local => &resolution.local,
            }
        };
        matches.extend(find_inline_matches(&source.document, source, servers)?);
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
    let updated = if target_file_matches == 1 {
        update_existing_json_match(&mut root, &resolution.selected)?;
        true
    } else if target_file_matches == 0 {
        add_json_hook(&mut root, &resolution.selected)?;
        false
    } else {
        return Ok(InstallResult::AlreadyConfigured {
            locations: matches.into_iter().map(|item| item.location).collect(),
        });
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
        "Prerequisite: the FlickNote MCP server must already be connected to Codex."
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
    for (scope, path) in [
        (Scope::Global, &paths.global_config),
        (Scope::Local, &paths.local_config),
    ] {
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
        let document = toml_to_json(document);
        let servers = configured_servers(&document, path)?;
        sources.push(ConfigSource {
            path: path.clone(),
            scope,
            document,
            servers,
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

fn resolve_server(scope: Scope, sources: &[ConfigSource]) -> Result<ServerResolution, CliError> {
    let mut global_raw = RawServerMap::new();
    let mut local_raw = RawServerMap::new();
    for source in sources {
        match source.scope {
            Scope::Global => global_raw.extend(source.servers.clone()),
            Scope::Local => local_raw.extend(source.servers.clone()),
        }
    }

    let global = effective_server_map(&global_raw);
    let effective_local = merge_server_maps(&global_raw, &local_raw);
    let effective = match scope {
        Scope::Global => &global,
        Scope::Local => &effective_local,
    };
    let candidates = live_servers(effective);
    if candidates.is_empty() {
        if scope == Scope::Global && !live_servers(&effective_local).is_empty() {
            return Err(CliError::Other(
                "cannot install a global Codex hook: the enabled FlickNote MCP registration with note_recall exists only in the current project's config.toml".into(),
            ));
        }
        return Err(no_available_server_error(scope, effective));
    }
    if candidates.len() != 1 {
        return Err(CliError::Other(format!(
            "ambiguous FlickNote MCP registrations in Codex config.toml: {}",
            candidates.join(", ")
        )));
    }

    Ok(ServerResolution {
        selected: candidates[0].clone(),
        global,
        local: effective_local,
    })
}

fn effective_server_map(servers: &RawServerMap) -> ServerMap {
    servers
        .iter()
        .map(|(name, server)| (name.clone(), server.effective()))
        .collect()
}

fn merge_server_maps(global: &RawServerMap, local: &RawServerMap) -> ServerMap {
    let mut merged = global.clone();
    for (name, server) in local {
        merged
            .entry(name.clone())
            .and_modify(|global| *global = global.merged_with(server))
            .or_insert_with(|| server.clone());
    }
    effective_server_map(&merged)
}

fn configured_servers(document: &Value, path: &Path) -> Result<RawServerMap, CliError> {
    let mut servers = RawServerMap::new();
    for key in MCP_SERVERS_KEYS {
        let Some(value) = document.get(key) else {
            continue;
        };
        let Some(server_entries) = value.as_object() else {
            return Err(CliError::Other(format!(
                "invalid Codex [{key}] table in {}",
                path.display()
            )));
        };
        for (name, server) in server_entries {
            let Some(server) = server.as_object() else {
                return Err(CliError::Other(format!(
                    "invalid Codex MCP server {name:?} in {}",
                    path.display()
                )));
            };

            let command = server
                .get("command")
                .map(|value| {
                    value.as_str().map(str::to_string).ok_or_else(|| {
                        CliError::Other(format!(
                            "invalid command for Codex MCP server {name:?} in {}",
                            path.display()
                        ))
                    })
                })
                .transpose()?;
            let args = string_array(server.get("args"), || {
                format!(
                    "invalid args for Codex MCP server {name:?} in {}",
                    path.display()
                )
            })?;
            let enabled = server
                .get("enabled")
                .map(|value| {
                    value.as_bool().ok_or_else(|| {
                        CliError::Other(format!(
                            "invalid enabled value for Codex MCP server {name:?} in {}",
                            path.display()
                        ))
                    })
                })
                .transpose()?;
            let enabled_tools = string_array(server.get("enabled_tools"), || {
                format!(
                    "invalid enabled_tools for Codex MCP server {name:?} in {}",
                    path.display()
                )
            })?;
            let disabled_tools = string_array(server.get("disabled_tools"), || {
                format!(
                    "invalid disabled_tools for Codex MCP server {name:?} in {}",
                    path.display()
                )
            })?;
            servers.insert(
                name.clone(),
                RawServerDefinition {
                    command,
                    args,
                    enabled,
                    enabled_tools,
                    disabled_tools,
                },
            );
        }
    }
    Ok(servers)
}

fn string_array<F>(value: Option<&Value>, message: F) -> Result<Option<Vec<String>>, CliError>
where
    F: Fn() -> String,
{
    let Some(value) = value else {
        return Ok(None);
    };
    let values = value.as_array().ok_or_else(|| CliError::Other(message()))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| CliError::Other(message()))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn live_servers(servers: &ServerMap) -> Vec<String> {
    servers
        .iter()
        .filter(|(_, server)| server.flicknote && server.enabled && server.note_recall_available)
        .map(|(name, _)| name.clone())
        .collect()
}

fn no_available_server_error(scope: Scope, servers: &ServerMap) -> CliError {
    let flicknote_names = servers
        .iter()
        .filter(|(_, server)| server.flicknote)
        .map(|(name, server)| {
            let status = match (server.enabled, server.note_recall_available) {
                (false, _) => "disabled",
                (_, false) => "note_recall unavailable",
                (true, true) => "available",
            };
            format!("{name} ({status})")
        })
        .collect::<Vec<_>>();
    let scope_hint = if scope == Scope::Local {
        " in the effective user/project configuration"
    } else {
        " in the current user's configuration"
    };
    let detail = if flicknote_names.is_empty() {
        if servers.is_empty() {
            "add an mcp_servers entry whose command is flicknote with the mcp argument".to_string()
        } else {
            "the effective configuration contains no usable FlickNote entry; a project server may be shadowing a user entry, or add an mcp_servers entry whose command is flicknote with the mcp argument".to_string()
        }
    } else {
        format!(
            "the discovered registrations are not usable: {}",
            flicknote_names.join(", ")
        )
    };
    CliError::Other(format!(
        "could not find an enabled FlickNote MCP registration with note_recall available{scope_hint}; {detail}, then retry"
    ))
}

fn is_flicknote_command(command: &str, args: &[String]) -> bool {
    let name = Path::new(command)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(command);
    (name == "flicknote" || name == "flicknote.exe") && args.iter().any(|arg| arg == "mcp")
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

fn find_json_matches(
    root: &Value,
    path: &Path,
    servers: &ServerMap,
) -> Result<Vec<HookMatch>, CliError> {
    find_hook_matches(
        root,
        &path.display().to_string(),
        Some(path),
        HookFormat::Json,
        servers,
    )
}

fn find_inline_matches(
    document: &Value,
    source: &ConfigSource,
    servers: &ServerMap,
) -> Result<Vec<HookMatch>, CliError> {
    find_hook_matches(
        document,
        &source.path.display().to_string(),
        None,
        HookFormat::Toml,
        servers,
    )
}

fn find_hook_matches(
    root: &Value,
    source_label: &str,
    path: Option<&Path>,
    format: HookFormat,
    servers: &ServerMap,
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
            if matches_handler(handler, servers)? {
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

fn matches_handler(handler: &Map<String, Value>, servers: &ServerMap) -> Result<bool, CliError> {
    if handler.get("type").and_then(Value::as_str) != Some(HOOK_TYPE) {
        return Ok(false);
    }
    let Some(server) = handler.get("server").and_then(Value::as_str) else {
        return Err(CliError::Other(
            "Codex mcp_tool hook is missing its server name; no file was changed".into(),
        ));
    };
    let Some(tool) = handler.get("tool").and_then(Value::as_str) else {
        return Err(CliError::Other(
            "Codex mcp_tool hook is missing its tool name; no file was changed".into(),
        ));
    };
    Ok(tool == RECALL_TOOL
        && servers.get(server).is_some_and(|definition| {
            definition.flicknote && definition.enabled && definition.note_recall_available
        }))
}

fn handler_disabled(handler: &Map<String, Value>) -> bool {
    handler.get("enabled").and_then(Value::as_bool) == Some(false)
        || handler.get("disabled").and_then(Value::as_bool) == Some(true)
}

fn desired_handler(server: &str) -> Value {
    json!({
        "type": HOOK_TYPE,
        "server": server,
        "tool": RECALL_TOOL,
        "input": { "prompt": "${prompt}" },
        "timeout": RECALL_TIMEOUT_SECONDS,
    })
}

fn add_json_hook(root: &mut Value, server: &str) -> Result<(), CliError> {
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
    events.push(json!({ "hooks": [desired_handler(server)] }));
    Ok(())
}

fn update_existing_json_match(root: &mut Value, server: &str) -> Result<(), CliError> {
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
    for group in events {
        let Some(group) = group.as_object_mut() else {
            continue;
        };
        let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        for handler in handlers {
            let Some(handler) = handler.as_object_mut() else {
                continue;
            };
            if handler.get("type").and_then(Value::as_str) != Some(HOOK_TYPE)
                || handler.get("tool").and_then(Value::as_str) != Some(RECALL_TOOL)
                || handler.get("server").and_then(Value::as_str) != Some(server)
            {
                continue;
            }
            let desired = desired_handler(server);
            let desired = desired.as_object().unwrap();
            for key in ["type", "server", "tool", "input", "timeout"] {
                handler.insert(key.to_string(), desired[key].clone());
            }
            if handler.get("async").and_then(Value::as_bool) == Some(true) {
                handler.insert("async".to_string(), Value::Bool(false));
            } else {
                handler.remove("async");
            }
            return Ok(());
        }
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

    fn write_config(context: &InstallContext, local: bool, name: &str) {
        let paths = context.paths();
        let path = if local {
            &paths.local_config
        } else {
            &paths.global_config
        };
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!("[mcp_servers.{name}]\ncommand = \"flicknote\"\nargs = [\"mcp\"]\n"),
        )
        .unwrap();
    }

    fn setup_context(temp: &tempfile::TempDir) -> InstallContext {
        let root = temp.path();
        fs::create_dir_all(root.join("repo/.git")).unwrap();
        fs::create_dir_all(root.join("repo/nested")).unwrap();
        context(root)
    }

    #[test]
    fn local_install_uses_git_root_and_preserves_other_hooks() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        write_config(&context, false, "custom_flicknote");
        let paths = context.paths();
        fs::create_dir_all(paths.local_hooks.parent().unwrap()).unwrap();
        fs::write(
            &paths.local_hooks,
            r#"{"description":"keep","hooks":{"Stop":[{"hooks":[{"type":"command","command":"keep"}]}]}}"#,
        )
        .unwrap();

        let result = install_codex(Scope::Local, &paths).unwrap();
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
        assert_eq!(installed["hooks"][HOOK_EVENT].as_array().unwrap().len(), 1);
        assert_eq!(
            installed["hooks"][HOOK_EVENT][0]["hooks"][0]["server"],
            "custom_flicknote"
        );
        assert_eq!(
            installed["hooks"][HOOK_EVENT][0]["hooks"][0]["input"]["prompt"],
            "${prompt}"
        );
    }

    #[test]
    fn repeated_install_updates_one_match_without_duplicating_it() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        write_config(&context, false, "flicknote");
        let paths = context.paths();
        fs::create_dir_all(paths.local_hooks.parent().unwrap()).unwrap();
        fs::write(
            &paths.local_hooks,
            json!({"hooks": {HOOK_EVENT: [{"matcher": "ignored", "hooks": [{"type": HOOK_TYPE, "server": "flicknote", "tool": RECALL_TOOL, "timeout": 30, "input": {"prompt": "old"}}]}]}}).to_string(),
        )
        .unwrap();
        install_codex(Scope::Local, &paths).unwrap();
        let before = fs::read_to_string(&paths.local_hooks).unwrap();
        let result = install_codex(Scope::Local, &paths).unwrap();
        assert_eq!(
            result,
            InstallResult::Installed {
                path: paths.local_hooks.clone(),
                updated: true
            }
        );
        let installed: Value =
            serde_json::from_str(&fs::read_to_string(&paths.local_hooks).unwrap()).unwrap();
        assert_eq!(installed["hooks"][HOOK_EVENT].as_array().unwrap().len(), 1);
        assert_eq!(
            installed["hooks"][HOOK_EVENT][0]["hooks"][0]["timeout"],
            RECALL_TIMEOUT_SECONDS
        );
        assert_eq!(
            installed["hooks"][HOOK_EVENT][0]["hooks"][0]["input"]["prompt"],
            "${prompt}"
        );
        assert_eq!(before, fs::read_to_string(&paths.local_hooks).unwrap());
    }

    #[test]
    fn cross_scope_hook_is_reported_without_writing_target() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        write_config(&context, false, "flicknote");
        let paths = context.paths();
        fs::create_dir_all(paths.global_hooks.parent().unwrap()).unwrap();
        fs::write(
            &paths.global_hooks,
            json!({"hooks": {HOOK_EVENT: [{"hooks": [{"type": HOOK_TYPE, "server": "flicknote", "tool": RECALL_TOOL}]}]}}).to_string(),
        )
        .unwrap();
        let result = install_codex(Scope::Local, &paths).unwrap();
        assert!(matches!(result, InstallResult::AlreadyConfigured { .. }));
        assert!(!paths.local_hooks.exists());
    }

    #[test]
    fn global_install_rejects_local_only_registration() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        write_config(&context, true, "flicknote");
        let paths = context.paths();
        let error = install_codex(Scope::Global, &paths).unwrap_err();
        assert!(
            error.to_string().contains("current project's"),
            "unexpected error: {error}"
        );
        assert!(!paths.global_hooks.exists());
    }

    #[test]
    fn invalid_target_json_is_not_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        write_config(&context, false, "flicknote");
        let paths = context.paths();
        fs::create_dir_all(paths.local_hooks.parent().unwrap()).unwrap();
        fs::write(&paths.local_hooks, "not json").unwrap();
        let error = install_codex(Scope::Local, &paths).unwrap_err();
        assert!(error.to_string().contains("invalid Codex hooks JSON"));
        assert_eq!(fs::read_to_string(&paths.local_hooks).unwrap(), "not json");
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
    fn inline_hook_is_reported_without_writing_a_file_hook() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        fs::create_dir_all(paths.global_config.parent().unwrap()).unwrap();
        fs::write(
            &paths.global_config,
            r#"
[mcp_servers.flicknote]
command = "flicknote"
args = ["mcp"]

[[hooks.UserPromptSubmit]]
[[hooks.UserPromptSubmit.hooks]]
type = "mcp_tool"
server = "flicknote"
tool = "note_recall"
"#,
        )
        .unwrap();

        let result = install_codex(Scope::Local, &paths).unwrap();
        assert!(matches!(result, InstallResult::AlreadyConfigured { .. }));
        assert!(!paths.local_hooks.exists());
    }

    #[test]
    fn disabled_hooks_are_reported_without_writing_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        fs::create_dir_all(paths.global_config.parent().unwrap()).unwrap();
        fs::write(
            &paths.global_config,
            r#"
[mcp_servers.flicknote]
command = "flicknote"
args = ["mcp"]

[features]
hooks = false
"#,
        )
        .unwrap();

        let error = install_codex(Scope::Local, &paths).unwrap_err();
        assert!(error.to_string().contains("explicitly disabled"));
        assert!(!paths.local_hooks.exists());
    }

    #[test]
    fn disabled_mcp_server_is_not_selected() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        fs::create_dir_all(paths.global_config.parent().unwrap()).unwrap();
        fs::write(
            &paths.global_config,
            r#"
[mcp_servers.flicknote]
command = "flicknote"
args = ["mcp"]
enabled = false
"#,
        )
        .unwrap();

        let error = install_codex(Scope::Local, &paths).unwrap_err();
        assert!(
            error.to_string().contains("disabled"),
            "unexpected error: {error}"
        );
        assert!(!paths.local_hooks.exists());
    }

    #[test]
    fn mcp_server_without_recall_tool_is_not_selected() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        let paths = context.paths();
        fs::create_dir_all(paths.global_config.parent().unwrap()).unwrap();
        fs::write(
            &paths.global_config,
            r#"
[mcp_servers.flicknote]
command = "flicknote"
args = ["mcp"]
enabled_tools = ["note_get"]
"#,
        )
        .unwrap();

        let error = install_codex(Scope::Local, &paths).unwrap_err();
        assert!(
            error.to_string().contains("note_recall unavailable"),
            "unexpected error: {error}"
        );
        assert!(!paths.local_hooks.exists());
    }

    #[test]
    fn local_server_overrides_same_name_global_server() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        write_config(&context, false, "flicknote");
        let paths = context.paths();
        fs::create_dir_all(paths.local_config.parent().unwrap()).unwrap();
        fs::write(
            &paths.local_config,
            r#"
[mcp_servers.flicknote]
command = "other-mcp"
args = ["mcp"]
"#,
        )
        .unwrap();

        let error = install_codex(Scope::Local, &paths).unwrap_err();
        assert!(
            error.to_string().contains("shadowing"),
            "unexpected error: {error}"
        );
        assert!(!paths.local_hooks.exists());
    }

    #[test]
    fn local_server_inherits_unspecified_global_fields() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        write_config(&context, false, "fn");
        let paths = context.paths();
        fs::create_dir_all(paths.local_config.parent().unwrap()).unwrap();
        fs::write(
            &paths.local_config,
            r#"
[mcp_servers.fn]
enabled_tools = ["note_recall"]
"#,
        )
        .unwrap();

        let result = install_codex(Scope::Local, &paths).unwrap();
        assert_eq!(
            result,
            InstallResult::Installed {
                path: paths.local_hooks.clone(),
                updated: false,
            }
        );
        let installed: Value =
            serde_json::from_str(&fs::read_to_string(&paths.local_hooks).unwrap()).unwrap();
        assert_eq!(
            installed["hooks"][HOOK_EVENT][0]["hooks"][0]["server"],
            "fn"
        );
    }

    #[test]
    fn locally_shadowed_global_hook_does_not_block_local_install() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        write_config(&context, false, "fn");
        let paths = context.paths();
        fs::create_dir_all(paths.global_hooks.parent().unwrap()).unwrap();
        fs::write(
            &paths.global_hooks,
            json!({"hooks": {HOOK_EVENT: [{"hooks": [{"type": HOOK_TYPE, "server": "fn", "tool": RECALL_TOOL}]}]}}).to_string(),
        )
        .unwrap();
        fs::create_dir_all(paths.local_config.parent().unwrap()).unwrap();
        fs::write(
            &paths.local_config,
            r#"
[mcp_servers.fn]
command = "other-mcp"
args = ["mcp"]

[mcp_servers.project_flicknote]
command = "flicknote"
args = ["mcp"]
"#,
        )
        .unwrap();

        let result = install_codex(Scope::Local, &paths).unwrap();
        assert_eq!(
            result,
            InstallResult::Installed {
                path: paths.local_hooks.clone(),
                updated: false,
            }
        );
        let installed: Value =
            serde_json::from_str(&fs::read_to_string(&paths.local_hooks).unwrap()).unwrap();
        assert_eq!(
            installed["hooks"][HOOK_EVENT][0]["hooks"][0]["server"],
            "project_flicknote"
        );
    }

    #[test]
    fn global_repeat_uses_global_server_definition_after_local_disable() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        write_config(&context, false, "fn");
        let paths = context.paths();
        fs::create_dir_all(paths.local_config.parent().unwrap()).unwrap();
        fs::write(
            &paths.local_config,
            r#"
[mcp_servers.fn]
enabled = false
"#,
        )
        .unwrap();
        fs::create_dir_all(paths.global_hooks.parent().unwrap()).unwrap();
        fs::write(
            &paths.global_hooks,
            json!({"hooks": {HOOK_EVENT: [{"hooks": [{"type": HOOK_TYPE, "server": "fn", "tool": RECALL_TOOL, "timeout": 30}]}]}}).to_string(),
        )
        .unwrap();

        let result = install_codex(Scope::Global, &paths).unwrap();
        assert_eq!(
            result,
            InstallResult::Installed {
                path: paths.global_hooks.clone(),
                updated: true,
            }
        );
        let installed: Value =
            serde_json::from_str(&fs::read_to_string(&paths.global_hooks).unwrap()).unwrap();
        assert_eq!(installed["hooks"][HOOK_EVENT].as_array().unwrap().len(), 1);
        assert_eq!(
            installed["hooks"][HOOK_EVENT][0]["hooks"][0]["timeout"],
            RECALL_TIMEOUT_SECONDS
        );
    }

    #[test]
    fn live_local_hook_with_a_different_server_name_blocks_global_install() {
        let temp = tempfile::tempdir().unwrap();
        let context = setup_context(&temp);
        write_config(&context, false, "global_flicknote");
        write_config(&context, true, "project_flicknote");
        let paths = context.paths();
        fs::create_dir_all(paths.local_hooks.parent().unwrap()).unwrap();
        fs::write(
            &paths.local_hooks,
            json!({"hooks": {HOOK_EVENT: [{"hooks": [{"type": HOOK_TYPE, "server": "project_flicknote", "tool": RECALL_TOOL}]}]}}).to_string(),
        )
        .unwrap();

        let result = install_codex(Scope::Global, &paths).unwrap();
        assert!(matches!(result, InstallResult::AlreadyConfigured { .. }));
        assert!(!paths.global_hooks.exists());
    }
}
