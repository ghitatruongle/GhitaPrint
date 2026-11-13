use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::os::process::process_image_name;
use crate::status::{self, JobSummary, StatusSnapshot};

const WATCH_APP_LABEL: &str = "GhitaPrint Watch";
const EVENTS_LOG_NAME: &str = "events.log";
const EVENTS_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
const TOAST_THROTTLE_MS: u64 = 3_000;

static LAST_TOAST_MS: AtomicU64 = AtomicU64::new(0);

pub fn run_watch(interval_ms: u64, no_toast: bool, once: bool) {
    let interval = Duration::from_millis(interval_ms.max(100));
    let Some(status_path) = status::status_file_path() else {
        eprintln!("✖ Không xác định được thư mục status (APPDATA) — watch dừng.");
        return;
    };
    let Some(events_path) = status::status_dir().map(|dir| dir.join(EVENTS_LOG_NAME)) else {
        eprintln!("✖ Không xác định được thư mục events (APPDATA) — watch dừng.");
        return;
    };

    let mut runtime = WatchRuntime {
        events_path,
        last_mtime: None,
        last_seen_job_ms: 0,
        first_snapshot_seen: false,
        daemon_pid: 0,
        daemon_dead_announced: false,
        missing_announced: false,
        last_snapshot: None,
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
            watch_daemon_liveness(&mut runtime, &snapshot);
        }
        if once {
            break;
        }
        std::thread::sleep(interval);
    }
}

struct WatchRuntime {
    events_path: PathBuf,
    last_mtime: Option<SystemTime>,
    last_seen_job_ms: u64,
    first_snapshot_seen: bool,
    daemon_pid: u32,
    daemon_dead_announced: bool,
    missing_announced: bool,
    last_snapshot: Option<StatusSnapshot>,
}

fn process_snapshot(runtime: &mut WatchRuntime, status_path: &Path, no_toast: bool) {
    let Some(snapshot) = status::load_status_from(status_path) else {
        return;
    };
    if !runtime.first_snapshot_seen {
        runtime.first_snapshot_seen = true;
        runtime.last_seen_job_ms = snapshot
            .jobs
            .iter()
            .map(|job| job.created_at_ms)
            .max()
            .unwrap_or(0);
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
    }

    let mut new_jobs = snapshot
        .jobs
        .iter()
        .filter(|job| job.created_at_ms > runtime.last_seen_job_ms)
        .cloned()
        .collect::<Vec<JobSummary>>();
    new_jobs.sort_by_key(|job| job.created_at_ms);
    if let Some(latest) = new_jobs.last() {
        runtime.last_seen_job_ms = latest.created_at_ms;
    }
    for job in new_jobs {
        let line = format_job_event(&job);
        append_event(&runtime.events_path, &line);
        if !job.success && !no_toast && toast_throttle_gate() {
            show_failure_toast(&format_job_toast(&job));
        }
    }
    runtime.last_snapshot = Some(snapshot);
}

fn watch_daemon_liveness(runtime: &mut WatchRuntime, snapshot: &StatusSnapshot) {
    #[cfg(not(windows))]
    {
        let _ = (runtime, snapshot);
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
        if !alive && !runtime.daemon_dead_announced {
            runtime.daemon_dead_announced = true;
            append_event(
                &runtime.events_path,
                &format!("Daemon không còn sống (PID {})", snapshot.pid),
            );
        }
    }
}

fn format_job_event(job: &JobSummary) -> String {
    format!(
        "{} #{} {} {} bytes → {} (mode {}, nguồn {})",
        if job.success { "✔ In thành công" } else { "✖ In thất bại" },
        job.id,
        job.code,
        job.bytes_sent,
        if job.target.is_empty() { "(không target)" } else { job.target.as_str() },
        job.mode,
        job.source
    )
}

fn format_job_toast(job: &JobSummary) -> String {
    format!(
        "In thất bại #{} → {}: {}",
        job.id,
        if job.target.is_empty() { "(không target)" } else { job.target.as_str() },
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

fn toast_throttle_gate() -> bool {
    let now = status::now_ms();
    let last = LAST_TOAST_MS.load(Ordering::SeqCst);
    if now.saturating_sub(last) < TOAST_THROTTLE_MS {
        return false;
    }
    LAST_TOAST_MS.store(now, Ordering::SeqCst);
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
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000)
        .spawn();
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
        let failed = JobSummary { success: false, code: "transport_error".to_string(), ..job };
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
}