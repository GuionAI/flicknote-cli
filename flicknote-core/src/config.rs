use std::fs;
use std::path::PathBuf;

pub struct Config {
    pub supabase_url: String,
    pub supabase_anon_key: String,
    pub powersync_url: String,
    pub api_url: String,
    pub web_url: Option<String>,
    pub paths: ConfigPaths,
}

pub struct ConfigPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    pub session_file: PathBuf,
    pub db_file: PathBuf,
    pub log_file: PathBuf,
}

impl Config {
    pub fn load() -> Result<Self, crate::error::CliError> {
        let home = dirs::home_dir().ok_or_else(|| {
            crate::error::CliError::Other("Could not determine home directory".into())
        })?;

        let config_dir = std::env::var("XDG_CONFIG_HOME")
            .map(|d| PathBuf::from(d).join("flicknote"))
            .unwrap_or_else(|_| home.join(".config/flicknote"));

        let data_dir = std::env::var("XDG_DATA_HOME")
            .map(|d| PathBuf::from(d).join("flicknote"))
            .unwrap_or_else(|_| home.join(".local/share/flicknote"));

        fs::create_dir_all(&config_dir)?;
        fs::create_dir_all(&data_dir)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700))?;
        }

        let config_file = config_dir.join("config.json");
        let session_file = config_dir.join("session.json");
        let db_file = data_dir.join("flicknote.db");
        let log_file = data_dir.join("flicknote.log");

        let mut supabase_url = String::new();
        let mut supabase_anon_key = String::new();
        let mut powersync_url = String::new();
        let mut api_url = String::new();
        let mut web_url: Option<String> = None;

        if config_file.exists()
            && let Ok(raw) = fs::read_to_string(&config_file)
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw)
        {
            if let Some(v) = json.get("supabaseUrl").and_then(|v| v.as_str()) {
                supabase_url = v.to_string();
            }
            if let Some(v) = json.get("supabaseAnonKey").and_then(|v| v.as_str()) {
                supabase_anon_key = v.to_string();
            }
            if let Some(v) = json.get("powersyncUrl").and_then(|v| v.as_str()) {
                powersync_url = v.to_string();
            }
            if let Some(v) = json.get("apiUrl").and_then(|v| v.as_str()) {
                api_url = v.to_string();
            }
            if let Some(v) = json.get("webUrl").and_then(|v| v.as_str()) {
                web_url = Some(v.to_string());
            }
        }

        if let Ok(v) = std::env::var("FLICKNOTE_SUPABASE_URL") {
            supabase_url = v;
        }
        if let Ok(v) = std::env::var("FLICKNOTE_SUPABASE_KEY") {
            supabase_anon_key = v;
        }
        if let Ok(v) = std::env::var("FLICKNOTE_POWERSYNC_URL") {
            powersync_url = v;
        }
        if let Ok(v) = std::env::var("FLICKNOTE_API_URL") {
            api_url = v;
        }
        if let Ok(v) = std::env::var("FLICKNOTE_WEB_URL") {
            web_url = Some(v);
        }

        // Fallback: per-field built-in defaults if nothing else configured that field.
        // Each field is guarded independently so a user can override just one env var
        // (e.g. FLICKNOTE_POWERSYNC_URL) without losing their custom value when
        // other fields fall back to the built-in set.
        if supabase_url.is_empty()
            || supabase_anon_key.is_empty()
            || powersync_url.is_empty()
            || api_url.is_empty()
        {
            let env = std::env::var("FLICKNOTE_ENV").unwrap_or_else(|_| "dev".into());
            let (s_url, s_key, ps_url, a_url) = builtin_defaults(&env);
            if supabase_url.is_empty() {
                supabase_url = s_url.into();
            }
            if supabase_anon_key.is_empty() {
                supabase_anon_key = s_key.into();
            }
            if powersync_url.is_empty() {
                powersync_url = ps_url.into();
            }
            if api_url.is_empty() {
                api_url = a_url.into();
            }
        }

        let paths = ConfigPaths {
            config_dir,
            data_dir,
            config_file,
            session_file,
            db_file,
            log_file,
        };

        Ok(Self {
            supabase_url,
            supabase_anon_key,
            powersync_url,
            api_url,
            web_url,
            paths,
        })
    }

    /// Validate that api_url is set. Call before API operations.
    pub fn validate_api(&self) -> Result<(), crate::error::CliError> {
        if self.api_url.is_empty() {
            return Err(crate::error::CliError::Other(
                "apiUrl is not configured — set it in config.json or FLICKNOTE_API_URL".into(),
            ));
        }
        Ok(())
    }

    /// Validate that required fields are set. Call before operations that need them.
    /// Under normal usage built-in defaults fill these fields, but explicit empty-string
    /// env vars (e.g. `FLICKNOTE_SUPABASE_URL=`) or a broken config.json can still
    /// result in empty values.
    pub fn validate(&self) -> Result<(), crate::error::CliError> {
        if self.supabase_url.is_empty() {
            return Err(crate::error::CliError::Other(
                "supabaseUrl is not configured — set it in config.json or FLICKNOTE_SUPABASE_URL"
                    .into(),
            ));
        }
        if self.supabase_anon_key.is_empty() {
            return Err(crate::error::CliError::Other(
                "supabaseAnonKey is not configured — set it in config.json or FLICKNOTE_SUPABASE_KEY".into(),
            ));
        }
        if self.powersync_url.is_empty() {
            return Err(crate::error::CliError::Other(
                "powersyncUrl is not configured — set it in config.json or FLICKNOTE_POWERSYNC_URL"
                    .into(),
            ));
        }
        Ok(())
    }
}

fn builtin_defaults(env: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    match env {
        "prod" => (
            "https://auth.flicknote.app",
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImFocGNqYW1maGJpb3BqZG5laW5uIiwicm9sZSI6ImFub24iLCJpYXQiOjE3NTA0NTc1NDIsImV4cCI6MjA2NjAzMzU0Mn0.g6B2UohS8Zw_mrsDljAB7n6feUTvpmMVvvsf7VMRXA4",
            "https://sync.flicknote.app",
            "https://api.flicknote.app/api/v1",
        ),
        _ => (
            "https://dev-auth.flicknote.app",
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoiYW5vbiIsImlzcyI6InN1cGFiYXNlIiwiaWF0IjoxNzY1NTM1NTg4LCJleHAiOjE5MjMyMTU1ODh9.7ErMPvghlVm6mew-IKjSShP1Lf6wTCbNgs9ufuh3yqo",
            "https://dev-sync.flicknote.app",
            "https://dev-api.flicknote.app/api/v1",
        ),
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env vars are process-wide — use a mutex to prevent test interference
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_clean_env<F: FnOnce()>(flicknote_env: Option<&str>, f: F) {
        let _guard = ENV_LOCK.lock().unwrap();
        let keys = [
            "FLICKNOTE_ENV",
            "FLICKNOTE_SUPABASE_URL",
            "FLICKNOTE_SUPABASE_KEY",
            "FLICKNOTE_POWERSYNC_URL",
            "FLICKNOTE_API_URL",
            "FLICKNOTE_WEB_URL",
        ];
        let saved: Vec<_> = keys.iter().map(|k| std::env::var(k).ok()).collect();

        for key in &keys {
            unsafe { std::env::remove_var(key) };
        }
        if let Some(env) = flicknote_env {
            unsafe { std::env::set_var("FLICKNOTE_ENV", env) };
        }

        f();

        for (key, val) in keys.iter().zip(saved) {
            match val {
                Some(v) => unsafe { std::env::set_var(key, v) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }

    #[test]
    fn test_builtin_defaults_dev() {
        let (url, key, ps, api) = builtin_defaults("dev");
        assert_eq!(url, "https://dev-auth.flicknote.app");
        assert_eq!(ps, "https://dev-sync.flicknote.app");
        assert_eq!(api, "https://dev-api.flicknote.app/api/v1");
        assert!(!key.is_empty());
    }

    #[test]
    fn test_builtin_defaults_prod() {
        let (url, key, ps, api) = builtin_defaults("prod");
        assert_eq!(url, "https://auth.flicknote.app");
        assert_eq!(ps, "https://sync.flicknote.app");
        assert_eq!(api, "https://api.flicknote.app/api/v1");
        assert!(!key.is_empty());
    }

    #[test]
    fn test_builtin_defaults_unknown_falls_back_to_dev() {
        let (url, _, _, _) = builtin_defaults("staging");
        assert_eq!(url, "https://dev-auth.flicknote.app");
    }

    #[test]
    fn test_env_var_overrides_builtin() {
        with_clean_env(None, || {
            unsafe { std::env::set_var("FLICKNOTE_SUPABASE_URL", "https://custom.example.com") };
            unsafe {
                std::env::set_var(
                    "XDG_CONFIG_HOME",
                    std::env::temp_dir()
                        .join("flicknote-test-cfg")
                        .to_str()
                        .unwrap(),
                )
            };
            unsafe {
                std::env::set_var(
                    "XDG_DATA_HOME",
                    std::env::temp_dir()
                        .join("flicknote-test-data")
                        .to_str()
                        .unwrap(),
                )
            };
            let cfg = Config::load().expect("Config::load should succeed");
            assert_eq!(cfg.supabase_url, "https://custom.example.com");
        });
    }

    #[test]
    fn test_config_file_overrides_builtin() {
        with_clean_env(None, || {
            let tmp = tempfile::tempdir().expect("tempdir");
            let cfg_dir = tmp.path().join("flicknote");
            std::fs::create_dir_all(&cfg_dir).unwrap();
            let cfg_file = cfg_dir.join("config.json");
            std::fs::write(
                &cfg_file,
                r#"{"supabaseUrl":"https://file.example.com","supabaseAnonKey":"key","powersyncUrl":"https://ps.example.com","apiUrl":"https://api.example.com/v1"}"#,
            )
            .unwrap();
            unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.path()) };
            unsafe {
                std::env::set_var(
                    "XDG_DATA_HOME",
                    std::env::temp_dir()
                        .join("flicknote-test-data2")
                        .to_str()
                        .unwrap(),
                )
            };
            let cfg = Config::load().expect("Config::load should succeed");
            assert_eq!(cfg.supabase_url, "https://file.example.com");
        });
    }
}
