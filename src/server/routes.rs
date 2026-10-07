use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::{Html, IntoResponse, Json},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::time::Instant;

use crate::cli::output::PrinterInfo;
use crate::discovery::DeviceScanner;
use crate::encoder::PrintDocument;
use crate::transport::{PrinterTransport, TcpTransport, UsbTransport};

#[derive(Clone)]
pub struct AppState {
    pub start_time: Instant,
    pub default_printer: String,
    pub timeout_secs: u64,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub default_printer: String,
}

#[derive(Deserialize)]
pub struct PrintRequest {
    pub target: Option<String>,
    pub document: Option<PrintDocument>,
    pub raw: Option<String>,
}

#[derive(Serialize)]
pub struct PrintResponse {
    pub success: bool,
    pub message: String,
    pub bytes_sent: usize,
}

use axum::extract::DefaultBodyLimit;
use tower_http::cors::{Any, CorsLayer};

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/", get(welcome_handler))
        .route("/api/v1/health", get(health_handler))
        .route("/api/v1/printers", get(printers_handler))
        .route("/api/v1/print", post(print_handler))
        .route("/api/v1/ws", get(ws_handler))
        .layer(cors)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state(Arc::new(state))
}

async fn welcome_handler() -> Html<&'static str> {
    Html(
        r#"<!DOCTYPE html>
<html lang="vi">
<head>
    <meta charset="UTF-8">
    <title>GhitaPrint Local API</title>
    <style>
        body { font-family: system-ui, sans-serif; background: #0f172a; color: #f8fafc; padding: 40px; text-align: center; }
        .card { background: #1e293b; max-width: 600px; margin: 40px auto; padding: 30px; border-radius: 12px; box-shadow: 0 4px 20px rgba(0,0,0,0.5); }
        h1 { color: #38bdf8; margin-bottom: 8px; }
        .badge { background: #10b981; color: white; padding: 4px 12px; border-radius: 9999px; font-size: 14px; font-weight: bold; }
        ul { text-align: left; line-height: 1.8; margin-top: 20px; }
        code { background: #334155; padding: 2px 6px; border-radius: 4px; color: #f43f5e; }
    </style>
</head>
<body>
    <div class="card">
        <h1>🖨️ GhitaPrint Daemon</h1>
        <p><span class="badge">Online (Sẵn Sàng)</span></p>
        <p>Universal Driverless Print Engine API đang hoạt động.</p>
        <ul>
            <li><code>GET /api/v1/health</code> - Kiểm tra trạng thái máy chủ</li>
            <li><code>GET /api/v1/printers</code> - Danh sách máy in USB & Mạng kết nối</li>
            <li><code>POST /api/v1/print</code> - Gửi lệnh in (JSON Document hoặc Raw)</li>
            <li><code>WS /api/v1/ws</code> - Kênh WebSocket hai chiều thời gian thực</li>
        </ul>
    </div>
</body>
</html>"#,
    )
}

async fn health_handler(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        default_printer: state.default_printer.clone(),
    })
}

async fn printers_handler() -> Json<Vec<PrinterInfo>> {
    let printers = DeviceScanner::scan_usb();
    Json(printers)
}

async fn print_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PrintRequest>,
) -> Json<PrintResponse> {
    let target = req
        .target
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&state.default_printer);

    if target.is_empty() {
        return Json(PrintResponse {
            success: false,
            message: "Chưa cấu hình máy in mục tiêu (target).".to_string(),
            bytes_sent: 0,
        });
    }

    let raw_bytes = if let Some(doc) = req.document {
        doc.compile_escpos()
    } else if let Some(raw) = req.raw {
        raw.into_bytes()
    } else {
        return Json(PrintResponse {
            success: false,
            message: "Thiếu dữ liệu in (document hoặc raw).".to_string(),
            bytes_sent: 0,
        });
    };

    let len = raw_bytes.len();

    if target.starts_with("usb://") {
        match UsbTransport::open_uri(target) {
            Ok(mut usb) => match usb.send_raw(&raw_bytes).await {
                Ok(_) => Json(PrintResponse {
                    success: true,
                    message: format!("Đã in thành công {} bytes qua USB Direct", len),
                    bytes_sent: len,
                }),
                Err(e) => Json(PrintResponse {
                    success: false,
                    message: format!("Lỗi ghi USB: {}", e),
                    bytes_sent: 0,
                }),
            },
            Err(e) => Json(PrintResponse {
                success: false,
                message: format!("Không thể mở thiết bị USB: {}", e),
                bytes_sent: 0,
            }),
        }
    } else {
        match TcpTransport::connect(target, state.timeout_secs).await {
            Ok(mut tcp) => match tcp.send_raw(&raw_bytes).await {
                Ok(_) => Json(PrintResponse {
                    success: true,
                    message: format!("Đã in thành công {} bytes qua TCP JetDirect", len),
                    bytes_sent: len,
                }),
                Err(e) => Json(PrintResponse {
                    success: false,
                    message: format!("Lỗi gửi mạng: {}", e),
                    bytes_sent: 0,
                }),
            },
            Err(e) => Json(PrintResponse {
                success: false,
                message: format!("Không thể kết nối máy in mạng {}: {}", target, e),
                bytes_sent: 0,
            }),
        }
    }
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    if socket
        .send(Message::Text(
            "{\"event\":\"connected\",\"message\":\"GhitaPrint WebSocket Channel Ready\"}".into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    while let Some(Ok(msg)) = socket.recv().await {
        if matches!(&msg, Message::Text(text) if text.contains("ping")) {
            let _ = socket
                .send(Message::Text("{\"event\":\"pong\"}".into()))
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_endpoint() {
        let state = AppState {
            start_time: Instant::now(),
            default_printer: "usb://0416:5011".to_string(),
            timeout_secs: 5,
        };

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_welcome_page() {
        let state = AppState {
            start_time: Instant::now(),
            default_printer: String::new(),
            timeout_secs: 5,
        };

        let app = create_router(state);

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cors_headers_present() {
        let state = AppState {
            start_time: Instant::now(),
            default_printer: String::new(),
            timeout_secs: 5,
        };

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .header("Origin", "http://localhost:3000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response
                .headers()
                .contains_key("access-control-allow-origin")
        );
    }

    #[tokio::test]
    async fn test_print_endpoint_empty_target_validation() {
        let state = AppState {
            start_time: Instant::now(),
            default_printer: String::new(),
            timeout_secs: 5,
        };

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"target":"","raw":"test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
