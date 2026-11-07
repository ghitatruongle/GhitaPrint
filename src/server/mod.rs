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

use std::net::SocketAddr;
use std::path::PathBuf;

pub fn daemon_pid_file() -> Option<PathBuf> {
    let app_data = std::env::var("APPDATA").ok()?;
    let mut dir = PathBuf::from(app_data);
    dir.push("GhitaPrint");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("daemon.pid"))
}

pub fn write_pid_file() {
    if let Some(path) = daemon_pid_file() {
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

pub async fn start_server(
    port: u16,
    default_printer: String,
    timeout_secs: u64,
    api_token: String,
    diacritics: String,
    protocol: String,
    json_mode: bool,
) -> Result<(), std::io::Error> {
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
    tracing::info!("Daemon đã bind tại 127.0.0.1:{}", port);

    if json_mode {
        println!(
            "{}",
            serde_json::json!({
                "event": "daemon_ready",
                "port": port,
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
