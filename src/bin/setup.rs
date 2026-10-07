use colored::Colorize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
use winreg::RegKey;
#[cfg(windows)]
use winreg::enums::*;

const EMBEDDED_GHITAPRINT_EXE: &[u8] = include_bytes!("../../target/release/ghitaprint.exe");

const APP_NAME: &str = "GhitaPrint";
const REG_RUN_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const DEFAULT_PORT: u16 = 9123;

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

fn get_install_dir() -> PathBuf {
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local_app_data)
            .join("Programs")
            .join(APP_NAME)
    } else if let Ok(user_profile) = std::env::var("USERPROFILE") {
        PathBuf::from(user_profile)
            .join("AppData")
            .join("Local")
            .join("Programs")
            .join(APP_NAME)
    } else {
        PathBuf::from(r"C:\GhitaPrint")
    }
}

fn stop_running_instances() {
    let _ = Command::new("taskkill")
        .args(["/IM", "ghitaprint.exe", "/F"])
        .output();
    std::thread::sleep(std::time::Duration::from_millis(300));
}

#[cfg(windows)]
fn add_to_user_path(dir: &Path) -> io::Result<bool> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env_key, _) = hkcu.create_subkey("Environment")?;
    let dir_str = dir.to_string_lossy().to_string();

    let current_path: String = env_key.get_value("Path").unwrap_or_default();

    let paths: Vec<&str> = current_path.split(';').filter(|s| !s.is_empty()).collect();
    if paths.iter().any(|p| p.eq_ignore_ascii_case(&dir_str)) {
        return Ok(false);
    }

    let new_path = if current_path.is_empty() {
        dir_str
    } else {
        format!("{};{}", current_path.trim_end_matches(';'), dir_str)
    };

    env_key.set_value("Path", &new_path)?;

    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "[System.Environment]::SetEnvironmentVariable('GHITAPRINT_INSTALLED', '1', 'User')",
        ])
        .output();

    Ok(true)
}

#[cfg(windows)]
fn remove_from_user_path(dir: &Path) -> io::Result<bool> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(env_key) = hkcu.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE) {
        let dir_str = dir.to_string_lossy().to_string();
        let current_path: String = env_key.get_value("Path").unwrap_or_default();
        let paths: Vec<&str> = current_path
            .split(';')
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case(&dir_str))
            .collect();
        let new_path = paths.join(";");
        env_key.set_value("Path", &new_path)?;
        return Ok(true);
    }
    Ok(false)
}

#[cfg(windows)]
fn install_registry_run(exe_path: &Path) -> io::Result<()> {
    let cmd = format!("\"{}\" daemon --start", exe_path.display());
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(REG_RUN_PATH)?;
    key.set_value(APP_NAME, &cmd)?;
    Ok(())
}

#[cfg(windows)]
fn uninstall_registry_run() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(REG_RUN_PATH, KEY_WRITE) {
        let _ = key.delete_value(APP_NAME);
    }
}

fn register_virtual_printer(port: u16) -> bool {
    let port_url = format!("http://127.0.0.1:{}/printers/ghitaprint", port);
    let ps_cmd = format!(
        "try {{ Add-Printer -Name '{}' -PortName '{}' -DriverName 'Microsoft IPP Class Driver' -ErrorAction Stop; Write-Output 'OK' }} catch {{ Write-Output $_.Exception.Message }}",
        APP_NAME, port_url
    );

    if let Ok(output) = Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps_cmd])
        .output()
    {
        let res = String::from_utf8_lossy(&output.stdout);
        return res.contains("OK");
    }
    false
}

fn unregister_virtual_printer() {
    let ps_cmd = format!(
        "try {{ Remove-Printer -Name '{}' -ErrorAction SilentlyContinue }} catch {{}}",
        APP_NAME
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps_cmd])
        .output();
}

fn start_daemon_detached(exe_path: &Path) {
    let _ = Command::new(exe_path).args(["daemon", "--start"]).spawn();
}

fn run_install(silent: bool) {
    if !silent {
        print_banner();
        println!("\n{}", "Bắt đầu cài đặt GhitaPrint...".bright_cyan().bold());
    }

    let install_dir = get_install_dir();
    let exe_dest = install_dir.join("ghitaprint.exe");

    if !silent {
        println!(
            "  1. Thư mục cài đặt : {}",
            install_dir.display().to_string().yellow()
        );
    }

    stop_running_instances();

    if let Err(e) = fs::create_dir_all(&install_dir) {
        eprintln!(
            "{} Không thể tạo thư mục cài đặt: {}",
            "✖".bright_red().bold(),
            e
        );
        return;
    }

    if !silent {
        print!("  2. Trích xuất file thực thi... ");
    }
    match fs::write(&exe_dest, EMBEDDED_GHITAPRINT_EXE) {
        Ok(_) => {
            if !silent {
                println!("{}", "[OK]".bright_green());
            }
        }
        Err(e) => {
            if !silent {
                println!("{}", "[LỖI]".bright_red());
            }
            eprintln!(
                "{} Không thể ghi file {}: {}",
                "✖".bright_red().bold(),
                exe_dest.display(),
                e
            );
            return;
        }
    }

    if !silent {
        print!("  3. Đăng ký biến môi trường PATH (System/User)... ");
    }
    #[cfg(windows)]
    match add_to_user_path(&install_dir) {
        Ok(true) => {
            if !silent {
                println!("{}", "[ĐÃ THÊM]".bright_green());
            }
        }
        Ok(false) => {
            if !silent {
                println!("{}", "[ĐÃ TỒN TẠI]".bright_blue());
            }
        }
        Err(_) => {
            if !silent {
                println!("{}", "[BỎ QUA]".bright_yellow());
            }
        }
    }

    if !silent {
        print!("  4. Thiết lập tự khởi động cùng Windows (Registry Run)... ");
    }
    #[cfg(windows)]
    match install_registry_run(&exe_dest) {
        Ok(_) => {
            if !silent {
                println!("{}", "[OK]".bright_green());
            }
        }
        Err(e) => {
            if !silent {
                println!("{}", format!("[BỎ QUA: {}]", e).bright_yellow());
            }
        }
    }

    if !silent {
        print!("  5. Cài đặt Máy in ảo Windows 'GhitaPrint' (Microsoft IPP)... ");
    }
    if register_virtual_printer(DEFAULT_PORT) {
        if !silent {
            println!("{}", "[OK]".bright_green());
        }
    } else if !silent {
        println!("{}", "[HOÀN TẤT VỚI CẢNH BÁO]".bright_yellow());
    }

    if !silent {
        print!("  6. Khởi động GhitaPrint Daemon nền... ");
    }
    start_daemon_detached(&exe_dest);
    if !silent {
        println!("{}", "[ĐANG CHẠY]".bright_green());
    }

    if !silent {
        println!(
            "\n{}",
            "============================================================".bright_green()
        );
        println!("  🎉 {}!", "CÀI ĐẶT THÀNH CÔNG".bold().bright_green());
        println!(
            "  • GhitaPrint đã sẵn sàng hoạt động tại: {}",
            exe_dest.display()
        );
        println!("  • Cổng Local API   : http://127.0.0.1:9123");
        println!("  • Máy in ảo        : GhitaPrint (In từ Word, Excel, Chrome)");
        println!("  • Sử dụng CLI      : Mở Terminal và gõ: ghitaprint --help");
        println!("  • Gỡ cài đặt       : Chạy lệnh setup --uninstall");
        println!(
            "{}\n",
            "============================================================".bright_green()
        );
    }
}

fn run_uninstall(silent: bool) {
    if !silent {
        print_banner();
        println!(
            "\n{}",
            "Bắt đầu gỡ bỏ GhitaPrint khỏi hệ thống..."
                .bright_yellow()
                .bold()
        );
    }

    let install_dir = get_install_dir();

    stop_running_instances();

    #[cfg(windows)]
    uninstall_registry_run();

    #[cfg(windows)]
    let _ = remove_from_user_path(&install_dir);

    unregister_virtual_printer();

    if install_dir.exists() {
        let _ = fs::remove_dir_all(&install_dir);
    }

    if !silent {
        println!(
            "{}",
            "✔ Đã gỡ bỏ toàn bộ GhitaPrint thành công!"
                .bright_green()
                .bold()
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let is_uninstall = args
        .iter()
        .any(|a| a == "--uninstall" || a == "-u" || a == "/U");
    let is_silent = args
        .iter()
        .any(|a| a == "--silent" || a == "-s" || a == "/S");
    let is_help = args.iter().any(|a| a == "--help" || a == "-h" || a == "/?");

    if is_help {
        println!("Trình cài đặt tự động GhitaPrint Setup");
        println!("Cách dùng:");
        println!("  setup.exe                 Cài đặt tiêu chuẩn có giao diện");
        println!("  setup.exe --silent        Cài đặt ẩn trong nền không cần tương tác");
        println!("  setup.exe --uninstall     Gỡ bỏ hoàn toàn GhitaPrint khỏi hệ thống");
        return;
    }

    if is_uninstall {
        run_uninstall(is_silent);
    } else {
        run_install(is_silent);
    }
}
