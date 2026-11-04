use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub api_port: u16,
    #[serde(default)]
    pub default_printer: String,
    #[serde(default)]
    pub auto_startup: bool,
    #[serde(default)]
    pub log_level: String,
    #[serde(default)]
    pub connection_timeout_secs: u64,
    #[serde(default)]
    pub api_token: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_port: 9123,
            default_printer: String::new(),
            auto_startup: false,
            log_level: "info".to_string(),
            connection_timeout_secs: 5,
            api_token: String::new(),
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
        let tmp_path = self.path.with_extension("toml.tmp");
        fs::write(&tmp_path, toml_str)?;
        match fs::rename(&tmp_path, &self.path) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                Err(e)
            }
        }
    }

    pub fn get_path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config_without_api_token_backcompat() {
        let content = "api_port = 9123\ndefault_printer = \"usb://0416:5011\"\nauto_startup = true\nlog_level = \"info\"\nconnection_timeout_secs = 5\n";
        let parsed: Settings =
            toml::from_str(content).expect("config cũ thiếu api_token phải parse được");
        assert_eq!(parsed.api_token, "");
        assert_eq!(parsed.api_port, 9123);
    }

    #[test]
    fn test_parse_partial_config_keeps_present_values() {
        let content = "api_port = 9500\n";
        let parsed: Settings = toml::from_str(content).expect("config một phần phải parse được");
        assert_eq!(parsed.api_port, 9500);
        assert_eq!(parsed.default_printer, "");
        assert_eq!(parsed.api_token, "");
    }

    #[test]
    fn test_config_roundtrip_includes_api_token() {
        let settings = Settings {
            api_token: "secret".to_string(),
            ..Settings::default()
        };
        let serialized = toml::to_string_pretty(&settings).unwrap();
        let parsed: Settings = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.api_token, "secret");
    }
}
