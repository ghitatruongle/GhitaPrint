use clap::Parser;
use ghitaprint::cli::output::{
    print_banner, print_error, print_info, print_success, print_warning, render_printers,
};
use ghitaprint::cli::{Cli, Commands, ConfigAction, StartupAction, VirtualPrinterAction};
use ghitaprint::config::{ConfigManager, Settings};
use ghitaprint::discovery::DeviceScanner;
use ghitaprint::encoder::{EscPosBuilder, FloydSteinbergRasterizer, PrintDocument, TsplBuilder};
use ghitaprint::server;
use ghitaprint::transport::{PrinterTransport, TcpTransport, UsbTransport};
use std::io::Write as IoWrite;
use std::path::Path;
use std::process::ExitCode;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const EXIT_OK: u8 = 0;
const EXIT_GENERAL_ERROR: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_PRINTER_UNREACHABLE: u8 = 3;
const EXIT_DAEMON_ERROR: u8 = 4;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Cli::parse();
    ghitaprint::status::mark_boot_started();

    let config_mgr = match &args.config_path {
        Some(path) => ConfigManager::with_path(path.clone()),
        None => ConfigManager::new(),
    };
    let settings = config_mgr.load();

    let is_daemon_run = matches!(
        &args.command,
        Commands::Daemon(daemon_args) if daemon_args.start || daemon_args.restart
    );
    let is_daemon_foreground = matches!(
        &args.command,
        Commands::Daemon(daemon_args) if (daemon_args.start || daemon_args.restart) && !daemon_args.hidden
    );

    if is_daemon_run && config_mgr.is_cwd_override() {
        print_error(
            "Phát hiện ghitaprint.toml trong thư mục hiện tại đang đè lên cấu hình hệ thống (%APPDATA%\\GhitaPrint). \
             Daemon từ chối khởi động để tránh chạy sai cấu hình — xóa/dời file này hoặc dùng --config-path.",
        );
        return ExitCode::from(EXIT_USAGE);
    }

    init_tracing(
        args.log_level.as_deref().unwrap_or(&settings.log_level),
        is_daemon_foreground,
    );
    if is_daemon_foreground {
        install_panic_hook();
    }

    if !args.json {
        print_banner();
    }

    match args.command {
        Commands::Scan(scan_args) => run_scan(&scan_args, &settings, args.json).await,
        Commands::Test(test_args) => run_test(&test_args, &settings).await,
        Commands::Print(print_args) => run_print(&print_args, &settings).await,
        Commands::Config(config_args) => {
            run_config(config_args.action, &config_mgr, &settings, args.json).await
        }
        Commands::Startup(startup_args) => run_startup(startup_args.action, args.json).await,
        Commands::VirtualPrinter(vp_args) => {
            run_virtual_printer(vp_args.action, &settings, args.json).await
        }
        Commands::Daemon(daemon_args) => run_daemon(&daemon_args, &settings, args.json).await,
        Commands::Status => run_status(&settings, args.json).await,
        Commands::Watch(watch_args) => {
            ghitaprint::watch::run_watch(watch_args.interval_ms, watch_args.no_toast, watch_args.once);
            ExitCode::from(EXIT_OK)
        }
    }
}

fn init_tracing(level: &str, to_file: bool) {
    let filter = tracing_subscriber::EnvFilter::try_new(level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let subscriber = tracing_subscriber::registry().with(filter).with(
        tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_ansi(!to_file),
    );
    if to_file && let Some(dir) = log_dir() {
        cleanup_old_logs(&dir);
        let appender = tracing_appender::rolling::daily(&dir, "ghitaprint.log");
        subscriber
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(appender),
            )
            .init();
        return;
    }
    subscriber.init();
}

fn log_dir() -> Option<std::path::PathBuf> {
    let app_data = std::env::var("APPDATA").ok()?;
    let dir = Path::new(&app_data).join("GhitaPrint").join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn install_panic_hook() {
    let Some(dir) = log_dir() else {
        return;
    };
    let crash_path = dir.join("crash.log");
    std::panic::set_hook(Box::new(move |info| {
        ghitaprint::server::remove_pid_file();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = format!("[unix {}] PANIC: {}\n", timestamp, info);
        eprintln!(
            "✖ Lỗi nghiêm trọng trong daemon — chi tiết tại {}",
            crash_path.display()
        );
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&crash_path)
            .and_then(|mut file| writeln!(file, "{}", entry));
    }));
}

const LOG_RETENTION_DAYS: u64 = 7;
const CRASH_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;

fn cleanup_old_logs(dir: &Path) {
    let retention = std::time::Duration::from_secs(LOG_RETENTION_DAYS * 24 * 60 * 60);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            if !name.starts_with("ghitaprint.log.") {
                continue;
            }
            let stale = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .map(|age| age > retention)
                .unwrap_or(false);
            if stale {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    let crash_log = dir.join("crash.log");
    if let Ok(meta) = std::fs::metadata(&crash_log)
        && meta.len() > CRASH_LOG_MAX_BYTES
    {
        let _ = std::fs::rename(&crash_log, dir.join("crash.log.old"));
    }
}

fn emit_json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

fn json_result(success: bool, code: &str, message: &str) -> serde_json::Value {
    serde_json::json!({
        "success": success,
        "code": code,
        "message": message
    })
}

async fn run_scan(
    scan_args: &ghitaprint::cli::ScanArgs,
    settings: &Settings,
    json: bool,
) -> ExitCode {
    if !json {
        print_info("Đang quét các thiết bị máy in thực tế trên hệ thống...");
    }

    let mut detected_printers = Vec::new();

    if scan_args.all {
        if let Ok(all_usb) = ghitaprint::transport::list_all_usb_devices() {
            for dev in all_usb {
                detected_printers.push(ghitaprint::cli::output::PrinterInfo {
                    id: format!("usb-{:04x}-{:04x}", dev.vendor_id, dev.product_id),
                    name: format!("{} {}", dev.manufacturer, dev.product),
                    transport: "USB Generic".to_string(),
                    address: dev.uri(),
                    status: "Đang cắm".to_string(),
                });
            }
        }
    } else {
        if scan_args.bluetooth {
            print_warning("Quét Bluetooth hiện chưa được hỗ trợ trong phiên bản này.");
        }
        if scan_args.usb || (!scan_args.network && !scan_args.bluetooth) {
            let usb_printers = DeviceScanner::scan_usb();
            detected_printers.extend(usb_printers);
        }

        if scan_args.network || (!scan_args.usb && !scan_args.bluetooth) {
            if let Some(cidr) = &scan_args.subnet {
                match DeviceScanner::scan_subnet(cidr, 9100, scan_args.timeout_ms).await {
                    Ok(found) => detected_printers.extend(found),
                    Err(e) => {
                        print_error(&e);
                        return ExitCode::from(EXIT_USAGE);
                    }
                }
            } else {
                let mut candidates: Vec<String> = Vec::new();
                if !settings.default_printer.is_empty()
                    && !settings.default_printer.starts_with("usb://")
                {
                    candidates.push(settings.default_printer.clone());
                }
                if candidates.is_empty() {
                    if let Some(cidr) = DeviceScanner::local_subnet_cidr() {
                        if !json {
                            print_info(&format!("Tự dò subnet LAN của máy: {}", cidr));
                        }
                        match DeviceScanner::scan_subnet(&cidr, 9100, scan_args.timeout_ms).await {
                            Ok(found) => detected_printers.extend(found),
                            Err(e) => print_warning(&e),
                        }
                    }
                } else {
                    let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
                    detected_printers.extend(DeviceScanner::scan_network(&refs).await);
                }
            }
        }
    }

    render_printers(&detected_printers, json);
    ExitCode::from(EXIT_OK)
}

async fn run_test(test_args: &ghitaprint::cli::TestArgs, settings: &Settings) -> ExitCode {
    let target_addr = test_args
        .address
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(settings.default_printer.as_str());

    let (print_data, doc_name) = if test_args.receipt {
        (EscPosBuilder::build_test_receipt(), "Hóa đơn nhiệt ESC/POS")
    } else if test_args.label {
        (TsplBuilder::build_test_label(), "Tem nhãn mã vạch TSPL")
    } else {
        print_info(
            "Vui lòng chọn loại in thử: --receipt (hóa đơn nhiệt) hoặc --label (tem nhãn mã vạch)",
        );
        return ExitCode::from(EXIT_USAGE);
    };

    print_info(&format!(
        "Đã tạo mẫu {} chuẩn ({} bytes)",
        doc_name,
        print_data.len()
    ));

    if target_addr.is_empty() {
        print_warning("Chưa cấu hình địa chỉ máy in mục tiêu!");
        println!("  • Chỉ định trực tiếp: ghitaprint test --receipt --address 192.168.1.200:9100");
        println!("  • Hoặc máy in USB   : ghitaprint test --label --address usb://0416:5011");
        println!("  • Hoặc đặt mặc định : ghitaprint config set default_printer <address>");
        return ExitCode::from(EXIT_USAGE);
    }

    print_info(&format!("Đang gửi lệnh in tới: {}", target_addr));
    match send_to_printer(target_addr, &print_data, settings.connection_timeout_secs).await {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(e) => {
            print_error(&e);
            ExitCode::from(EXIT_PRINTER_UNREACHABLE)
        }
    }
}

async fn run_print(print_args: &ghitaprint::cli::PrintArgs, settings: &Settings) -> ExitCode {
    let target_addr = print_args
        .address
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(settings.default_printer.as_str());

    if target_addr.is_empty() {
        print_error(
            "Vui lòng cung cấp địa chỉ máy in qua cờ --address hoặc cấu hình default_printer.",
        );
        return ExitCode::from(EXIT_USAGE);
    }

    let raw_data = if let Some(ref img_path) = print_args.image {
        match image::open(img_path) {
            Ok(img) => {
                print_info(&format!(
                    "Đang xử lý ảnh {} qua thuật toán Floyd-Steinberg...",
                    img_path.display()
                ));
                let target_width = print_args.image_width.map(|w| w.clamp(1, 2048));
                let mut raster_bytes = vec![0x1B, 0x40];
                match FloydSteinbergRasterizer::to_escpos_raster(&img, target_width) {
                    Ok(raster) => {
                        raster_bytes.extend(raster);
                        raster_bytes.extend_from_slice(&[0x1B, 0x64, 0x03]);
                        raster_bytes
                    }
                    Err(e) => {
                        print_error(&e);
                        return ExitCode::from(EXIT_GENERAL_ERROR);
                    }
                }
            }
            Err(e) => {
                print_error(&format!(
                    "Không thể mở file ảnh {}: {}",
                    img_path.display(),
                    e
                ));
                return ExitCode::from(EXIT_GENERAL_ERROR);
            }
        }
    } else if let Some(ref file_path) = print_args.file {
        match std::fs::read_to_string(file_path) {
            Ok(content) => {
                let looks_like_json = content.trim_start().starts_with('{');
                match serde_json::from_str::<PrintDocument>(&content) {
                    Ok(doc) => {
                        print_info("Đã nhận dạng JSON PrintDocument, đang biên dịch...");
                        match doc.compile(&settings.protocol, &settings.diacritics) {
                            Ok(bytes) => bytes,
                            Err(e) => {
                                print_error(&format!("Tài liệu không hợp lệ: {}", e));
                                return ExitCode::from(EXIT_GENERAL_ERROR);
                            }
                        }
                    }
                    Err(parse_error) if looks_like_json => {
                        print_error(&format!(
                            "File {} có vẻ là JSON nhưng sai cấu trúc PrintDocument ({}). Dùng --raw nếu muốn in nguyên văn.",
                            file_path.display(),
                            parse_error
                        ));
                        return ExitCode::from(EXIT_GENERAL_ERROR);
                    }
                    Err(_) => content.into_bytes(),
                }
            }
            Err(_) => match std::fs::read(file_path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    print_error(&format!("Không thể đọc tệp {}: {}", file_path.display(), e));
                    return ExitCode::from(EXIT_GENERAL_ERROR);
                }
            },
        }
    } else if let Some(ref text) = print_args.raw {
        text.as_bytes().to_vec()
    } else {
        print_error("Thiếu nội dung cần in! Dùng --file <path>, --image <path> hoặc --raw <data>.");
        return ExitCode::from(EXIT_USAGE);
    };

    if raw_data.is_empty() {
        print_error("Dữ liệu in rỗng — không gửi gì tới máy in.");
        return ExitCode::from(EXIT_USAGE);
    }

    print_info(&format!(
        "Đang gửi {} bytes tới máy in {}...",
        raw_data.len(),
        target_addr
    ));
    match send_to_printer(target_addr, &raw_data, settings.connection_timeout_secs).await {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(e) => {
            print_error(&e);
            ExitCode::from(EXIT_PRINTER_UNREACHABLE)
        }
    }
}

async fn run_config(
    action: ConfigAction,
    config_mgr: &ConfigManager,
    settings: &Settings,
    json: bool,
) -> ExitCode {
    match action {
        ConfigAction::List => {
            if json {
                let mut value = serde_json::to_value(settings).unwrap_or_default();
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "api_token".to_string(),
                        serde_json::Value::String(settings.masked_token()),
                    );
                }
                emit_json(&value);
            } else {
                print_info(&format!(
                    "Tệp cấu hình: {}",
                    config_mgr.get_path().display()
                ));
                println!("  • api_port                = {}", settings.api_port);
                println!(
                    "  • default_printer         = \"{}\"",
                    settings.default_printer
                );
                println!("  • auto_startup            = {}", settings.auto_startup);
                println!("  • log_level               = \"{}\"", settings.log_level);
                println!(
                    "  • connection_timeout_secs = {}",
                    settings.connection_timeout_secs
                );
                println!("  • protocol                = \"{}\"", settings.protocol);
                println!("  • diacritics              = \"{}\"", settings.diacritics);
                println!(
                    "  • api_token               = \"{}\"",
                    settings.masked_token()
                );
            }
            ExitCode::from(EXIT_OK)
        }
        ConfigAction::Get { key } => {
            match key.as_str() {
                "api_port" => println!("{}", settings.api_port),
                "default_printer" => println!("{}", settings.default_printer),
                "auto_startup" => println!("{}", settings.auto_startup),
                "log_level" => println!("{}", settings.log_level),
                "connection_timeout_secs" => println!("{}", settings.connection_timeout_secs),
                "protocol" => println!("{}", settings.protocol),
                "diacritics" => println!("{}", settings.diacritics),
                "api_token" => {
                    if settings.api_token.is_empty() {
                        println!("(chưa đặt)");
                    } else {
                        println!("{}", settings.masked_token());
                    }
                }
                _ => {
                    print_error(&format!("Không tìm thấy khóa cấu hình '{}'", key));
                    return ExitCode::from(EXIT_USAGE);
                }
            }
            ExitCode::from(EXIT_OK)
        }
        ConfigAction::Set { key, value } => {
            if let Err(validation) = Settings::validate_key(&key, &value) {
                print_error(&validation);
                return ExitCode::from(EXIT_USAGE);
            }

            let mut new_settings = settings.clone();
            match key.as_str() {
                "api_port" => new_settings.api_port = value.parse().unwrap_or(settings.api_port),
                "default_printer" => new_settings.default_printer = value.clone(),
                "auto_startup" => {
                    new_settings.auto_startup =
                        matches!(value.to_lowercase().as_str(), "true" | "1")
                }
                "log_level" => new_settings.log_level = value.clone(),
                "connection_timeout_secs" => {
                    new_settings.connection_timeout_secs =
                        value.parse().unwrap_or(settings.connection_timeout_secs)
                }
                "protocol" => new_settings.protocol = value.clone(),
                "diacritics" => new_settings.diacritics = value.clone(),
                "api_token" => new_settings.api_token = value.clone(),
                _ => {}
            }

            match config_mgr.save(&new_settings) {
                Ok(()) => {
                    if json {
                        emit_json(&json_result(true, "ok", &format!("Đã cập nhật '{}'", key)));
                    } else if key == "api_token" {
                        print_success(&format!(
                            "Đã cập nhật '{}' = '{}'",
                            key,
                            new_settings.masked_token()
                        ));
                        if new_settings.api_token.is_empty() {
                            print_warning(
                                "api_token rỗng — bảo vệ API đang TẮT. Mọi website có thể in qua daemon.",
                            );
                        }
                    } else {
                        print_success(&format!("Đã cập nhật '{}' = '{}'", key, value));
                    }
                    print_info(
                        "Gợi ý: khởi động lại daemon để áp dụng (ghitaprint daemon --restart).",
                    );
                    ExitCode::from(EXIT_OK)
                }
                Err(e) => {
                    print_error(&format!("Không thể lưu cấu hình: {}", e));
                    ExitCode::from(EXIT_GENERAL_ERROR)
                }
            }
        }
        ConfigAction::Reset { yes } => {
            if !yes {
                print_warning(
                    "Lệnh này xóa toàn bộ cấu hình (kể cả api_token). Chạy lại với --yes để xác nhận.",
                );
                return ExitCode::from(EXIT_USAGE);
            }
            let default_settings = Settings::default();
            match config_mgr.save(&default_settings) {
                Ok(()) => {
                    if json {
                        emit_json(&json_result(true, "ok", "Đã khôi phục cấu hình mặc định"));
                    } else {
                        print_success("Đã khôi phục cấu hình về mặc định.");
                        print_info("Gợi ý: khởi động lại daemon để áp dụng.");
                    }
                    ExitCode::from(EXIT_OK)
                }
                Err(e) => {
                    print_error(&format!("Không thể lưu cấu hình: {}", e));
                    ExitCode::from(EXIT_GENERAL_ERROR)
                }
            }
        }
    }
}

async fn run_daemon(
    daemon_args: &ghitaprint::cli::DaemonArgs,
    settings: &Settings,
    json: bool,
) -> ExitCode {
    let env_port = std::env::var("GHITAPRINT_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok());
    let daemon_port = daemon_args.port.or(env_port).unwrap_or(settings.api_port);

    if daemon_args.start && daemon_args.hidden {
        if spawn_hidden_daemon(settings) {
            if json {
                emit_json(&json_result(true, "ok", "Daemon nền đã khởi chạy ẩn"));
            } else {
                print_success("Daemon nền đã được khởi chạy ẩn (không cửa sổ console).");
                print_info("Xác nhận trạng thái bằng: ghitaprint status");
            }
            return ExitCode::from(EXIT_OK);
        }
        print_error("Không thể khởi chạy daemon nền ẩn (CreateProcessW thất bại).");
        return ExitCode::from(EXIT_DAEMON_ERROR);
    }

    if daemon_args.start || daemon_args.restart {
        if daemon_args.restart {
            print_info("Đang khởi động lại GhitaPrint Daemon...");
            stop_running_daemon();
            wait_port_released(daemon_port).await;
        }
        run_daemon_foreground(daemon_port, settings, json).await
    } else if daemon_args.stop {
        print_info("Đang dừng GhitaPrint Daemon...");
        if stop_running_daemon() {
            if json {
                emit_json(&json_result(true, "ok", "Đã dừng daemon"));
            } else {
                print_success("Đã dừng GhitaPrint Daemon.");
            }
            ExitCode::from(EXIT_OK)
        } else {
            print_info(
                "Không tìm thấy daemon nào đang chạy (không có file PID). Nếu daemon 0.0.1 cũ còn chạy, dùng Task Manager kết thúc ghitaprint.exe.",
            );
            ExitCode::from(EXIT_OK)
        }
    } else {
        print_error("Sử dụng: ghitaprint daemon --start | --stop | --restart");
        ExitCode::from(EXIT_USAGE)
    }
}

async fn run_startup(action: StartupAction, json: bool) -> ExitCode {
    match action {
        StartupAction::Install => {
            if !json {
                print_info("Đang đăng ký GhitaPrint vào Windows Registry Run (HKCU)...");
            }
            let daemon_result = ghitaprint::os::install_startup();
            let watch_result = ghitaprint::os::install_watch_startup();
            match (daemon_result, watch_result) {
                (Ok(daemon_cmd), Ok(watch_cmd)) => {
                    if json {
                        emit_json(&serde_json::json!({
                            "daemon": daemon_cmd,
                            "watch": watch_cmd
                        }));
                    } else {
                        print_success("Đã kích hoạt tự khởi động ẩn cùng Windows thành công!");
                        println!("  Daemon : {}", daemon_cmd);
                        println!("  Watch  : {}", watch_cmd);
                    }
                    ExitCode::from(EXIT_OK)
                }
                (daemon_result, watch_result) => {
                    if let Err(e) = &daemon_result {
                        print_error(&format!("Lỗi khi đăng ký khởi động daemon: {}", e));
                    }
                    if let Err(e) = &watch_result {
                        print_error(&format!("Lỗi khi đăng ký khởi động watch: {}", e));
                    }
                    ExitCode::from(EXIT_GENERAL_ERROR)
                }
            }
        }
        StartupAction::Uninstall => {
            let daemon_result = ghitaprint::os::uninstall_startup();
            let watch_result = ghitaprint::os::uninstall_watch_startup();
            let all_ok = daemon_result.is_ok() && watch_result.is_ok();
            if all_ok {
                if json {
                    emit_json(&json_result(true, "ok", "Đã hủy tự khởi động"));
                } else {
                    print_success("Đã hủy tự khởi động cùng Windows (daemon + watch)!");
                }
                ExitCode::from(EXIT_OK)
            } else {
                if let Err(e) = &daemon_result {
                    print_error(&format!("Lỗi khi gỡ đăng ký daemon: {}", e));
                }
                if let Err(e) = &watch_result {
                    print_error(&format!("Lỗi khi gỡ đăng ký watch: {}", e));
                }
                ExitCode::from(EXIT_GENERAL_ERROR)
            }
        }
        StartupAction::Status => {
            let is_enabled = ghitaprint::os::is_startup_enabled();
            let is_watch = ghitaprint::os::is_watch_startup_enabled();
            if json {
                emit_json(&serde_json::json!({
                    "enabled": is_enabled,
                    "watch_enabled": is_watch
                }));
            } else {
                print_info("Trạng thái tự khởi động cùng Windows:");
                println!(
                    "  • Registry Run Daemon: {}",
                    if is_enabled {
                        "Đã kích hoạt [✔]"
                    } else {
                        "Chưa kích hoạt"
                    }
                );
                println!(
                    "  • Registry Run Watch : {}",
                    if is_watch {
                        "Đã kích hoạt [✔]"
                    } else {
                        "Chưa kích hoạt"
                    }
                );
                if let Some(cmd) = ghitaprint::os::get_startup_command() {
                    println!("  • Lệnh daemon        : {}", cmd);
                }
                if let Some(cmd) = ghitaprint::os::get_watch_startup_command() {
                    println!("  • Lệnh watch         : {}", cmd);
                }
                if !is_enabled {
                    println!("  • Để kích hoạt, chạy : ghitaprint startup install");
                }
            }
            ExitCode::from(EXIT_OK)
        }
    }
}

async fn run_virtual_printer(
    action: VirtualPrinterAction,
    settings: &Settings,
    json: bool,
) -> ExitCode {
    match action {
        VirtualPrinterAction::Install => {
            if !json {
                print_info("Đang đăng ký máy in ảo 'GhitaPrint' vào Windows Print Spooler...");
            }
            match server::reinstall_virtual_printer(settings.api_port) {
                Ok(msg) => {
                    if json {
                        emit_json(&json_result(true, "ok", &msg));
                    } else {
                        print_success(&msg);
                    }
                    ExitCode::from(EXIT_OK)
                }
                Err(e) => {
                    print_error(&format!("Lỗi cài đặt máy in ảo: {}", e));
                    ExitCode::from(EXIT_GENERAL_ERROR)
                }
            }
        }
        VirtualPrinterAction::Uninstall => match server::uninstall_virtual_printer() {
            Ok(msg) => {
                if json {
                    emit_json(&json_result(true, "ok", &msg));
                } else {
                    print_success(&msg);
                }
                ExitCode::from(EXIT_OK)
            }
            Err(e) => {
                print_error(&format!("Lỗi gỡ bỏ máy in ảo: {}", e));
                ExitCode::from(EXIT_GENERAL_ERROR)
            }
        },
        VirtualPrinterAction::Status => {
            let installed = server::is_virtual_printer_installed();
            let expected = server::virtual_printer::RAW_PORT_NAME.to_string();
            let raw_port = server::virtual_printer::raw_port_number(settings.api_port);
            let installed_port = if installed {
                server::get_installed_printer_port()
            } else {
                None
            };
            if json {
                emit_json(&serde_json::json!({
                    "installed": installed,
                    "installed_port": installed_port,
                    "expected_port": expected,
                    "raw_listen_port": raw_port,
                    "port_matches_config": installed_port.as_deref() == Some(expected.as_str())
                }));
            } else {
                print_info("Trạng thái Máy in ảo Windows:");
                if installed {
                    println!("  • Máy in ảo 'GhitaPrint': Đã cài đặt [✔]");
                    match &installed_port {
                        Some(port) => println!("  • Điểm tiếp nhận thực tế: {}", port),
                        None => println!("  • Điểm tiếp nhận thực tế: (không đọc được)"),
                    }
                    println!(
                        "  • Raw spooler listener  : 127.0.0.1:{} (daemon)",
                        raw_port
                    );
                    if installed_port.as_deref() != Some(expected.as_str()) {
                        print_warning(
                            "  • Cổng cấu hình đã đổi — máy in ảo đang trỏ sai! Chạy 'ghitaprint virtual-printer install' để cập nhật.",
                        );
                    }
                } else {
                    println!("  • Máy in ảo 'GhitaPrint': Chưa cài đặt [Chưa có]");
                    println!("  • Để cài đặt, chạy      : ghitaprint virtual-printer install");
                }
            }
            ExitCode::from(EXIT_OK)
        }
    }
}

async fn run_daemon_foreground(port: u16, settings: &Settings, json: bool) -> ExitCode {
    tracing::info!(
        "GhitaPrint daemon v{} khởi động trên cổng {}",
        env!("CARGO_PKG_VERSION"),
        port
    );

    let check_virtual_printer = port == settings.api_port;
    if !check_virtual_printer {
        tracing::info!(
            "Daemon chạy trên cổng tạm {} (khác cổng cấu hình {}) — bỏ qua kiểm tra máy in ảo",
            port,
            settings.api_port
        );
    }

    if !json {
        print_info(&format!(
            "Đang khởi chạy GhitaPrint Daemon trên cổng {}...",
            port
        ));
    }

    let result = server::start_server(server::DaemonServerConfig {
        port,
        default_printer: settings.default_printer.clone(),
        timeout_secs: settings.connection_timeout_secs,
        api_token: settings.api_token.clone(),
        diacritics: settings.diacritics.clone(),
        protocol: settings.protocol.clone(),
        json_mode: json,
        check_virtual_printer,
    })
    .await;

    match result {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if server::probe_health(port, 1).await.is_some() {
                print_error(&format!(
                    "Cổng {} đã có GhitaPrint daemon đang chạy. Dừng trước bằng: ghitaprint daemon --stop",
                    port
                ));
            } else {
                print_error(&format!(
                    "Cổng {} đã bị ứng dụng khác chiếm — chọn cổng khác bằng: ghitaprint config set api_port <port>",
                    port
                ));
            }
            ExitCode::from(EXIT_DAEMON_ERROR)
        }
        Err(e) => {
            print_error(&format!("Lỗi máy chủ: {}", e));
            ExitCode::from(EXIT_DAEMON_ERROR)
        }
    }
}

async fn wait_port_released(port: u16) {
    for _ in 0..20 {
        if !TcpTransport::check_alive(&format!("127.0.0.1:{}", port), 1).await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}

async fn run_status(settings: &Settings, json: bool) -> ExitCode {
    let is_startup = ghitaprint::os::is_startup_enabled();
    let is_watch = ghitaprint::os::is_watch_startup_enabled();
    let snapshot = ghitaprint::status::load_status();
    let snapshot_live = snapshot
        .as_ref()
        .map(|snap| server::is_live_ghitaprint_pid(snap.pid))
        .unwrap_or(false);
    let daemon_version = if snapshot_live {
        snapshot.as_ref().map(|snap| snap.version.clone())
    } else {
        server::probe_health(settings.api_port, 1).await
    };
    let daemon_alive = daemon_version.is_some();

    if json {
        emit_json(&serde_json::json!({
            "daemon_running": daemon_alive,
            "daemon_version": daemon_version,
            "startup_enabled": is_startup,
            "watch_enabled": is_watch,
            "default_printer": settings.default_printer,
            "api_port": settings.api_port,
            "snapshot": snapshot.as_ref().map(|snap| serde_json::json!({
                "pid": snap.pid,
                "ready_after_ms": snap.ready_after_ms,
                "prints_total": snap.prints_total,
                "prints_ok": snap.prints_ok,
                "prints_failed": snap.prints_failed,
                "last_error": snap.last_error,
                "jobs": snap.jobs,
            }))
        }));
        return ExitCode::from(EXIT_OK);
    }

    print_info("Trạng thái GhitaPrint Engine:");
    println!(
        "  • Tiến trình Daemon   : {}",
        match &daemon_version {
            Some(version) => format!("Đang chạy [✔] (v{})", version),
            None => "Không chạy".to_string(),
        }
    );
    if let (true, Some(snap)) = (snapshot_live, snapshot.as_ref()) {
        if let Some(ready_after) = snap.ready_after_ms {
            println!("  • Khởi động boot→ready: {} ms", ready_after);
        }
        println!(
            "  • Thống kê in         : tổng {} (OK {}, lỗi {})",
            snap.prints_total, snap.prints_ok, snap.prints_failed
        );
        if let Some(error) = &snap.last_error {
            println!(
                "  • Lỗi gần nhất        : {} — {} ({})",
                error.target, error.message, error.source
            );
        }
        for job in snap.jobs.iter().take(5) {
            println!(
                "    {} #{} {} {} bytes → {} ({})",
                if job.success { "✔" } else { "✖" },
                job.id,
                if job.success { "OK" } else { "LỖI" },
                job.bytes_sent,
                if job.target.is_empty() { "(không target)" } else { &job.target },
                job.source
            );
        }
    }
    println!(
        "  • Tự khởi động Windows: {}",
        if is_startup {
            "Đã bật [✔]"
        } else {
            "Chưa bật"
        }
    );
    println!(
        "  • Watch companion     : {}",
        if is_watch {
            "Đã bật [✔] (ghitaprint watch)"
        } else {
            "Chưa bật (ghitaprint startup install để bật kèm daemon)"
        }
    );
    println!(
        "  • Máy in mặc định     : {}",
        if settings.default_printer.is_empty() {
            "Chưa cấu hình"
        } else {
            &settings.default_printer
        }
    );
    println!(
        "  • Cổng Local API      : http://127.0.0.1:{} (WebSocket + REST)",
        settings.api_port
    );
    ExitCode::from(EXIT_OK)
}

async fn send_to_printer(target_addr: &str, data: &[u8], timeout_secs: u64) -> Result<(), String> {
    if target_addr.starts_with("usb://") {
        let target = target_addr.to_string();
        let opened = tokio::task::spawn_blocking(move || UsbTransport::open_uri(&target)).await;
        match opened {
            Ok(Ok(mut usb)) => {
                usb.timeout_secs = timeout_secs.max(30);
                usb.send_raw(data)
                    .await
                    .map_err(|e| format!("Lỗi khi gửi dữ liệu USB: {}", e))
            }
            Ok(Err(e)) => Err(format!("Không thể mở máy in USB: {}", e)),
            Err(e) => Err(format!("Lỗi task USB: {}", e)),
        }
    } else {
        match TcpTransport::connect(target_addr, timeout_secs).await {
            Ok(mut tcp) => tcp
                .send_raw(data)
                .await
                .map_err(|e| format!("Lỗi khi gửi dữ liệu mạng: {}", e)),
            Err(e) => Err(format!(
                "Không thể kết nối máy in mạng {}: {}",
                target_addr, e
            )),
        }
    }
}

#[cfg(windows)]
fn spawn_hidden_daemon(_settings: &Settings) -> bool {
    let Ok(exe_path) = std::env::current_exe() else {
        return false;
    };

    ghitaprint::os::process::spawn_detached(&exe_path, &["daemon", "--start"]).is_ok()
}

#[cfg(not(windows))]
fn spawn_hidden_daemon(_settings: &Settings) -> bool {
    false
}

#[cfg(windows)]
fn stop_running_daemon() -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        PROCESS_TERMINATE, OpenProcess, TerminateProcess,
    };

    let self_pid = std::process::id();
    let Some(pid) = server::read_daemon_pid().filter(|candidate| *candidate != self_pid) else {
        return false;
    };

    if !server::is_live_ghitaprint_pid(pid) {
        tracing::warn!(
            "PID {} trong daemon.pid không còn là tiến trình ghitaprint (PID-reuse?) — bỏ qua kill, chỉ dọn file",
            pid
        );
        server::remove_pid_file();
        return false;
    }

    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let terminated = unsafe { TerminateProcess(handle, 1) } != 0;
    unsafe {
        let _ = CloseHandle(handle);
    }

    std::thread::sleep(std::time::Duration::from_millis(400));
    server::remove_pid_file();
    terminated
}

#[cfg(not(windows))]
fn stop_running_daemon() -> bool {
    server::remove_pid_file();
    false
}
