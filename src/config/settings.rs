use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub api_port: u16,
    pub default_printer: String,
    pub auto_startup: bool,
    pub log_level: String,
    pub connection_timeout_secs: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_port: 9123,
            default_printer: String::new(),
            auto_startup: false,
            log_level: "info".to_string(),
            connection_timeout_secs: 5,
        }
    }
}

pub struct ConfigManager {
    path: PathBuf,
}

impl ConfigManager {
    pub fn new() -> Self {
        let path = Self::default_config_path();
        Self { path }
    }

    pub fn default_config_path() -> PathBuf {
        let local_path = Path::new("ghitaprint.toml");
        if local_path.exists() {
            return local_path.to_path_buf();
        }

        if let Ok(app_data) = std::env::var("APPDATA") {
            let mut dir = PathBuf::from(app_data);
            dir.push("GhitaPrint");
            let _ = fs::create_dir_all(&dir);
            dir.push("ghitaprint.toml");
            return dir;
        }

        PathBuf::from("ghitaprint.toml")
    }

    pub fn load(&self) -> Settings {
        if !self.path.exists() {
            let default_settings = Settings::default();
            let _ = self.save(&default_settings);
            return default_settings;
        }

        match fs::read_to_string(&self.path) {
            Ok(content) => toml::from_str(&content).unwrap_or_else(|_| Settings::default()),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self, settings: &Settings) -> Result<(), std::io::Error> {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let toml_str =
            toml::to_string_pretty(settings).map_err(|e| std::io::Error::other(e.to_string()))?;
        fs::write(&self.path, toml_str)
    }

    pub fn get_path(&self) -> &Path {
        &self.path
    }
}
