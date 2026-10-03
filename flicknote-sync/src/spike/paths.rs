use flicknote_core::config::{Config, ConfigPaths};
use std::path::{Path, PathBuf};

const MARKER: &str = "flicknote-embedded-gpui-synthetic-v1\n";

// Resolve symlinks in every existing ancestor before any mutation. Also rejects
// ancestors of the live dirs: a spike marker must never bless an XDG/home root.
fn resolved(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Spike root must be absolute and contain no '..'".into());
    }
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(ancestor.file_name().ok_or("Invalid root")?.to_owned());
        ancestor = ancestor.parent().ok_or("Invalid root")?;
    }
    let mut result = ancestor.canonicalize().map_err(|e| e.to_string())?;
    for part in suffix.into_iter().rev() {
        result.push(part);
    }
    Ok(result)
}

pub(super) fn config(root: &Path) -> Result<Config, String> {
    let root = resolved(root)?;
    let home = dirs::home_dir().ok_or("Cannot determine home for live-root rejection")?;
    let mut protected = vec![
        home.join(".local/share/flicknote"),
        home.join(".config/flicknote"),
    ];
    for (key, fallback) in [
        ("XDG_DATA_HOME", ".local/share"),
        ("XDG_CONFIG_HOME", ".config"),
    ] {
        let base = std::env::var_os(key)
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(fallback));
        protected.push(base.join("flicknote"));
    }
    for live in protected {
        let live = resolved(&live)?;
        if root.starts_with(&live) || live.starts_with(&root) {
            return Err("Refusing live FlickNote root or its ancestor/descendant".into());
        }
    }
    if std::fs::symlink_metadata(root.join("SPIKE_ONLY")).is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err("Spike marker must not be a symlink".into());
    }
    if root.exists()
        && std::fs::read_dir(&root)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
        && std::fs::read_to_string(root.join("SPIKE_ONLY"))
            .ok()
            .as_deref()
            != Some(MARKER)
    {
        return Err("Refusing nonempty root without synthetic spike marker".into());
    }
    // Existing spike subdirectories may not escape through symlinks.
    for sub in ["data", "data/flicknote", "config", "config/flicknote"] {
        if resolved(&root.join(sub))? != root.join(sub) {
            return Err("Spike subdirectories must not contain symlinks".into());
        }
    }
    let data = root.join("data/flicknote");
    let config = root.join("config/flicknote");
    std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&config).map_err(|e| e.to_string())?;
    if !root.join("SPIKE_ONLY").exists() {
        std::fs::write(root.join("SPIKE_ONLY"), MARKER).map_err(|e| e.to_string())?;
    }
    // Refuse database/session/socket aliases as well, without opening them.
    for path in [
        data.join("flicknote.db"),
        data.join("daemon.sock"),
        data.join("flicknote.log"),
        data.join("daemon.lock"),
        config.join("session.json"),
    ] {
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Spike files must not be symlinks".into());
        }
    }
    Ok(Config {
        supabase_url: String::new(),
        supabase_anon_key: String::new(),
        powersync_url: String::new(),
        api_url: String::new(),
        gateway_url: String::new(),
        web_url: None,
        paths: ConfigPaths {
            config_file: config.join("config.json"),
            session_file: config.join("session.json"),
            db_file: data.join("flicknote.db"),
            log_file: data.join("flicknote.log"),
            config_dir: config,
            data_dir: data,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_and_unmarked_roots_are_rejected_before_writes() {
        let home = dirs::home_dir().unwrap();
        // Only path checks; never inspect or open the live database/session.
        assert!(
            config(&home.join(".local/share/flicknote"))
                .err()
                .unwrap()
                .contains("live")
        );
        assert!(config(&home).is_err());
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("unrelated"), "keep").unwrap();
        assert!(config(root.path()).is_err());
        assert!(!root.path().join("data").exists());
        assert_eq!(
            std::fs::read_to_string(root.path().join("unrelated")).unwrap(),
            "keep"
        );
    }
    #[test]
    fn symlink_escape_is_rejected_before_initialization() {
        let root = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("SPIKE_ONLY"), MARKER).unwrap();
        std::os::unix::fs::symlink(target.path(), root.path().join("data")).unwrap();
        assert!(config(root.path()).is_err());
        assert!(std::fs::read_dir(target.path()).unwrap().next().is_none());
    }
}
