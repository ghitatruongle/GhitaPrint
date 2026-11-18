use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::os::process::{process_image_name, spawn_detached};
use crate::status::{self, JobSummary, StatusSnapshot};

const WATCH_APP_LABEL: &str = "GhitaPrint Watch";
const EVENTS_LOG_NAME: &str = "events.log";
const EVENTS_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
const TOAST_THROTTLE_MS: u64 = 3_000;
const TOAST_MAX_LIFETIME_SECS: u64 = 5;
const WATCH_PID_NAME: &str = "watch.pid";
const STOP_MARKER_NAME: &str = "daemon.stopped";
const RELAUNCH_COOLDOWN_MS: u64 = 60_000;
const RELAUNCH_WINDOW_MS: u64 = 10 * 60_000;
const RELAUNCH_MAX_ATTEMPTS: u32 = 5;

static LAST_TOAST_MS: AtomicU64 = AtomicU64::new(0);
static LAST_DAEMON_TOAST_MS: AtomicU64 = AtomicU64::new(0);

pub fn watch_pid_file() -> Option<PathBuf> {
    status::status_dir().map(|dir| dir.join(WATCH_PID_NAME))
}

fn stop_marker_file() -> Option<PathBuf> {
    status::status_dir().map(|dir| dir.join(STOP_MARKER_NAME))
}

pub fn read_watch_pid() -> Option<u32> {
    let path = watch_pid_file()?;
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
}

pub fn is_watch_pid_live(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    if pid == std::process::id() {
        return true;
    }
    process_image_name(pid)
        .map(|image| {
            let name = Path::new(&image)
                .file_name()
                .map(|value| value.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            name == "ghitaprint.exe" || name == "watch.exe"
        })
        .unwrap_or(false)
}

pub fn is_watch_running() -> bool {
    read_watch_pid().map(is_watch_pid_live).unwrap_or(false)
}

pub fn clear_stale_watch_pid(pid: u32) {
    if read_watch_pid() == Some(pid)
        && let Some(path) = watch_pid_file()
    {
        let _ = std::fs::remove_file(path);
    }
}

fn write_watch_pid() {
    if let Some(path) = watch_pid_file() {
        let _ = std::fs::write(path, std::process::id().to_string());
    }
}

fn remove_watch_pid() {
    if let Some(path) = watch_pid_file() {
        let _ = std::fs::remove_file(path);
    }
}

pub fn write_stop_marker() {
    if let Some(path) = stop_marker_file() {
        let _ = std::fs::write(path, status::now_ms().to_string());
    }
}

pub fn clear_stop_marker() {
    if let Some(path) = stop_marker_file() {
        let _ = std::fs::remove_file(path);
    }
}

fn stop_requested() -> bool {
    stop_marker_file()
        .map(|path| path.exists())
        .unwrap_or(false)
}

pub fn clear_markers_for_install() {
    clear_stop_marker();
    if let Some(pid) = read_watch_pid()
        && !is_watch_pid_live(pid)
    {
        clear_stale_watch_pid(pid);
    }
}

pub fn prune_old_events(app_dir: &Path) {
    for name in ["events.log.old", "events.log"] {
        let path = app_dir.join(name);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() <= EVENTS_LOG_MAX_BYTES {
            continue;
        }
        if name == "events.log" {
            let _ = std::fs::rename(&path, app_dir.join("events.log.old"));
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn daemon_exe_path() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    let name = current
        .file_name()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name == "watch.exe" {
        let sibling = current.with_file_name("ghitaprint.exe");
        return sibling.exists().then_some(sibling);
    }
    Some(current)
}

pub fn run_watch(interval_ms: u64, no_toast: bool, once: bool) {
    run_watch_with_hidden(interval_ms, no_toast, once, false);
}

pub fn run_watch_with_hidden(interval_ms: u64, no_toast: bool, once: bool, hidden: bool) {
    if hidden && !once {
        if spawn_hidden_watch(interval_ms, no_toast) {
            return;
        }
        eprintln!("✖ Không thể re-exec watch ẩn — chạy trực tiếp ở cửa sổ hiện tại.");
    }
    run_watch_loop(interval_ms, no_toast, once);
}

pub fn spawn_hidden_watch(interval_ms: u64, no_toast: bool) -> bool {
    let Ok(exe_path) = std::env::current_exe() else {
        return false;
    };
    spawn_hidden_watch_at(&exe_path, interval_ms, no_toast)
}

pub fn spawn_hidden_watch_at(exe_path: &Path, interval_ms: u64, no_toast: bool) -> bool {
    let current_name = exe_path
        .file_name()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let interval = interval_ms.max(100).to_string();
    let mut arguments: Vec<&str> = if current_name == "watch.exe" {
        Vec::new()
    } else {
        vec!["watch"]
    };
    arguments.push("--interval-ms");
    arguments.push(&interval);
    if no_toast {
        arguments.push("--no-toast");
    }
    spawn_detached(exe_path, &arguments).is_ok()
}

fn run_watch_loop(interval_ms: u64, no_toast: bool, once: bool) {
    let interval = Duration::from_millis(interval_ms.max(100));
    let Some(status_path) = status::status_file_path() else {
        eprintln!("✖ Không xác định được thư mục status (APPDATA) — watch dừng.");
        return;
    };
    let Some(events_path) = status::status_dir().map(|dir| dir.join(EVENTS_LOG_NAME)) else {
        eprintln!("✖ Không xác định được thư mục events (APPDATA) — watch dừng.");
        return;
    };

    if !once {
        write_watch_pid();
    }

    let mut runtime = WatchRuntime {
        events_path,
        last_mtime: None,
        last_seen_job_id: 0,
        first_snapshot_seen: false,
        daemon_pid: 0,
        daemon_dead_announced: false,
        missing_announced: false,
        last_snapshot: None,
        relaunch_window_start_ms: 0,
        relaunch_attempts: 0,
        last_relaunch_ms: 0,
    };
    append_event(
        &runtime.events_path,
        &format!(
            "{} bắt đầu theo dõi status.json (chu kỳ {} ms, toast {})",
            WATCH_APP_LABEL,
            interval.as_millis(),
            if no_toast { "TẮT" } else { "BẬT" }
        ),
    );

    loop {
        if let Ok(meta) = std::fs::metadata(&status_path) {
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if runtime.last_mtime.map(|seen| seen != mtime).unwrap_or(true) {
                runtime.last_mtime = Some(mtime);
                process_snapshot(&mut runtime, &status_path, no_toast);
            }
        } else if !runtime.missing_announced {
            runtime.missing_announced = true;
            append_event(
                &runtime.events_path,
                "Chưa thấy status.json — chờ daemon ghi trạng thái đầu tiên",
            );
        }
        if let Some(snapshot) = runtime.last_snapshot.clone() {
            watch_daemon_liveness(&mut runtime, &snapshot, no_toast);
        }
        if once {
            break;
        }
        std::thread::sleep(interval);
    }

    if !once {
        remove_watch_pid();
    }
}

struct WatchRuntime {
    events_path: PathBuf,
    last_mtime: Option<SystemTime>,
    last_seen_job_id: u32,
    first_snapshot_seen: bool,
    daemon_pid: u32,
    daemon_dead_announced: bool,
    missing_announced: bool,
    last_snapshot: Option<StatusSnapshot>,
    relaunch_window_start_ms: u64,
    relaunch_attempts: u32,
    last_relaunch_ms: u64,
}

fn process_snapshot(runtime: &mut WatchRuntime, status_path: &Path, no_toast: bool) {
    let Some(snapshot) = status::load_status_from(status_path) else {
        return;
    };
    if !runtime.first_snapshot_seen {
        runtime.first_snapshot_seen = true;
        runtime.last_seen_job_id = snapshot.jobs.iter().map(|job| job.id).max().unwrap_or(0);
    }
    if runtime.daemon_pid == 0 {
        runtime.daemon_pid = snapshot.pid;
        append_event(
            &runtime.events_path,
            &format!(
                "Daemon đang chạy (PID {}, v{}, port {})",
                snapshot.pid, snapshot.version, snapshot.api_port
            ),
        );
    } else if runtime.daemon_pid != snapshot.pid {
        append_event(
            &runtime.events_path,
            &format!(
                "Daemon mới khởi động (PID {} thay cho PID {})",
                snapshot.pid, runtime.daemon_pid
            ),
        );
        runtime.daemon_pid = snapshot.pid;
        runtime.daemon_dead_announced = false;
        runtime.last_seen_job_id = snapshot.jobs.iter().map(|job| job.id).max().unwrap_or(0);
    }

    let mut new_jobs = snapshot
        .jobs
        .iter()
        .filter(|job| job.id > runtime.last_seen_job_id)
        .cloned()
        .collect::<Vec<JobSummary>>();
    new_jobs.sort_by_key(|job| job.id);
    if let Some(latest) = new_jobs.last() {
        runtime.last_seen_job_id = latest.id;
    }
    for job in new_jobs {
        let line = format_job_event(&job);
        append_event(&runtime.events_path, &line);
        if !job.success && !no_toast && toast_throttle_gate(&LAST_TOAST_MS) {
            show_failure_toast(&format_job_toast(&job));
        }
    }
    runtime.last_snapshot = Some(snapshot);
}

fn watch_daemon_liveness(runtime: &mut WatchRuntime, snapshot: &StatusSnapshot, no_toast: bool) {
    #[cfg(not(windows))]
    {
        let _ = (runtime, snapshot, no_toast);
    }
    #[cfg(windows)]
    {
        if runtime.daemon_pid != snapshot.pid {
            return;
        }
        let alive = process_image_name(snapshot.pid)
            .map(|image| {
                std::path::Path::new(&image)
                    .file_name()
                    .map(|name| name.eq_ignore_ascii_case("ghitaprint.exe"))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if alive {
            runtime.daemon_dead_announced = false;
            return;
        }
        if stop_requested() {
            return;
        }
        if !runtime.daemon_dead_announced {
            runtime.daemon_dead_announced = true;
            append_event(
                &runtime.events_path,
                &format!("Daemon không còn sống (PID {})", snapshot.pid),
            );
            if !no_toast && toast_throttle_gate(&LAST_DAEMON_TOAST_MS) {
                show_failure_toast("Daemon GhitaPrint đã dừng — đang tự khởi động lại");
            }
        }
        if let Some(exe) = daemon_exe_path() {
            try_relaunch_daemon(runtime, &exe);
        }
    }
}

fn try_relaunch_daemon(runtime: &mut WatchRuntime, exe: &Path) -> bool {
    let now = status::now_ms();
    if runtime.last_relaunch_ms != 0
        && now.saturating_sub(runtime.last_relaunch_ms) < RELAUNCH_COOLDOWN_MS
    {
        return false;
    }
    if runtime.relaunch_window_start_ms == 0
        || now.saturating_sub(runtime.relaunch_window_start_ms) > RELAUNCH_WINDOW_MS
    {
        runtime.relaunch_window_start_ms = now;
        runtime.relaunch_attempts = 0;
    }
    if runtime.relaunch_attempts >= RELAUNCH_MAX_ATTEMPTS {
        return false;
    }
    runtime.last_relaunch_ms = now;
    runtime.relaunch_attempts += 1;
    match spawn_detached(exe, &["daemon", "--start"]) {
        Ok(pid) => {
            append_event(
                &runtime.events_path,
                &format!(
                    "Đã tự khởi động lại daemon (PID {}, lần {}/{})",
                    pid, runtime.relaunch_attempts, RELAUNCH_MAX_ATTEMPTS
                ),
            );
            true
        }
        Err(error) => {
            append_event(
                &runtime.events_path,
                &format!("Không thể tự khởi động lại daemon: {}", error),
            );
            false
        }
    }
}

fn format_job_event(job: &JobSummary) -> String {
    format!(
        "{} #{} {} {} bytes → {} (mode {}, nguồn {})",
        if job.success {
            "✔ In thành công"
        } else {
            "✖ In thất bại"
        },
        job.id,
        job.code,
        job.bytes_sent,
        if job.target.is_empty() {
            "(không target)"
        } else {
            job.target.as_str()
        },
        job.mode,
        job.source
    )
}

fn format_job_toast(job: &JobSummary) -> String {
    format!(
        "In thất bại #{} → {}: {}",
        job.id,
        if job.target.is_empty() {
            "(không target)"
        } else {
            job.target.as_str()
        },
        truncate_message(&job.message, 160)
    )
}

fn truncate_message(message: &str, max_chars: usize) -> String {
    if message.chars().count() <= max_chars {
        return message.to_string();
    }
    let cut: String = message.chars().take(max_chars).collect();
    format!("{}…", cut)
}

fn toast_throttle_gate(slot: &AtomicU64) -> bool {
    let now = status::now_ms();
    let last = slot.load(Ordering::SeqCst);
    if now.saturating_sub(last) < TOAST_THROTTLE_MS {
        return false;
    }
    slot.store(now, Ordering::SeqCst);
    true
}

fn append_event(events_path: &Path, line: &str) {
    if let Ok(meta) = std::fs::metadata(events_path)
        && meta.len() > EVENTS_LOG_MAX_BYTES
    {
        let _ = std::fs::rename(events_path, events_path.with_extension("log.old"));
    }
    let timestamp = format_local_timestamp();
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(events_path)
    {
        let _ = writeln!(file, "[{}] {}", timestamp, line);
    }
}

fn format_local_timestamp() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SYSTEMTIME;
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
        unsafe { GetLocalTime(&mut time) };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
        )
    }
    #[cfg(not(windows))]
    {
        let seconds = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let (year, month, day, hour, minute, second) = civil_from_unix(seconds as i64);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            year, month, day, hour, minute, second
        )
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn civil_from_unix(seconds: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = seconds.div_euclid(86_400);
    let rem = seconds.rem_euclid(86_400);
    let hour = (rem / 3_600) as u32;
    let minute = ((rem % 3_600) / 60) as u32;
    let second = (rem % 60) as u32;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day, hour, minute, second)
}

#[cfg(windows)]
fn show_failure_toast(message: &str) {
    use std::os::windows::process::CommandExt;

    const TOAST_APP_ID: &str =
        "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";
    let safe_message = sanitize_toast_message(message);
    let script = format!(
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; \
         $xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
         $texts = $xml.GetElementsByTagName('text'); \
         $texts.Item(0).AppendChild($xml.CreateTextNode('{TITLE}')) | Out-Null; \
         $texts.Item(1).AppendChild($xml.CreateTextNode('{MSG}')) | Out-Null; \
         $toast = [Windows.UI.Notifications.ToastNotification]::new($xml); \
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{APPID}').Show($toast)",
        TITLE = WATCH_APP_LABEL,
        MSG = safe_message,
        APPID = TOAST_APP_ID
    );
    let spawned = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000)
        .spawn();
    let Ok(mut child) = spawned else {
        return;
    };
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(TOAST_MAX_LIFETIME_SECS);
        loop {
            match child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) => {}
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
}

#[cfg(not(windows))]
fn show_failure_toast(_message: &str) {}

fn sanitize_toast_message(message: &str) -> String {
    let without_quotes = message.replace('\'', "''");
    without_quotes.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_job_event_success_and_failure() {
        let job = JobSummary {
            id: 7,
            created_at_ms: 42,
            source: "rest".to_string(),
            target: "127.0.0.1:9100".to_string(),
            mode: "raw".to_string(),
            success: true,
            code: "ok".to_string(),
            message: "Đã in thành công".to_string(),
            bytes_sent: 512,
        };
        let line = format_job_event(&job);
        assert!(line.contains("✔ In thành công"));
        assert!(line.contains("#7"));
        assert!(line.contains("512 bytes"));
        assert!(line.contains("127.0.0.1:9100"));
        let failed = JobSummary {
            success: false,
            code: "transport_error".to_string(),
            ..job
        };
        assert!(format_job_event(&failed).contains("✖ In thất bại"));
    }

    #[test]
    fn test_format_job_event_empty_target_placeholder() {
        let job = JobSummary {
            id: 1,
            created_at_ms: 1,
            source: "ipp".to_string(),
            target: String::new(),
            mode: "ipp".to_string(),
            success: false,
            code: "transport_error".to_string(),
            message: "x".to_string(),
            bytes_sent: 0,
        };
        assert!(format_job_event(&job).contains("(không target)"));
    }

    #[test]
    fn test_truncate_message_caps_length() {
        let long = "a".repeat(300);
        let cut = truncate_message(&long, 160);
        assert!(cut.chars().count() <= 161);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn test_civil_from_unix_known_dates() {
        assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil_from_unix(1_700_000_000), (2023, 11, 14, 22, 13, 20));
    }

    #[test]
    fn test_sanitize_toast_message_escapes_quotes_and_newlines() {
        assert_eq!(sanitize_toast_message("it's 'ok'"), "it''s ''ok''");
        assert_eq!(sanitize_toast_message("a\r\nb"), "a  b");
    }

    #[test]
    fn test_new_job_filter_uses_monotonic_id() {
        let snapshot_jobs = [
            JobSummary {
                id: 10,
                ..Default::default()
            },
            JobSummary {
                id: 12,
                ..Default::default()
            },
        ];
        let seen = 11u32;
        let filtered: Vec<u32> = snapshot_jobs
            .iter()
            .filter(|job| job.id > seen)
            .map(|job| job.id)
            .collect();
        assert_eq!(filtered, vec![12]);
    }

    #[test]
    fn test_relaunch_cooldown_constants_are_sane() {
        const {
            assert!(RELAUNCH_COOLDOWN_MS >= 60_000);
            assert!(RELAUNCH_WINDOW_MS >= RELAUNCH_COOLDOWN_MS);
            assert!(RELAUNCH_MAX_ATTEMPTS >= 3);
            assert!(TOAST_MAX_LIFETIME_SECS <= 30);
        }
    }

    #[test]
    fn test_is_watch_pid_live_rejects_zero() {
        assert!(!is_watch_pid_live(0));
        assert!(is_watch_pid_live(std::process::id()));
    }

    #[test]
    fn test_prune_old_events_rotates_oversized_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");
        std::fs::write(&path, vec![b'x'; EVENTS_LOG_MAX_BYTES as usize + 1]).unwrap();

        prune_old_events(dir.path());

        assert!(!path.exists());
        assert!(dir.path().join("events.log.old").exists());
    }

    #[test]
    fn test_prune_old_events_drops_oversized_rotated_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log.old");
        std::fs::write(&path, vec![b'x'; EVENTS_LOG_MAX_BYTES as usize + 1]).unwrap();

        prune_old_events(dir.path());

        assert!(!path.exists());
    }

    #[test]
    fn test_prune_old_events_keeps_small_logs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");
        std::fs::write(&path, b"small").unwrap();

        prune_old_events(dir.path());

        assert_eq!(std::fs::read(&path).unwrap(), b"small");
    }
}
