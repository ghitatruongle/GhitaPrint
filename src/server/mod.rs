pub mod ipp;
pub mod pwg;
pub mod routes;
pub mod spooler_listener;
pub mod virtual_printer;

pub use routes::{AppState, JobRecord, PrintResponse, create_router, execute_print};
pub use virtual_printer::{
    ensure_virtual_printer_matches, get_installed_printer_port, install_virtual_printer,
    is_virtual_printer_installed, reinstall_virtual_printer, uninstall_virtual_printer,
};

use crate::status;
use std::net::SocketAddr;
use std::path::PathBuf;

pub fn daemon_pid_file() -> Option<PathBuf> {
    let app_data = std::env::var("APPDATA").ok()?;
    let mut dir = PathBuf::from(app_data);
    dir.push("GhitaPrint");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("daemon.pid"))
}

pub fn daemon_process_file_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name().map(|name| name.to_string_lossy().to_string()))
        .unwrap_or_else(|| "ghitaprint.exe".to_string())
}

pub fn is_live_ghitaprint_pid(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    let expected_name = daemon_process_file_name();
    crate::os::process::process_image_name(pid)
        .map(|image| {
            std::path::Path::new(&image)
                .file_name()
                .map(|name| name.eq_ignore_ascii_case(&expected_name))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

pub struct DaemonServerConfig {
    pub port: u16,
    pub default_printer: String,
    pub timeout_secs: u64,
    pub api_token: String,
    pub diacritics: String,
    pub protocol: String,
    pub json_mode: bool,
    pub check_virtual_printer: bool,
}

pub fn write_pid_file() {
    if let Some(path) = daemon_pid_file() {
        if let Some(existing) = read_daemon_pid()
            && existing != std::process::id()
            && is_live_ghitaprint_pid(existing)
        {
            tracing::warn!(
                "daemon.pid đã trỏ tới tiến trình ghitaprint đang sống (PID {}) — ghi đè. Nếu chạy đa cổng, 'daemon --stop' sẽ dừng daemon mới nhất.",
                existing
            );
        }
        let _ = std::fs::write(&path, std::process::id().to_string());
    }
}

pub fn read_daemon_pid() -> Option<u32> {
    let path = daemon_pid_file()?;
    let content = std::fs::read_to_string(path).ok()?;
    content.trim().parse::<u32>().ok()
}

pub fn remove_pid_file() {
    if let Some(path) = daemon_pid_file() {
        let _ = std::fs::remove_file(path);
    }
}

pub async fn probe_health(port: u16, attempts: u32) -> Option<String> {
    for _ in 0..attempts {
        let connected = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await;
        if let Ok(Ok(mut stream)) = connected {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let request = format!(
                "GET /api/v1/health HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n",
                port
            );
            if stream.write_all(request.as_bytes()).await.is_ok() {
                let mut buf = vec![0u8; 2048];
                if let Ok(n) = stream.read(&mut buf).await {
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    if text.starts_with("HTTP/1.1 200") || text.starts_with("HTTP/1.0 200") {
                        if let Some(body_start) = text.find('{')
                            && let Ok(json) =
                                serde_json::from_str::<serde_json::Value>(&text[body_start..])
                        {
                            return json["version"].as_str().map(|s| s.to_string());
                        }
                        return Some(env!("CARGO_PKG_VERSION").to_string());
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    None
}

pub async fn start_server(config: DaemonServerConfig) -> Result<(), std::io::Error> {
    let DaemonServerConfig {
        port,
        default_printer,
        timeout_secs,
        api_token,
        diacritics,
        protocol,
        json_mode,
        check_virtual_printer,
    } = config;
    status::set_api_port(port);
    if let Some(dir) = status::status_dir() {
        status::init_status_file(dir.join(status::STATUS_FILE_NAME));
    }
    let state = AppState::new(
        default_printer,
        timeout_secs,
        api_token,
        diacritics,
        protocol,
    );

    let app = create_router(state.clone());
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    let raw_port = port.wrapping_add(1);
    match tokio::net::TcpListener::bind(("127.0.0.1", raw_port)).await {
        Ok(raw_listener) => {
            tracing::info!("Raw spooler listener đã bind tại 127.0.0.1:{}", raw_port);
            let raw_state = state.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    spooler_listener::run_raw_listener_on(raw_listener, raw_state).await
                {
                    tracing::warn!("Raw spooler listener dừng: {}", error);
                }
            });
        }
        Err(error) => {
            tracing::warn!(
                "Không bind được raw spooler listener 127.0.0.1:{} ({}): máy in ảo sẽ không nhận job từ spooler",
                raw_port,
                error
            );
        }
    }

    write_pid_file();
    status::mark_ready();
    write_initial_status(port);

    spawn_background_virtual_printer_check(port, check_virtual_printer);

    tracing::info!("Daemon đã bind tại 127.0.0.1:{}", port);

    if json_mode {
        println!(
            "{}",
            serde_json::json!({
                "event": "daemon_ready",
                "port": port,
                "ready_after_ms": status::boot_to_ready_ms(),
                "version": env!("CARGO_PKG_VERSION")
            })
        );
    } else {
        println!(
            "✔ GhitaPrint Local API đã sẵn sàng tại: http://127.0.0.1:{}",
            port
        );
        println!("  • REST API   : http://127.0.0.1:{}/api/v1/print", port);
        println!("  • WebSocket  : ws://127.0.0.1:{}/api/v1/ws", port);
        if let Some(ready_after) = status::boot_to_ready_ms() {
            println!("  • Thời gian khởi động: {} ms", ready_after);
        }
        println!("  • Nhấn Ctrl + C để dừng dịch vụ.");
    }

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("Nhận tín hiệu dừng (Ctrl+C) — đang tắt mềm...");
    };

    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await;
    remove_pid_file();
    tracing::info!("Daemon đã dừng sạch sẽ");
    serve_result
}

fn write_initial_status(port: u16) {
    let snapshot = status::StatusSnapshot {
        version: env!("CARGO_PKG_VERSION").to_string(),
        pid: std::process::id(),
        api_port: port,
        boot_started_at_ms: status::boot_started_ms(),
        ready_at_ms: status::ready_at_ms(),
        ready_after_ms: status::boot_to_ready_ms(),
        updated_at_ms: status::now_ms(),
        ..status::StatusSnapshot::default()
    };
    if let Err(e) = status::write_status(&snapshot) {
        tracing::warn!("Không thể ghi status.json ban đầu: {}", e);
    }
}

fn spawn_background_virtual_printer_check(port: u16, check_virtual_printer: bool) {
    if !check_virtual_printer {
        tracing::info!(
            "Daemon chạy trên cổng tạm {} — bỏ qua kiểm tra máy in ảo nền",
            port
        );
        return;
    }
    let raw_port = virtual_printer::raw_port_number(port);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let check_port = port;
        let outcome = tokio::task::spawn_blocking(move || {
            virtual_printer::ensure_virtual_printer_matches(check_port)
        })
        .await;
        let port_ok = match outcome {
            Ok(Ok(None)) => {
                tracing::info!(
                    "Máy in ảo 'GhitaPrint' sẵn sàng (raw listener 127.0.0.1:{})",
                    raw_port
                );
                true
            }
            Ok(Ok(Some(note))) => {
                tracing::warn!("Máy in ảo: {}", note);
                false
            }
            Ok(Err(error)) => {
                tracing::warn!("Kiểm tra máy in ảo lỗi: {}", error);
                false
            }
            Err(join_error) => {
                tracing::warn!("Task kiểm tra máy in ảo lỗi: {}", join_error);
                false
            }
        };
        status::record_virtual_printer_check(status::VirtualPrinterCheck {
            port_ok,
            raw_port,
            checked_at_ms: status::now_ms(),
        });
    });
}
