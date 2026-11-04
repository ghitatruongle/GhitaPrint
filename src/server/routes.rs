use axum::{
    Router,
    extract::{
        DefaultBodyLimit, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::time::Instant;

use crate::discovery::DeviceScanner;
use crate::encoder::PrintDocument;
use crate::transport::{PrinterTransport, TcpTransport, UsbTransport};

#[derive(Clone)]
pub struct AppState {
    pub start_time: Instant,
    pub default_printer: String,
    pub timeout_secs: u64,
    pub api_token: String,
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
            <li><code>WS /api/v1/ws</code> - Kênh WebSocket hai chiều: nhận job in JSON realtime</li>
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

fn is_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    if state.api_token.is_empty() {
        return true;
    }
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == format!("Bearer {}", state.api_token))
        .unwrap_or(false)
}

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "success": false,
            "message": "Token API không hợp lệ hoặc thiếu. Truyền header 'Authorization: Bearer <token>'."
        })),
    )
        .into_response()
}

async fn printers_handler(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !is_authorized(&state, &headers) {
        return unauthorized_response();
    }

    let mut printers = DeviceScanner::scan_usb();
    let mut candidates: Vec<String> = vec!["127.0.0.1:9100".to_string()];
    if !state.default_printer.is_empty() && !state.default_printer.starts_with("usb://") {
        candidates.push(state.default_printer.clone());
    }
    let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
    printers.extend(DeviceScanner::scan_network(&refs).await);
    Json(printers).into_response()
}

async fn print_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<PrintRequest>,
) -> Response {
    if !is_authorized(&state, &headers) {
        return unauthorized_response();
    }

    Json(execute_print(&state, req.target.as_deref(), req.document, req.raw).await).into_response()
}

async fn execute_print(
    state: &AppState,
    target_override: Option<&str>,
    document: Option<PrintDocument>,
    raw: Option<String>,
) -> PrintResponse {
    let target = target_override
        .filter(|s| !s.is_empty())
        .unwrap_or(&state.default_printer);

    if target.is_empty() {
        return PrintResponse {
            success: false,
            message: "Chưa cấu hình máy in mục tiêu (target).".to_string(),
            bytes_sent: 0,
        };
    }

    let raw_bytes = if let Some(doc) = document {
        doc.compile_escpos()
    } else if let Some(raw) = raw {
        raw.into_bytes()
    } else {
        return PrintResponse {
            success: false,
            message: "Thiếu dữ liệu in (document hoặc raw).".to_string(),
            bytes_sent: 0,
        };
    };

    let len = raw_bytes.len();

    if target.starts_with("usb://") {
        match UsbTransport::open_uri(target) {
            Ok(mut usb) => match usb.send_raw(&raw_bytes).await {
                Ok(_) => PrintResponse {
                    success: true,
                    message: format!("Đã in thành công {} bytes qua USB Direct", len),
                    bytes_sent: len,
                },
                Err(e) => PrintResponse {
                    success: false,
                    message: format!("Lỗi ghi USB: {}", e),
                    bytes_sent: 0,
                },
            },
            Err(e) => PrintResponse {
                success: false,
                message: format!("Không thể mở thiết bị USB: {}", e),
                bytes_sent: 0,
            },
        }
    } else {
        match TcpTransport::connect(target, state.timeout_secs).await {
            Ok(mut tcp) => match tcp.send_raw(&raw_bytes).await {
                Ok(_) => PrintResponse {
                    success: true,
                    message: format!("Đã in thành công {} bytes qua TCP JetDirect", len),
                    bytes_sent: len,
                },
                Err(e) => PrintResponse {
                    success: false,
                    message: format!("Lỗi gửi mạng: {}", e),
                    bytes_sent: 0,
                },
            },
            Err(e) => PrintResponse {
                success: false,
                message: format!("Không thể kết nối máy in mạng {}: {}", target, e),
                bytes_sent: 0,
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum WsCommand {
    Print {
        target: Option<String>,
        document: Option<PrintDocument>,
        raw: Option<String>,
    },
}

#[derive(Deserialize)]
struct WsAuthQuery {
    token: Option<String>,
}

async fn ws_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WsAuthQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if !state.api_token.is_empty() && query.token.as_deref() != Some(state.api_token.as_str()) {
        return (StatusCode::UNAUTHORIZED, "Token API không hợp lệ").into_response();
    }
    let handler_state = (*state).clone();
    ws.on_upgrade(move |socket| handle_socket(handler_state, socket))
}

async fn handle_socket(state: AppState, mut socket: WebSocket) {
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
        if let Message::Text(text) = msg {
            if text.contains("ping") {
                let _ = socket
                    .send(Message::Text("{\"event\":\"pong\"}".into()))
                    .await;
                continue;
            }

            match serde_json::from_str::<WsCommand>(&text) {
                Ok(WsCommand::Print {
                    target,
                    document,
                    raw,
                }) => {
                    let result = execute_print(&state, target.as_deref(), document, raw).await;
                    let reply = serde_json::json!({
                        "event": "print_result",
                        "success": result.success,
                        "message": result.message,
                        "bytes_sent": result.bytes_sent,
                    });
                    let _ = socket.send(Message::Text(reply.to_string().into())).await;
                }
                Err(e) => {
                    let reply = serde_json::json!({
                        "event": "error",
                        "message": format!("JSON lệnh không hợp lệ: {}", e),
                    });
                    let _ = socket.send(Message::Text(reply.to_string().into())).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_state(api_token: &str) -> AppState {
        AppState {
            start_time: Instant::now(),
            default_printer: String::new(),
            timeout_secs: 5,
            api_token: api_token.to_string(),
        }
    }

    fn print_request_body() -> Body {
        Body::from(r#"{"raw":"test"}"#)
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let app = create_router(test_state(""));
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
        let app = create_router(test_state(""));
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cors_headers_present() {
        let app = create_router(test_state(""));
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
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .body(print_request_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_print_requires_token_when_configured() {
        let app = create_router(test_state("secret123"));

        let no_token = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .body(print_request_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(no_token.status(), StatusCode::UNAUTHORIZED);

        let wrong_token = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .header("Authorization", "Bearer sai-token")
                    .body(print_request_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong_token.status(), StatusCode::UNAUTHORIZED);

        let correct_token = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .header("Authorization", "Bearer secret123")
                    .body(print_request_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(correct_token.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_printers_requires_token_when_configured() {
        let app = create_router(test_state("secret123"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/printers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_ws_command_deserialization() {
        let payload = r#"{"event":"print","raw":"ESC @ test"}"#;
        let parsed =
            serde_json::from_str::<WsCommand>(payload).expect("lệnh print qua WS phải parse được");
        let WsCommand::Print {
            raw,
            target,
            document,
        } = parsed;
        assert_eq!(raw.as_deref(), Some("ESC @ test"));
        assert!(target.is_none());
        assert!(document.is_none());
    }
}
