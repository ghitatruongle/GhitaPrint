use colored::Colorize;
use ghitaprint::os::process::{probe_health_sync, spawn_detached};
use ghitaprint::server::virtual_printer::{
    RAW_PORT_NAME, add_printer_command, raw_port_number, remove_printer_port_command,
    run_powershell_utf8,
};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const EMBEDDED_GHITAPRINT_EXE: &[u8] = include_bytes!("../../target/release/ghitaprint.exe");
const EMBEDDED_SHA256: &str = env!("GHITAPRINT_EMBEDDED_SHA256");
const EMBEDDED_SIZE_STR: &str = env!("GHITAPRINT_EMBEDDED_SIZE");

const APP_NAME: &str = "GhitaPrint";
const REG_RUN_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STUB_PREFIX: &[u8] = b"GHITAPRINT-STUB";

const EXIT_OK: u8 = 0;
const EXIT_INSTALL_FAILED: u8 = 1;

fn print_banner() {
    println!(
        "{}",
        "============================================================".bright_blue()
    );
    println!(
        "  🖨️  {} - Universal Driverless Print Engine",
        "GhitaPrint Setup".bold().bright_green()
    );
    println!(
        "  Phiên bản: {} | Trình cài đặt tự động 1-Click",
        env!("CARGO_PKG_VERSION").bright_yellow()
    );
    println!(
        "{}",
        "============================================================".bright_blue()
    );
}

fn verify_embedded_payload() -> Result<(), String> {
    if EMBEDDED_GHITAPRINT_EXE.starts_with(STUB_PREFIX) {
        return Err(
            "Bộ cài này được build nhầm từ file stub — chạy 'cargo build --release --bin ghitaprint' TRƯỚC rồi build lại setup."
                .to_string(),
        );
    }

    let digest = Sha256::digest(EMBEDDED_GHITAPRINT_EXE);
    let hex: String = digest.iter().map(|byte| format!("{:02x}", byte)).collect();
    if hex != EMBEDDED_SHA256 {
        return Err(format!(
            "SHA-256 của payload nhúng không khớp ({}... vs {}...) — bộ cài hỏng",
            &hex[..8],
            &EMBEDDED_SHA256[..8]
        ));
    }
    if EMBEDDED_GHITAPRINT_EXE.len() != embedded_size() {
        return Err("Kích thước payload nhúng không khớp — bộ cài hỏng".to_string());
    }
    Ok(())
}

fn payload_fingerprint() -> String {
    let digest = Sha256::digest(EMBEDDED_GHITAPRINT_EXE);
    digest.iter().map(|byte| format!("{:02x}", byte)).collect()
}

fn embedded_size() -> usize {
    EMBEDDED_SIZE_STR.parse().unwrap_or(0)
}

fn safe_join(base: &str, parts: &[&str]) -> Option<PathBuf> {
    if base.trim().is_empty() || base.contains("..") {
        return None;
    }
    let mut path = PathBuf::from(base);
    for part in parts {
        if part.contains("..") || part.contains('/') || part.contains('\\') {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

fn get_install_dir() -> PathBuf {
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA")
        && let Some(path) = safe_join(&local_app_data, &["Programs", APP_NAME])
    {
        return path;
    }
    if let Ok(user_profile) = std::env::var("USERPROFILE")
        && let Some(path) = safe_join(&user_profile, &["AppData", "Local", "Programs", APP_NAME])
    {
        return path;
    }
    PathBuf::from(r"C:\GhitaPrint")
}

fn merge_user_path(current: &str, dir: &str) -> (String, bool) {
    let normalized_dir = dir.trim_end_matches(['\\', '/']).to_lowercase();
    let already = current
        .split(';')
        .filter(|entry| !entry.is_empty())
        .any(|entry| entry.trim_end_matches(['\\', '/']).to_lowercase() == normalized_dir);
    if already {
        return (current.to_string(), false);
    }
    let new_path = if current.trim().is_empty() {
        dir.to_string()
    } else {
        format!("{};{}", current.trim_end_matches(';'), dir)
    };
    (new_path, true)
}

fn remove_from_path_value(current: &str, dir: &str) -> String {
    let normalized_dir = dir.trim_end_matches(['\\', '/']).to_lowercase();
    current
        .split(';')
        .filter(|entry| !entry.is_empty())
        .filter(|entry| entry.trim_end_matches(['\\', '/']).to_lowercase() != normalized_dir)
        .collect::<Vec<&str>>()
        .join(";")
}

fn stop_running_instances() {
    let _ = std::process::Command::new("taskkill")
        .args(["/IM", "ghitaprint.exe", "/F"])
        .output();
    std::thread::sleep(std::time::Duration::from_millis(300));
}

#[cfg(windows)]
fn reg_expand_sz(value: &str) -> winreg::RegValue {
    winreg::RegValue {
        bytes: value
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect(),
        vtype: winreg::enums::REG_EXPAND_SZ,
    }
}

#[cfg(windows)]
fn write_user_path(value: &str) -> std::io::Result<()> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (env_key, _) = hkcu.create_subkey("Environment")?;
    env_key.set_raw_value("Path", &reg_expand_sz(value))?;
    Ok(())
}

#[cfg(windows)]
fn read_user_path() -> String {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    hkcu.open_subkey("Environment")
        .and_then(|key| key.get_value::<String, _>("Path"))
        .unwrap_or_default()
}

#[cfg(windows)]
fn broadcast_environment_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };

    let environment_name: Vec<u16> = "Environment"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            environment_name.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            1000,
            std::ptr::null_mut(),
        );
    }
}

#[cfg(windows)]
fn install_registry_run(exe_path: &Path) -> std::io::Result<()> {
    let cmd = format!("\"{}\" daemon --start --hidden", exe_path.display());
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(REG_RUN_PATH)?;
    key.set_value(APP_NAME, &cmd)?;
    Ok(())
}

#[cfg(windows)]
fn uninstall_registry_run() {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(REG_RUN_PATH, winreg::enums::KEY_WRITE) {
        let _ = key.delete_value(APP_NAME);
    }
}

#[cfg(windows)]
fn delete_installed_marker_env() {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    if let Ok(env_key) = hkcu.open_subkey_with_flags("Environment", winreg::enums::KEY_WRITE) {
        let _ = env_key.delete_value("GHITAPRINT_INSTALLED");
    }
}

fn write_exe_with_retry(exe_dest: &Path) -> Result<(), String> {
    if exe_dest.exists() {
        for attempt in 0..6 {
            if OpenOptions::new().write(true).open(exe_dest).is_ok() {
                break;
            }
            if attempt == 5 {
                return Err(format!(
                    "File {} bị khóa sau 6 lần thử — có thể antivirus đang giữ. Đóng mọi tiến trình GhitaPrint rồi chạy lại bộ cài.",
                    exe_dest.display()
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }

    fs::write(exe_dest, EMBEDDED_GHITAPRINT_EXE)
        .map_err(|e| format!("Không thể ghi file {}: {}", exe_dest.display(), e))?;

    let written = fs::read(exe_dest).map_err(|e| {
        format!(
            "Không thể đọc lại file vừa ghi {}: {}",
            exe_dest.display(),
            e
        )
    })?;
    let digest = Sha256::digest(&written);
    let hex: String = digest.iter().map(|byte| format!("{:02x}", byte)).collect();
    if hex != payload_fingerprint() {
        return Err(
            "SHA-256 file ghi ra không khớp payload nhúng — antivirus có thể đã can thiệp. Đã dừng để tránh cài hỏng."
                .to_string(),
        );
    }
    Ok(())
}

struct InstallStep {
    name: &'static str,
    ok: bool,
    detail: String,
}

fn run_install(silent: bool, json_mode: bool) -> Result<Vec<InstallStep>, String> {
    let mut steps: Vec<InstallStep> = Vec::new();

    verify_embedded_payload()?;

    let install_dir = get_install_dir();
    let exe_dest = install_dir.join("ghitaprint.exe");
    let configured_port = ghitaprint::config::ConfigManager::new().load().api_port;

    if !silent && !json_mode {
        print_banner();
        println!("\n{}", "Bắt đầu cài đặt GhitaPrint...".bright_cyan().bold());
        println!(
            "  Thư mục cài đặt : {}",
            install_dir.display().to_string().yellow()
        );
    }

    stop_running_instances();

    fs::create_dir_all(&install_dir)
        .map_err(|e| format!("Không thể tạo thư mục cài đặt: {}", e))?;

    let exe_result = write_exe_with_retry(&exe_dest);
    steps.push(InstallStep {
        name: "extract_exe",
        ok: exe_result.is_ok(),
        detail: match &exe_result {
            Ok(()) => format!("{} (sha256 khớp)", exe_dest.display()),
            Err(e) => e.clone(),
        },
    });
    exe_result?;

    #[cfg(windows)]
    let path_result = (|| -> std::io::Result<bool> {
        let current = read_user_path();
        let (merged, changed) = merge_user_path(&current, &install_dir.to_string_lossy());
        if changed {
            write_user_path(&merged)?;
            broadcast_environment_change();
        }
        Ok(changed)
    })();
    #[cfg(windows)]
    steps.push(InstallStep {
        name: "path",
        ok: path_result.is_ok(),
        detail: match &path_result {
            Ok(true) => "Đã thêm vào PATH người dùng".to_string(),
            Ok(false) => "PATH đã có sẵn".to_string(),
            Err(e) => format!("Lỗi PATH: {}", e),
        },
    });
    #[cfg(not(windows))]
    steps.push(InstallStep {
        name: "path",
        ok: false,
        detail: "Chỉ hỗ trợ Windows".to_string(),
    });

    #[cfg(windows)]
    let run_result = install_registry_run(&exe_dest);
    #[cfg(windows)]
    steps.push(InstallStep {
        name: "autostart",
        ok: run_result.is_ok(),
        detail: match &run_result {
            Ok(()) => "Registry Run (HKCU) đã đăng ký --hidden".to_string(),
            Err(e) => format!("Lỗi Registry Run: {}", e),
        },
    });
    #[cfg(not(windows))]
    steps.push(InstallStep {
        name: "autostart",
        ok: false,
        detail: "Chỉ hỗ trợ Windows".to_string(),
    });

    let printer_result = run_powershell_utf8(&add_printer_command(configured_port));
    let printer_ok = printer_result
        .as_deref()
        .map(|result| result == "OK")
        .unwrap_or(false);
    steps.push(InstallStep {
        name: "virtual_printer",
        ok: printer_ok,
        detail: match &printer_result {
            Ok(result) if result == "OK" => format!(
                "{} → 127.0.0.1:{} (raw)",
                RAW_PORT_NAME,
                raw_port_number(configured_port)
            ),
            Ok(result) => format!("Spooler từ chối: {} (máy in trùng tên?)", result),
            Err(e) => e.clone(),
        },
    });

    let daemon_result = spawn_detached(&exe_dest, &["daemon", "--start"]).and_then(|_| {
        probe_health_sync(configured_port, 10)
            .map(|version| format!("daemon v{} đáp ứng health", version))
            .ok_or_else(|| {
                "Daemon đã spawn nhưng không đáp ứng health — xem logs tại %APPDATA%\\GhitaPrint\\logs"
                    .to_string()
            })
    });
    let daemon_ok = daemon_result.is_ok();
    steps.push(InstallStep {
        name: "daemon",
        ok: daemon_ok,
        detail: daemon_result.clone().unwrap_or_else(|error| error),
    });

    let all_critical_ok = steps
        .iter()
        .filter(|step| matches!(step.name, "extract_exe" | "daemon"))
        .all(|step| step.ok);

    if json_mode {
        let payload = serde_json::json!({
            "success": all_critical_ok,
            "install_dir": install_dir.display().to_string(),
            "port": configured_port,
            "steps": steps.iter().map(|step| serde_json::json!({
                "step": step.name,
                "ok": step.ok,
                "detail": step.detail
            })).collect::<Vec<_>>()
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else if !silent {
        for step in &steps {
            let marker = if step.ok {
                "[OK]".bright_green()
            } else {
                "[LỖI]".bright_red()
            };
            println!("  • {:<18} {} {}", step.name, marker, step.detail);
        }
        if all_critical_ok {
            println!("\n{}", "🎉 CÀI ĐẶT THÀNH CÔNG".bold().bright_green());
            println!("  • Sử dụng CLI   : Mở Terminal và gõ: ghitaprint --help");
            println!("  • Máy in ảo     : GhitaPrint (In từ Word, Excel, Chrome)");
            println!("  • Gỡ cài đặt    : Chạy lệnh setup --uninstall");
        } else {
            println!(
                "\n{}",
                "✖ CÀI ĐẶT CHƯA HOÀN TẤT — xem các bước [LỖI] phía trên"
                    .bright_red()
                    .bold()
            );
        }
    }

    if all_critical_ok {
        Ok(steps)
    } else {
        Err("Cài đặt không hoàn tất".to_string())
    }
}

fn run_uninstall(
    silent: bool,
    json_mode: bool,
    keep_config: bool,
) -> Result<Vec<InstallStep>, String> {
    let mut steps: Vec<InstallStep> = Vec::new();
    let install_dir = get_install_dir();

    if !silent && !json_mode {
        print_banner();
        println!(
            "\n{}",
            "Bắt đầu gỡ bỏ GhitaPrint khỏi hệ thống..."
                .bright_yellow()
                .bold()
        );
    }

    stop_running_instances();

    #[cfg(windows)]
    uninstall_registry_run();
    steps.push(InstallStep {
        name: "autostart",
        ok: true,
        detail: "Registry Run đã gỡ".to_string(),
    });

    #[cfg(windows)]
    {
        let current = read_user_path();
        let cleaned = remove_from_path_value(&current, &install_dir.to_string_lossy());
        let path_removed = cleaned != current;
        if path_removed {
            let _ = write_user_path(&cleaned);
            broadcast_environment_change();
        }
        delete_installed_marker_env();
        steps.push(InstallStep {
            name: "path",
            ok: true,
            detail: if path_removed {
                "Đã gỡ khỏi PATH + xóa biến GHITAPRINT_INSTALLED".to_string()
            } else {
                "PATH không có mục GhitaPrint".to_string()
            },
        });
    }

    let printer_result =
        run_powershell_utf8(&ghitaprint::server::virtual_printer::remove_printer_command(true));
    let _ = run_powershell_utf8(&remove_printer_port_command());
    steps.push(InstallStep {
        name: "virtual_printer",
        ok: true,
        detail: match &printer_result {
            Ok(result) => format!("Remove-Printer: {}", result),
            Err(e) => e.clone(),
        },
    });

    let mut config_note = "Đã giữ lại (theo --keep-config)".to_string();
    if !keep_config
        && let Ok(app_data) = std::env::var("APPDATA")
        && let Some(config_dir) = safe_join(&app_data, &["GhitaPrint"])
    {
        if config_dir.exists() {
            match fs::remove_dir_all(&config_dir) {
                Ok(()) => config_note = "Đã xóa cấu hình + logs (chứa api_token)".to_string(),
                Err(e) => config_note = format!("Không xóa được config dir: {}", e),
            }
        } else {
            config_note = "Không có thư mục cấu hình".to_string();
        }
    }
    steps.push(InstallStep {
        name: "config",
        ok: true,
        detail: config_note,
    });

    let dir_removed = if install_dir.exists() {
        match fs::remove_dir_all(&install_dir) {
            Ok(()) => "Đã xóa thư mục cài đặt".to_string(),
            Err(e) => format!("SÓT LẠI thư mục cài đặt ({}): {}", e, install_dir.display()),
        }
    } else {
        "Thư mục cài đặt không tồn tại".to_string()
    };
    let dir_clean = !install_dir.exists();
    steps.push(InstallStep {
        name: "install_dir",
        ok: dir_clean,
        detail: dir_removed,
    });

    let all_ok = steps.iter().all(|step| step.ok);
    if json_mode {
        let payload = serde_json::json!({
            "success": all_ok,
            "steps": steps.iter().map(|step| serde_json::json!({
                "step": step.name,
                "ok": step.ok,
                "detail": step.detail
            })).collect::<Vec<_>>()
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else if !silent {
        for step in &steps {
            let marker = if step.ok {
                "[OK]".bright_green()
            } else {
                "[SÓT]".bright_yellow()
            };
            println!("  • {:<18} {} {}", step.name, marker, step.detail);
        }
        if all_ok {
            println!(
                "\n{}",
                "✔ Đã gỡ bỏ toàn bộ GhitaPrint thành công!"
                    .bright_green()
                    .bold()
            );
        } else {
            println!(
                "\n{}",
                "⚠ Gỡ bỏ chưa sạch hoàn toàn — xem mục [SÓT]"
                    .bright_yellow()
                    .bold()
            );
        }
    }

    if all_ok {
        Ok(steps)
    } else {
        Err("Gỡ bỏ chưa sạch".to_string())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    let is_uninstall = args
        .iter()
        .any(|argument| argument == "--uninstall" || argument == "-u" || argument == "/U");
    let is_silent = args
        .iter()
        .any(|argument| argument == "--silent" || argument == "-s" || argument == "/S");
    let is_json = args.iter().any(|argument| argument == "--json");
    let keep_config = args.iter().any(|argument| argument == "--keep-config");
    let is_help = args
        .iter()
        .any(|argument| argument == "--help" || argument == "-h" || argument == "/?");

    if is_help {
        println!(
            "Trình cài đặt tự động GhitaPrint Setup v{}",
            env!("CARGO_PKG_VERSION")
        );
        println!("Cách dùng:");
        println!("  setup.exe                 Cài đặt tiêu chuẩn có giao diện");
        println!("  setup.exe --silent        Cài đặt ẩn trong nền không cần tương tác");
        println!("  setup.exe --json          Xuất kết quả cài/gỡ dạng JSON (cho script/MDM)");
        println!("  setup.exe --keep-config   (kèm --uninstall) Giữ lại thư mục cấu hình");
        println!("  setup.exe --uninstall     Gỡ bỏ hoàn toàn GhitaPrint khỏi hệ thống");
        return ExitCode::from(EXIT_OK);
    }

    let outcome = if is_uninstall {
        run_uninstall(is_silent, is_json, keep_config)
    } else {
        run_install(is_silent, is_json)
    };

    match outcome {
        Ok(_) => ExitCode::from(EXIT_OK),
        Err(message) => {
            if is_json {
                println!(
                    "{}",
                    serde_json::json!({
                        "success": false,
                        "error": message
                    })
                );
            } else if !message.is_empty() {
                eprintln!("✖ {}", message);
            }
            ExitCode::from(EXIT_INSTALL_FAILED)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_user_path_normalizes_trailing_separators() {
        let (merged, changed) = merge_user_path(r"C:\Other;C:\GhitaPrint\", r"C:\GhitaPrint");
        assert!(!changed);
        assert_eq!(merged, r"C:\Other;C:\GhitaPrint\");

        let (merged_new, changed_new) = merge_user_path(r"C:\Other", r"C:\GhitaPrint");
        assert!(changed_new);
        assert_eq!(merged_new, r"C:\Other;C:\GhitaPrint");
    }

    #[test]
    fn test_remove_from_path_value_case_insensitive() {
        let cleaned = remove_from_path_value(r"c:\other;C:\GHITAPRINT\x;;d:\y", r"C:\GhitaPrint\X");
        assert_eq!(cleaned, r"c:\other;d:\y");
    }

    #[test]
    fn test_safe_join_blocks_traversal_and_weird_parts() {
        assert!(safe_join(r"C:\Users\x", &["Programs", "GhitaPrint"]).is_some());
        assert!(safe_join(r"C:\Users\..\..", &["x"]).is_none());
        assert!(safe_join("", &["x"]).is_none());
        assert!(safe_join(r"C:\base", &["..\\evil"]).is_none());
        assert!(safe_join(r"C:\base", &["sub/dir"]).is_none());
    }

    #[test]
    fn test_payload_fingerprint_is_sha256_hex() {
        let fingerprint = payload_fingerprint();
        assert_eq!(fingerprint.len(), 64);
        assert!(fingerprint.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
