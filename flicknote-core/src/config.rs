use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ConfigFileValues {
    supabase_url: String,
    supabase_anon_key: String,
    powersync_url: String,
    api_url: String,
    gateway_url: String,
    web_url: Option<String>,
}

struct EndpointDefaults {
    supabase_url: &'static str,
    supabase_anon_key: &'static str,
    powersync_url: &'static str,
    api_url: &'static str,
    gateway_url: &'static str,
}

#[derive(Clone)]
pub struct Config {
    pub supabase_url: String,
    pub supabase_anon_key: String,
    pub powersync_url: String,
    pub api_url: String,
    pub gateway_url: String,
    pub web_url: Option<String>,
    pub paths: ConfigPaths,
}

#[derive(Clone)]
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

        let ConfigFileValues {
            mut supabase_url,
            mut supabase_anon_key,
            mut powersync_url,
            mut api_url,
            mut gateway_url,
            mut web_url,
        } = read_config_file(&config_file);

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
        if let Ok(v) = std::env::var("FLICKNOTE_GATEWAY_URL") {
            gateway_url = v;
        }
        if let Ok(v) = std::env::var("FLICKNOTE_WEB_URL") {
            web_url = Some(v);
        }

        if api_url.is_empty() != gateway_url.is_empty() {
            return Err(crate::error::CliError::Other(
                "apiUrl and gatewayUrl must be configured together — set both in config.json or via FLICKNOTE_API_URL and FLICKNOTE_GATEWAY_URL".into(),
            ));
        }

        // Fallback: per-field built-in defaults if nothing else configured that field.
        // Each field is guarded independently so a user can override just one env var
        // (e.g. FLICKNOTE_POWERSYNC_URL) without losing their custom value when
        // other fields fall back to the built-in set.
        if supabase_url.is_empty()
            || supabase_anon_key.is_empty()
            || powersync_url.is_empty()
            || api_url.is_empty()
            || gateway_url.is_empty()
        {
            let env = std::env::var("FLICKNOTE_ENV").unwrap_or_else(|_| "dev".into());
            let defaults = builtin_defaults(&env);
            if supabase_url.is_empty() {
                supabase_url = defaults.supabase_url.into();
            }
            if supabase_anon_key.is_empty() {
                supabase_anon_key = defaults.supabase_anon_key.into();
            }
            if powersync_url.is_empty() {
                powersync_url = defaults.powersync_url.into();
            }
            if api_url.is_empty() {
                api_url = defaults.api_url.into();
            }
            if gateway_url.is_empty() {
                gateway_url = defaults.gateway_url.into();
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
            gateway_url,
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

    /// Validate that gateway_url is set. Call before Gateway operations.
    pub fn validate_gateway(&self) -> Result<(), crate::error::CliError> {
        if self.gateway_url.is_empty() {
            return Err(crate::error::CliError::Other(
                "gatewayUrl is not configured — set it in config.json or FLICKNOTE_GATEWAY_URL"
                    .into(),
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

fn read_config_file(config_file: &Path) -> ConfigFileValues {
    fs::read_to_string(config_file)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn builtin_defaults(env: &str) -> EndpointDefaults {
    let (supabase_url, supabase_anon_key, powersync_url, api_url, gateway_url) = match env {
        "prod" => (
            "https://auth.flicknote.app",
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImFocGNqYW1maGJpb3BqZG5laW5uIiwicm9sZSI6ImFub24iLCJpYXQiOjE3NTA0NTc1NDIsImV4cCI6MjA2NjAzMzU0Mn0.g6B2UohS8Zw_mrsDljAB7n6feUTvpmMVvvsf7VMRXA4",
            "https://sync.flicknote.app",
            "https://api.flicknote.app/api/v1",
            "https://gw.flicknote.app",
        ),
        _ => (
            "https://dev-auth.flicknote.app",
            "sb_publishable_4VEs5DX9YlkHuViFbmRMQb_f_LPrdOR",
            "https://dev-sync.flicknote.app",
            "https://dev-api.flicknote.app/api/v1",
            "https://dev-gw.flicknote.app",
        ),
    };
    EndpointDefaults {
        supabase_url,
        supabase_anon_key,
        powersync_url,
        api_url,
        gateway_url,
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
            "FLICKNOTE_GATEWAY_URL",
            "FLICKNOTE_WEB_URL",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
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
        let defaults = builtin_defaults("dev");
        assert_eq!(defaults.supabase_url, "https://dev-auth.flicknote.app");
        assert_eq!(defaults.powersync_url, "https://dev-sync.flicknote.app");
        assert_eq!(defaults.api_url, "https://dev-api.flicknote.app/api/v1");
        assert_eq!(defaults.gateway_url, "https://dev-gw.flicknote.app");
        assert_eq!(
            defaults.supabase_anon_key,
            "sb_publishable_4VEs5DX9YlkHuViFbmRMQb_f_LPrdOR"
        );
    }

    #[test]
    fn test_builtin_defaults_prod() {
        let defaults = builtin_defaults("prod");
        assert_eq!(defaults.supabase_url, "https://auth.flicknote.app");
        assert_eq!(defaults.powersync_url, "https://sync.flicknote.app");
        assert_eq!(defaults.api_url, "https://api.flicknote.app/api/v1");
        assert_eq!(defaults.gateway_url, "https://gw.flicknote.app");
        assert!(!defaults.supabase_anon_key.is_empty());
    }

    #[test]
    fn test_builtin_defaults_unknown_falls_back_to_dev() {
        let defaults = builtin_defaults("staging");
        assert_eq!(defaults.supabase_url, "https://dev-auth.flicknote.app");
    }

    #[test]
    fn test_env_var_overrides_builtin() {
        with_clean_env(None, || {
            unsafe { std::env::set_var("FLICKNOTE_SUPABASE_URL", "https://custom.example.com") };
            unsafe { std::env::set_var("FLICKNOTE_SUPABASE_KEY", "custom-publishable-key") };
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
            assert_eq!(cfg.supabase_anon_key, "custom-publishable-key");
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
                r#"{"supabaseUrl":"https://file.example.com","supabaseAnonKey":"key","powersyncUrl":"https://ps.example.com","apiUrl":"https://api.example.com/v1","gatewayUrl":"https://gateway.example.com"}"#,
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
            assert_eq!(cfg.supabase_anon_key, "key");
            assert_eq!(cfg.api_url, "https://api.example.com/v1");
            assert_eq!(cfg.gateway_url, "https://gateway.example.com");
        });
    }

    #[test]
    fn test_config_file_rejects_an_unpaired_endpoint_url() {
        with_clean_env(None, || {
            let tmp = tempfile::tempdir().expect("tempdir");
            let cfg_dir = tmp.path().join("flicknote");
            std::fs::create_dir_all(&cfg_dir).unwrap();
            std::fs::write(
                cfg_dir.join("config.json"),
                r#"{"apiUrl":"https://api.example.com/v1"}"#,
            )
            .unwrap();
            unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.path()) };
            unsafe { std::env::set_var("XDG_DATA_HOME", tmp.path().join("data")) };

            let error = match Config::load() {
                Ok(_) => panic!("unpaired endpoints must be rejected"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("must be configured together"));
        });
    }
}
