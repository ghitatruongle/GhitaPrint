use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub const STATUS_FILE_NAME: &str = "status.json";
pub const STATUS_JOBS_IN_FILE: usize = 20;
pub const STATUS_DIR_ENV: &str = "APPDATA";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JobSummary {
    pub id: u32,
    pub created_at_ms: u64,
    pub source: String,
    pub target: String,
    pub mode: String,
    pub success: bool,
    pub code: String,
    pub message: String,
    pub bytes_sent: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VirtualPrinterCheck {
    pub port_ok: bool,
    pub raw_port: u16,
    pub checked_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StatusSnapshot {
    pub version: String,
    pub pid: u32,
    pub api_port: u16,
    pub boot_started_at_ms: u64,
    pub ready_at_ms: Option<u64>,
    pub ready_after_ms: Option<u64>,
    pub prints_total: u64,
    pub prints_ok: u64,
    pub prints_failed: u64,
    pub last_error: Option<JobSummary>,
    pub jobs: Vec<JobSummary>,
    pub virtual_printer: Option<VirtualPrinterCheck>,
    pub updated_at_ms: u64,
}

static BOOT_STARTED_MS: AtomicU64 = AtomicU64::new(0);
static READY_AT_MS: AtomicU64 = AtomicU64::new(0);
static API_PORT: AtomicU16 = AtomicU16::new(0);
static STATUS_PATH: OnceLock<PathBuf> = OnceLock::new();
static WRITE_LOCK: Mutex<()> = Mutex::new(());
static VP_CHECK: Mutex<Option<VirtualPrinterCheck>> = Mutex::new(Option::None);

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn mark_boot_started() {
    BOOT_STARTED_MS.compare_exchange(0, now_ms(), Ordering::SeqCst, Ordering::SeqCst).ok();
}

pub fn boot_started_ms() -> u64 {
    BOOT_STARTED_MS.load(Ordering::SeqCst)
}

pub fn mark_ready() {
    READY_AT_MS.compare_exchange(0, now_ms(), Ordering::SeqCst, Ordering::SeqCst).ok();
}

pub fn ready_at_ms() -> Option<u64> {
    let value = READY_AT_MS.load(Ordering::SeqCst);
    (value != 0).then_some(value)
}

pub fn boot_to_ready_ms() -> Option<u64> {
    let boot = boot_started_ms();
    let ready = ready_at_ms()?;
    (boot > 0 && ready >= boot).then(|| ready - boot)
}

pub fn set_api_port(port: u16) {
    API_PORT.store(port, Ordering::SeqCst);
}

pub fn api_port() -> u16 {
    API_PORT.load(Ordering::SeqCst)
}

pub fn status_dir() -> Option<PathBuf> {
    let app_data = std::env::var(STATUS_DIR_ENV).ok()?;
    let dir = PathBuf::from(app_data).join("GhitaPrint");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

pub fn status_file_path() -> Option<PathBuf> {
    STATUS_PATH
        .get()
        .cloned()
        .or_else(|| status_dir().map(|dir| dir.join(STATUS_FILE_NAME)))
}

pub fn init_status_file(path: PathBuf) {
    let _ = STATUS_PATH.set(path);
}

pub fn is_status_initialized() -> bool {
    STATUS_PATH.get().is_some()
}

pub fn record_virtual_printer_check(check: VirtualPrinterCheck) {
    if let Ok(mut slot) = VP_CHECK.lock() {
        *slot = Some(check);
    }
}

pub fn current_virtual_printer_check() -> Option<VirtualPrinterCheck> {
    VP_CHECK.lock().ok().and_then(|slot| slot.clone())
}

fn write_status_to_impl(path: &Path, snapshot: &StatusSnapshot) -> Result<(), String> {
    let tmp_path = path.with_file_name(format!(
        "{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("status"),
        std::process::id()
    ));
    let payload = serde_json::to_string_pretty(snapshot)
        .map_err(|e| format!("Không thể serialize status: {}", e))?;
    std::fs::write(&tmp_path, payload)
        .map_err(|e| format!("Không thể ghi {} : {}", tmp_path.display(), e))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|e| format!("Không thể thay thế {}: {}", path.display(), e))?;
    Ok(())
}

pub fn write_status_to(path: &Path, snapshot: &StatusSnapshot) -> Result<(), String> {
    let guard = WRITE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let result = write_status_to_impl(path, snapshot);
    drop(guard);
    result
}

pub fn write_status(snapshot: &StatusSnapshot) -> Result<(), String> {
    let Some(path) = STATUS_PATH.get() else {
        return Err("status.json chưa được khởi tạo trong ngữ cảnh này — bỏ qua ghi".to_string());
    };
    write_status_to(path, snapshot)
}

pub fn load_status() -> Option<StatusSnapshot> {
    let path = status_file_path()?;
    load_status_from(&path)
}

pub fn load_status_from(path: &Path) -> Option<StatusSnapshot> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_roundtrip_preserves_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status.json");
        let snapshot = StatusSnapshot {
            version: "0.0.3-preview".to_string(),
            pid: 1234,
            api_port: 9123,
            boot_started_at_ms: 1000,
            ready_at_ms: Some(1250),
            ready_after_ms: Some(250),
            prints_total: 7,
            prints_ok: 5,
            prints_failed: 2,
            last_error: Some(JobSummary {
                id: 9,
                created_at_ms: 42,
                source: "rest".to_string(),
                target: "127.0.0.1:9100".to_string(),
                mode: "raw".to_string(),
                success: false,
                code: "transport_error".to_string(),
                message: "Không thể kết nối".to_string(),
                bytes_sent: 0,
            }),
            jobs: Vec::new(),
            virtual_printer: Some(VirtualPrinterCheck {
                port_ok: true,
                raw_port: 9124,
                checked_at_ms: 999,
            }),
            updated_at_ms: 4321,
        };
        let written = write_status_to(&path, &snapshot);
        assert!(written.is_ok(), "ghi status phải thành công: {:?}", written.err());
        let loaded = load_status_from(&path).expect("status.json phải đọc lại được");
        assert_eq!(loaded.version, "0.0.3-preview");
        assert_eq!(loaded.pid, 1234);
        assert_eq!(loaded.api_port, 9123);
        assert_eq!(loaded.ready_after_ms, Some(250));
        assert_eq!(loaded.prints_failed, 2);
        assert_eq!(loaded.last_error.unwrap().id, 9);
        assert!(loaded.virtual_printer.unwrap().port_ok);
        assert_eq!(loaded.updated_at_ms, 4321);
        assert!(!dir.path().join("status.json.tmp").exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_boot_to_ready_requires_both_marks() {
        assert!(boot_to_ready_ms().is_none() || boot_started_ms() > 0);
    }
}
