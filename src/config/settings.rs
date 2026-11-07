use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

fn default_api_port() -> u16 {
    9123
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_connection_timeout_secs() -> u64 {
    5
}

fn default_protocol() -> String {
    "escpos".to_string()
}

fn default_diacritics() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_api_port")]
    pub api_port: u16,
    #[serde(default)]
    pub default_printer: String,
    #[serde(default)]
    pub auto_startup: bool,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_connection_timeout_secs")]
    pub connection_timeout_secs: u64,
    #[serde(default)]
    pub api_token: String,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default = "default_diacritics")]
    pub diacritics: String,
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
            protocol: "escpos".to_string(),
            diacritics: "auto".to_string(),
        }
    }
}

impl Settings {
    pub fn normalized(mut self) -> Self {
        if self.api_port == 0 {
            warn!("api_port = 0 không hợp lệ — dùng mặc định 9123");
            self.api_port = 9123;
        }
        if self.connection_timeout_secs == 0 {
            warn!("connection_timeout_secs = 0 không hợp lệ — dùng mặc định 5 giây");
            self.connection_timeout_secs = 5;
        }
        if !["escpos", "tspl"].contains(&self.protocol.as_str()) {
            warn!(
                "protocol '{}' không hợp lệ (escpos|tspl) — dùng mặc định escpos",
                self.protocol
            );
            self.protocol = "escpos".to_string();
        }
        if !["auto", "keep", "strip", "raw"].contains(&self.diacritics.as_str()) {
            warn!(
                "diacritics '{}' không hợp lệ (auto|keep|strip|raw) — dùng mặc định auto",
                self.diacritics
            );
            self.diacritics = "auto".to_string();
        }
        self
    }

    pub fn validate_key(key: &str, value: &str) -> Result<(), String> {
        match key {
            "api_port" => {
                let port: u16 = value
                    .parse()
                    .map_err(|_| "Giá trị api_port phải là số nguyên (ví dụ: 9123)".to_string())?;
                if port == 0 {
                    return Err("api_port phải từ 1 tới 65535".to_string());
                }
                Ok(())
            }
            "connection_timeout_secs" => {
                let secs: u64 = value
                    .parse()
                    .map_err(|_| "Giá trị connection_timeout_secs phải là số nguyên".to_string())?;
                if secs == 0 {
                    return Err("connection_timeout_secs phải ít nhất 1 giây".to_string());
                }
                Ok(())
            }
            "auto_startup" => {
                if !["true", "false", "1", "0"].contains(&value.to_lowercase().as_str()) {
                    return Err("Giá trị auto_startup phải là true hoặc false".to_string());
                }
                Ok(())
            }
            "log_level" => {
                if !["trace", "debug", "info", "warn", "error"]
                    .contains(&value.to_lowercase().as_str())
                {
                    return Err(
                        "log_level phải là một trong: trace, debug, info, warn, error".to_string(),
                    );
                }
                Ok(())
            }
            "protocol" => {
                if !["escpos", "tspl"].contains(&value) {
                    return Err("protocol phải là 'escpos' hoặc 'tspl'".to_string());
                }
                Ok(())
            }
            "diacritics" => {
                if !["auto", "keep", "strip", "raw"].contains(&value) {
                    return Err("diacritics phải là một trong: auto, keep, strip, raw".to_string());
                }
                Ok(())
            }
            "default_printer" | "api_token" => Ok(()),
            _ => Err(format!("Không hỗ trợ khóa cấu hình '{}'", key)),
        }
    }

    pub fn masked_token(&self) -> String {
        if self.api_token.is_empty() {
            String::new()
        } else {
            "••••••••".to_string()
        }
    }
}

pub struct ConfigManager {
    path: PathBuf,
    cwd_override: bool,
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigManager {
    pub fn new() -> Self {
        let local_path = Path::new("ghitaprint.toml");
        if local_path.exists() {
            warn!(
                "Đang dùng ghitaprint.toml trong thư mục hiện tại ({}) đè lên cấu hình tại %APPDATA%\\GhitaPrint",
                std::env::current_dir()
                    .map(|d| d.join("ghitaprint.toml").display().to_string())
                    .unwrap_or_else(|_| "ghitaprint.toml".to_string())
            );
            return Self {
                path: local_path.to_path_buf(),
                cwd_override: true,
            };
        }

        if let Ok(app_data) = std::env::var("APPDATA") {
            let dir = PathBuf::from(app_data).join("GhitaPrint");
            let _ = fs::create_dir_all(&dir);
            return Self {
                path: dir.join("ghitaprint.toml"),
                cwd_override: false,
            };
        }

        warn!("Biến môi trường APPDATA không tồn tại — ghi cấu hình vào thư mục hiện tại");
        Self {
            path: PathBuf::from("ghitaprint.toml"),
            cwd_override: false,
        }
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self {
            path,
            cwd_override: false,
        }
    }

    pub fn is_cwd_override(&self) -> bool {
        self.cwd_override
    }

    fn backup_path(&self) -> PathBuf {
        self.path.with_extension("toml.bak")
    }

    fn tmp_path(&self) -> PathBuf {
        self.path.with_extension("toml.tmp")
    }

    pub fn load(&self) -> Settings {
        let _ = fs::remove_file(self.tmp_path());

        if !self.path.exists() {
            return Settings::default().normalized();
        }

        let content = match fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(e) => {
                warn!(
                    "Không thể đọc cấu hình {}: {} — dùng cấu hình mặc định",
                    self.path.display(),
                    e
                );
                return Settings::default().normalized();
            }
        };

        match toml::from_str::<Settings>(&content) {
            Ok(settings) => settings.normalized(),
            Err(parse_error) => {
                warn!(
                    "Cấu hình {} hỏng ({}). Đang thử file backup...",
                    self.path.display(),
                    parse_error
                );
                match fs::read_to_string(self.backup_path()) {
                    Ok(backup) => match toml::from_str::<Settings>(&backup) {
                        Ok(settings) => {
                            warn!(
                                "Đã khôi phục cấu hình từ backup {}. Nên chạy 'ghitaprint config set api_port <port>' để ghi lại file sạch.",
                                self.backup_path().display()
                            );
                            return settings.normalized();
                        }
                        Err(_) => warn!("Backup cấu hình cũng hỏng — dùng cấu hình mặc định"),
                    },
                    Err(_) => warn!("Không có backup cấu hình — dùng cấu hình mặc định"),
                }
                Settings::default().normalized()
            }
        }
    }

    pub fn save(&self, settings: &Settings) -> Result<(), std::io::Error> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let toml_str =
            toml::to_string_pretty(settings).map_err(|e| std::io::Error::other(e.to_string()))?;

        let tmp_path = self.tmp_path();
        fs::write(&tmp_path, &toml_str)?;

        if self.path.exists() {
            let _ = fs::copy(&self.path, self.backup_path());
        }

        fs::rename(&tmp_path, &self.path).inspect_err(|_| {
            let _ = fs::remove_file(&tmp_path);
        })?;
        let _ = fs::copy(&self.path, self.backup_path());
        Ok(())
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
    fn test_parse_partial_config_uses_field_defaults_not_zero() {
        let content = "default_printer = \"usb://0416:5011\"\n";
        let parsed: Settings = toml::from_str(content).expect("config một phần phải parse được");
        assert_eq!(parsed.api_port, 9123);
        assert_eq!(parsed.connection_timeout_secs, 5);
        assert_eq!(parsed.protocol, "escpos");
        assert_eq!(parsed.diacritics, "auto");
    }

    #[test]
    fn test_config_roundtrip_includes_api_token_and_new_keys() {
        let settings = Settings {
            api_token: "secret".to_string(),
            protocol: "tspl".to_string(),
            diacritics: "strip".to_string(),
            ..Settings::default()
        };
        let serialized = toml::to_string_pretty(&settings).unwrap();
        let parsed: Settings = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.api_token, "secret");
        assert_eq!(parsed.protocol, "tspl");
        assert_eq!(parsed.diacritics, "strip");
    }

    #[test]
    fn test_normalized_fixes_zero_port_and_timeout() {
        let broken = Settings {
            api_port: 0,
            connection_timeout_secs: 0,
            ..Settings::default()
        };
        let fixed = broken.normalized();
        assert_eq!(fixed.api_port, 9123);
        assert_eq!(fixed.connection_timeout_secs, 5);
    }

    #[test]
    fn test_normalized_fixes_invalid_protocol_and_diacritics() {
        let broken = Settings {
            protocol: "ipp-only".to_string(),
            diacritics: "emoji".to_string(),
            ..Settings::default()
        };
        let fixed = broken.normalized();
        assert_eq!(fixed.protocol, "escpos");
        assert_eq!(fixed.diacritics, "auto");
    }

    #[test]
    fn test_validate_key_ranges() {
        assert!(Settings::validate_key("api_port", "0").is_err());
        assert!(Settings::validate_key("api_port", "70000").is_err());
        assert!(Settings::validate_key("api_port", "9123").is_ok());
        assert!(Settings::validate_key("connection_timeout_secs", "0").is_err());
        assert!(Settings::validate_key("auto_startup", "TRUE").is_ok());
        assert!(Settings::validate_key("log_level", "verbose").is_err());
        assert!(Settings::validate_key("protocol", "escpos").is_ok());
        assert!(Settings::validate_key("diacritics", "strip").is_ok());
        assert!(Settings::validate_key("unknown_key", "1").is_err());
    }

    #[test]
    fn test_load_corrupt_config_falls_back_to_backup() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ghitaprint.toml");
        fs::write(&config_path, "api_port = 9500@@@hỏng").unwrap();
        fs::write(dir.path().join("ghitaprint.toml.bak"), "api_port = 9600\n").unwrap();

        let mgr = ConfigManager::with_path(config_path);
        let settings = mgr.load();
        assert_eq!(settings.api_port, 9600);
    }

    #[test]
    fn test_load_corrupt_config_without_backup_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ghitaprint.toml");
        fs::write(&config_path, "totally [[ corrupt").unwrap();

        let mgr = ConfigManager::with_path(config_path);
        let settings = mgr.load();
        assert_eq!(settings.api_port, 9123);
        assert_eq!(settings.api_token, "");
    }

    #[test]
    fn test_save_creates_backup_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ghitaprint.toml");
        let mgr = ConfigManager::with_path(config_path);

        let settings = Settings {
            api_token: "keepme".to_string(),
            api_port: 9300,
            ..Settings::default()
        };
        mgr.save(&settings).unwrap();

        assert!(dir.path().join("ghitaprint.toml.bak").exists());

        let mut next = settings.clone();
        next.api_port = 9400;
        mgr.save(&next).unwrap();

        let reloaded = mgr.load();
        assert_eq!(reloaded.api_port, 9400);
        assert_eq!(reloaded.api_token, "keepme");
    }

    #[test]
    fn test_load_missing_file_does_not_create_it() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ghitaprint.toml");
        let mgr = ConfigManager::with_path(config_path.clone());
        let _ = mgr.load();
        assert!(!config_path.exists());
    }

    #[test]
    fn test_load_removes_stale_tmp_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ghitaprint.toml");
        fs::write(dir.path().join("ghitaprint.toml.tmp"), "rác").unwrap();
        fs::write(&config_path, "api_port = 9200\n").unwrap();

        let mgr = ConfigManager::with_path(config_path);
        let _ = mgr.load();
        assert!(!dir.path().join("ghitaprint.toml.tmp").exists());
    }

    #[test]
    fn test_masked_token() {
        let mut s = Settings::default();
        assert_eq!(s.masked_token(), "");
        s.api_token = "abc".to_string();
        assert_eq!(s.masked_token(), "••••••••");
    }
}
