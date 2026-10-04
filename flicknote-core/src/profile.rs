use crate::config::Config;
use std::path::{Path, PathBuf};

const MARKER: &str = "flicknote-real-account-trial-v1\n";

// Resolve symlinks in every existing ancestor before any mutation. Also rejects
// ancestors of the live dirs: a trial marker must never bless an XDG/home root.
fn resolved(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Trial root must be absolute and contain no '..'".into());
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

pub fn load(root: &Path) -> Result<Config, String> {
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
    if std::fs::symlink_metadata(root.join("REAL_ACCOUNT_TRIAL"))
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err("Trial marker must not be a symlink".into());
    }
    if root.exists()
        && std::fs::read_dir(&root)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
        && std::fs::read_to_string(root.join("REAL_ACCOUNT_TRIAL"))
            .ok()
            .as_deref()
            != Some(MARKER)
    {
        return Err("Refusing nonempty root without real-account trial marker".into());
    }
    // Existing trial subdirectories may not escape through symlinks.
    for sub in ["data", "data/flicknote", "config", "config/flicknote"] {
        if resolved(&root.join(sub))? != root.join(sub) {
            return Err("Trial subdirectories must not contain symlinks".into());
        }
    }
    let data = root.join("data/flicknote");
    let config = root.join("config/flicknote");
    std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&config).map_err(|e| e.to_string())?;
    if !root.join("REAL_ACCOUNT_TRIAL").exists() {
        std::fs::write(root.join("REAL_ACCOUNT_TRIAL"), MARKER).map_err(|e| e.to_string())?;
    }
    // Refuse database/session/socket aliases as well, without opening them.
    for path in [
        data.join("flicknote.db"),
        data.join("daemon.sock"),
        data.join("flicknote.log"),
        data.join("daemon.lock"),
        config.join("session.json"),
        config.join("session.pkce"),
        config.join("config.json"),
    ] {
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Trial files must not be symlinks".into());
        }
    }
    Config::load_from_dirs(config, data).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_profile_is_independent_and_rejects_unmarked_or_aliased_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap().join("trial");
        let config = load(&root).unwrap();
        assert_eq!(
            config.paths.session_file,
            root.join("config/flicknote/session.json")
        );
        assert_eq!(
            config.paths.db_file,
            root.join("data/flicknote/flicknote.db")
        );
        assert!(!config.paths.session_file.exists());
        assert!(!config.paths.db_file.exists());
        std::fs::write(
            &config.paths.config_file,
            r#"{"supabaseUrl":"http://127.0.0.1:12345"}"#,
        )
        .unwrap();
        assert_eq!(load(&root).unwrap().supabase_url, "http://127.0.0.1:12345");
        let other = directory.path().join("unmarked");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("keep"), "untouched").unwrap();
        assert!(load(&other).is_err());
        assert!(!other.join("data").exists());
        let target = directory.path().join("session-fixture");
        std::fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, &config.paths.session_file).unwrap();
        assert!(load(&root).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "keep");
    }
}
