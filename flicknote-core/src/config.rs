use std::fs;
use std::path::PathBuf;

pub struct Config {
    pub supabase_url: String,
    pub supabase_anon_key: String,
    pub powersync_url: String,
    pub api_url: String,
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

        if config_file.exists() {
            if let Ok(raw) = fs::read_to_string(&config_file) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
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
                }
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

        let paths = ConfigPaths {
            config_dir,
            data_dir,
            config_file,
            session_file,
            db_file,
            log_file,
        };

        Ok(Config {
            supabase_url,
            supabase_anon_key,
            powersync_url,
            api_url,
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
