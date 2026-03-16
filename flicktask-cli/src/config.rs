use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct FlicktaskConfig {
    #[serde(default)]
    pub(crate) uda: HashMap<String, UdaDef>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UdaDef {
    pub(crate) label: String,
}

impl FlicktaskConfig {
    pub(crate) fn load() -> Self {
        let Some(config_dir) = dirs::config_dir() else {
            return Self::default();
        };
        let config_file = config_dir.join("flicktask").join("config.toml");
        let Ok(content) = std::fs::read_to_string(&config_file) else {
            return Self::default();
        };
        toml::from_str(&content).unwrap_or_else(|e| {
            eprintln!(
                "Warning: failed to parse config at {}: {e}",
                config_file.display()
            );
            Self::default()
        })
    }

    pub(crate) fn uda_label<'a>(&'a self, key: &'a str) -> &'a str {
        self.uda.get(key).map(|d| d.label.as_str()).unwrap_or(key)
    }
}
