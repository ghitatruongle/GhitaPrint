use axum::{
    Router,
    extract::{
        DefaultBodyLimit, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Json, Response},
    routing::{get, post},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::Instant;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::discovery::DeviceScanner;
use crate::encoder::PrintDocument;
use crate::status::{self, JobSummary};
use crate::transport::{PrinterTransport, TcpTransport, UsbTransport};

pub const WS_MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const WS_IDLE_TIMEOUT_SECS: u64 = 300;
pub const MAX_JOBS_IN_MEMORY: usize = 200;
pub const MAX_PRINTER_LOCKS: usize = 256;
pub const MAX_CONCURRENT_WS_PRINTS: usize = 8;

#[derive(Clone)]
pub struct AppState {
    pub start_time: Instant,
    pub default_printer: String,
    pub timeout_secs: u64,
    pub api_token: String,
    pub diacritics: String,
    pub protocol: String,
    pub printer_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    pub jobs: Arc<Mutex<VecDeque<JobRecord>>>,
    pub last_error: Arc<Mutex<Option<JobRecord>>>,
    pub prints_total: Arc<AtomicU64>,
    pub prints_ok: Arc<AtomicU64>,
    pub prints_failed: Arc<AtomicU64>,
    pub ws_print_permits: Arc<Semaphore>,
    pub next_job_id: Arc<std::sync::atomic::AtomicU32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobRecord {
    pub id: u32,
    pub created_at_ms: u64,
    pub source: String,
    pub target: String,
    pub mode: String,
    pub success: bool,
    pub code: String,
    pub message: String,
    pub bytes_sent: usize,
}

impl AppState {
    pub fn new(
        default_printer: String,
        timeout_secs: u64,
        api_token: String,
        diacritics: String,
        protocol: String,
    ) -> Self {
        Self {
            start_time: Instant::now(),
            default_printer,
            timeout_secs,
            api_token,
            diacritics,
            protocol,
            printer_locks: Arc::new(Mutex::new(HashMap::new())),
            jobs: Arc::new(Mutex::new(VecDeque::new())),
            last_error: Arc::new(Mutex::new(None)),
            prints_total: Arc::new(AtomicU64::new(0)),
            prints_ok: Arc::new(AtomicU64::new(0)),
            prints_failed: Arc::new(AtomicU64::new(0)),
            ws_print_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_WS_PRINTS)),
            next_job_id: Arc::new(std::sync::atomic::AtomicU32::new(1)),
        }
    }

    pub async fn printer_lock(&self, target: &str) -> Arc<Mutex<()>> {
        let mut locks = self.printer_locks.lock().await;
        if locks.len() >= MAX_PRINTER_LOCKS {
            locks.retain(|_, lock| Arc::strong_count(lock) > 1);
        }
        locks
            .entry(target.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub async fn release_printer_lock(&self, target: &str) {
        let mut locks = self.printer_locks.lock().await;
        if let Some(lock) = locks.get(target)
            && Arc::strong_count(lock) == 1
        {
            locks.remove(target);
        }
    }

    pub fn allocate_job_id(&self) -> u32 {
        self.next_job_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record_print(
        &self,
        source: &str,
        target: &str,
        mode: &str,
        success: bool,
        code: &str,
        message: &str,
        bytes_sent: usize,
    ) -> JobRecord {
        let record = JobRecord {
            id: self.allocate_job_id(),
            created_at_ms: status::now_ms(),
            source: source.to_string(),
            target: target.to_string(),
            mode: mode.to_string(),
            success,
            code: code.to_string(),
            message: message.to_string(),
            bytes_sent,
        };
        let mut jobs = self.jobs.lock().await;
        jobs.push_back(record.clone());
        while jobs.len() > MAX_JOBS_IN_MEMORY {
            jobs.pop_front();
        }
        drop(jobs);
        if success {
            self.prints_ok.fetch_add(1, Ordering::SeqCst);
        } else {
            self.prints_failed.fetch_add(1, Ordering::SeqCst);
            *self.last_error.lock().await = Some(record.clone());
        }
        self.prints_total.fetch_add(1, Ordering::SeqCst);
        self.publish_status().await;
        record
    }

    async fn publish_status(&self) {
        if !status::is_status_initialized() {
            return;
        }
        let jobs_for_file: Vec<JobSummary> = {
            let jobs = self.jobs.lock().await;
            jobs.iter()
                .rev()
                .take(status::STATUS_JOBS_IN_FILE)
                .map(|record| JobSummary {
                    id: record.id,
                    created_at_ms: record.created_at_ms,
                    source: record.source.clone(),
                    target: record.target.clone(),
                    mode: record.mode.clone(),
                    success: record.success,
                    code: record.code.clone(),
                    message: record.message.clone(),
                    bytes_sent: record.bytes_sent as u64,
                })
                .collect()
        };
        let last_error_summary = self
            .last_error
            .lock()
            .await
            .clone()
            .map(|record| job_record_to_summary(&record));
        let snapshot = status::StatusSnapshot {
            version: env!("CARGO_PKG_VERSION").to_string(),
            pid: std::process::id(),
            api_port: status::api_port(),
            boot_started_at_ms: status::boot_started_ms(),
            ready_at_ms: status::ready_at_ms(),
            ready_after_ms: status::boot_to_ready_ms(),
            prints_total: self.prints_total.load(Ordering::SeqCst),
            prints_ok: self.prints_ok.load(Ordering::SeqCst),
            prints_failed: self.prints_failed.load(Ordering::SeqCst),
            last_error: last_error_summary,
            jobs: jobs_for_file,
            virtual_printer: status::current_virtual_printer_check(),
            updated_at_ms: status::now_ms(),
        };
        if let Err(e) = status::write_status(&snapshot) {
            tracing::warn!("Không thể cập nhật status.json: {}", e);
        }
    }
}

fn job_record_to_summary(record: &JobRecord) -> JobSummary {
    JobSummary {
        id: record.id,
        created_at_ms: record.created_at_ms,
        source: record.source.clone(),
        target: record.target.clone(),
        mode: record.mode.clone(),
        success: record.success,
        code: record.code.clone(),
        message: record.message.clone(),
        bytes_sent: record.bytes_sent as u64,
    }
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub ready_after_ms: Option<u64>,
}

#[derive(Deserialize, Clone)]
pub struct PrintRequest {
    pub target: Option<String>,
    pub document: Option<PrintDocument>,
    pub raw: Option<String>,
    pub raw_base64: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct PrintResponse {
    pub success: bool,
    pub code: String,
    pub message: String,
    pub bytes_sent: usize,
    pub used_target: String,
    pub mode: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub transport_failed: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub timed_out: bool,
}

impl PrintResponse {
    fn failure(code: &str, message: &str, target: &str, mode: &str) -> Self {
        Self {
            success: false,
            code: code.to_string(),
            message: message.to_string(),
            bytes_sent: 0,
            used_target: target.to_string(),
            mode: mode.to_string(),
            transport_failed: false,
            timed_out: false,
        }
    }

    fn transport_failure(message: &str, target: &str, mode: &str, timed_out: bool) -> Self {
        Self {
            success: false,
            code: if timed_out {
                "timeout".to_string()
            } else {
                "transport_error".to_string()
            },
            message: message.to_string(),
            bytes_sent: 0,
            used_target: target.to_string(),
            mode: mode.to_string(),
            transport_failed: true,
            timed_out,
        }
    }
}

fn is_local_web_origin(origin: &HeaderValue) -> bool {
    origin
        .to_str()
        .ok()
        .and_then(|o| {
            o.strip_prefix("http://")
                .or_else(|| o.strip_prefix("https://"))
        })
        .map(|host_port| {
            let host = host_port
                .rsplit_once(':')
                .map(|(h, _)| h)
                .unwrap_or(host_port);
            let host = host.trim_start_matches('[').trim_end_matches(']');
            host == "localhost" || host == "127.0.0.1" || host == "::1"
        })
        .unwrap_or(false)
}

fn is_local_host_header(host: &str) -> bool {
    let hostname = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    let hostname = hostname.trim_start_matches('[').trim_end_matches(']');
    hostname == "localhost" || hostname == "127.0.0.1" || hostname == "::1"
}

async fn host_guard(request: axum::extract::Request, next: Next) -> Response {
    if let Some(host) = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        && !is_local_host_header(host)
    {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "success": false,
                "code": "forbidden_host",
                "message": "Host không hợp lệ — Local API chỉ phục vụ máy cục bộ"
            })),
        )
            .into_response();
    }
    next.run(request).await
}

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _parts| {
            is_local_web_origin(origin)
        }))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ]);

    Router::new()
        .route("/", get(welcome_handler))
        .route("/api/v1/health", get(health_handler))
        .route("/api/v1/printers", get(printers_handler))
        .route("/api/v1/print", post(print_handler))
        .route("/api/v1/print/raw", post(print_raw_handler))
        .route("/api/v1/jobs", get(jobs_handler))
        .route("/printers/ghitaprint", post(ipp_handler))
        .route("/api/v1/ws", get(ws_handler))
        .fallback(api_fallback)
        .layer(middleware::from_fn(host_guard))
        .layer(cors)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state(Arc::new(state))
}

async fn api_fallback() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "success": false,
            "code": "not_found",
            "message": "Đường dẫn không tồn tại — xem GET / để biết danh sách endpoint"
        })),
    )
        .into_response()
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
            <li><code>GET /api/v1/printers</code> - Danh sách máy in USB &amp; Mạng kết nối</li>
            <li><code>POST /api/v1/print</code> - Gửi lệnh in (JSON Document, raw text hoặc raw_base64)</li>
            <li><code>POST /api/v1/print/raw</code> - Gửi dữ liệu nhị phân thô (application/octet-stream, ?target=)</li>
            <li><code>WS /api/v1/ws</code> - Kênh WebSocket hai chiều: nhận job in JSON realtime</li>
        </ul>
        <p style="color:#94a3b8">Khi khóa <code>api_token</code> được đặt trong cấu hình, các endpoint in đòi hỏi header <code>Authorization: Bearer &lt;token&gt;</code> (WS: query <code>?token=</code> hoặc header Authorization).</p>
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
        ready_after_ms: status::boot_to_ready_ms(),
    })
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u64;
    let max = a.len().max(b.len());
    for i in 0..max {
        let x = a.get(i).copied().unwrap_or(0) as u64;
        let y = b.get(i).copied().unwrap_or(0) as u64;
        diff |= x ^ y;
    }
    diff == 0
}

fn is_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    if state.api_token.is_empty() {
        return true;
    }
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| constant_time_eq(t, &state.api_token))
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

fn browser_origin_host(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .and_then(|origin| origin.strip_prefix("http://").or_else(|| origin.strip_prefix("https://")))
        .map(|host_port| {
            let host = host_port
                .rsplit_once(':')
                .map(|(host, _)| host)
                .unwrap_or(host_port);
            host.trim_start_matches('[').trim_end_matches(']').to_string()
        })
}

fn is_local_origin_host(host: &str) -> bool {
    host == "localhost" || host == "127.0.0.1" || host == "::1"
}

fn is_browser_request(headers: &HeaderMap) -> bool {
    headers.get(axum::http::header::ORIGIN).is_some()
        || headers
            .get("sec-fetch-site")
            .and_then(|value| value.to_str().ok())
            .is_some()
}

fn is_cross_site_browser(headers: &HeaderMap) -> bool {
    if let Some(origin_host) = browser_origin_host(headers) {
        return !is_local_origin_host(&origin_host);
    }
    headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .map(|site| site.eq_ignore_ascii_case("cross-site"))
        .unwrap_or(false)
}

fn cross_site_blocked_response() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({
            "success": false,
            "code": "cross_site_blocked",
            "message": "Yêu cầu đến từ website khác (cross-site) — bị chặn chống CSRF. Web POS cục bộ hãy dùng Origin localhost hoặc Authorization: Bearer <token>."
        })),
    )
        .into_response()
}

fn enforce_cross_site_policy(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    if !is_cross_site_browser(headers) {
        return None;
    }
    if state.api_token.is_empty() {
        return Some(cross_site_blocked_response());
    }
    if is_authorized(state, headers) {
        None
    } else {
        Some(unauthorized_response())
    }
}

async fn printers_handler(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !is_authorized(&state, &headers) {
        return unauthorized_response();
    }

    let usb_printers = tokio::task::spawn_blocking(DeviceScanner::scan_usb)
        .await
        .unwrap_or_default();
    let mut printers = usb_printers;

    let mut candidates: Vec<String> = Vec::new();
    if !state.default_printer.is_empty() && !state.default_printer.starts_with("usb://") {
        candidates.push(state.default_printer.clone());
    }
    let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
    printers.extend(DeviceScanner::scan_network(&refs).await);
    Json(printers).into_response()
}

fn prepare_print_payload(
    req: PrintRequest,
    state: &AppState,
) -> Result<(Vec<u8>, String), PrintResponse> {
    let provided =
        req.document.is_some() as u8 + req.raw.is_some() as u8 + req.raw_base64.is_some() as u8;
    if provided > 1 {
        return Err(PrintResponse::failure(
            "bad_request",
            "Chỉ được cung cấp một trong: document, raw hoặc raw_base64",
            "",
            "",
        ));
    }

    if let Some(doc) = req.document {
        let bytes = doc
            .compile(&state.protocol, &state.diacritics)
            .map_err(|e| PrintResponse::failure("validation_error", &e, "", "document"))?;
        if bytes.is_empty() {
            return Err(PrintResponse::failure(
                "validation_error",
                "Tài liệu biên dịch ra dữ liệu in rỗng",
                "",
                "document",
            ));
        }
        return Ok((bytes, "document".to_string()));
    }

    if let Some(b64) = req.raw_base64 {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| {
                PrintResponse::failure(
                    "validation_error",
                    &format!("raw_base64 không hợp lệ: {}", e),
                    "",
                    "raw",
                )
            })?;
        if bytes.is_empty() {
            return Err(PrintResponse::failure(
                "validation_error",
                "raw_base64 trỏ tới payload rỗng",
                "",
                "raw",
            ));
        }
        return Ok((bytes, "raw".to_string()));
    }

    if let Some(raw) = req.raw {
        if raw.is_empty() {
            return Err(PrintResponse::failure(
                "validation_error",
                "raw trỏ tới payload rỗng",
                "",
                "raw",
            ));
        }
        return Ok((raw.into_bytes(), "raw".to_string()));
    }

    Err(PrintResponse::failure(
        "validation_error",
        "Thiếu dữ liệu in (document, raw hoặc raw_base64)",
        "",
        "",
    ))
}

async fn print_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    payload: Result<Json<PrintRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if !is_authorized(&state, &headers) {
        return unauthorized_response();
    }
    if let Some(blocked) = enforce_cross_site_policy(&state, &headers) {
        return blocked;
    }

    let Json(req) = match payload {
        Ok(parsed) => parsed,
        Err(rejection) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "success": false,
                    "code": "malformed_json",
                    "message": format!("JSON body không hợp lệ: {}", rejection.body_text())
                })),
            )
                .into_response();
        }
    };

    let (bytes, mode) = match prepare_print_payload(req.clone(), &state) {
        Ok(prepared) => prepared,
        Err(resp) => {
            state
                .record_print("rest", "", &resp.mode, false, &resp.code, &resp.message, 0)
                .await;
            let status = if resp.code == "validation_error" {
                StatusCode::UNPROCESSABLE_ENTITY
            } else {
                StatusCode::BAD_REQUEST
            };
            return (status, Json(resp)).into_response();
        }
    };

    let outcome = execute_print(&state, "rest", req.target.as_deref(), &bytes, &mode).await;
    let status = if outcome.success {
        StatusCode::OK
    } else if outcome.timed_out {
        StatusCode::GATEWAY_TIMEOUT
    } else if outcome.transport_failed {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    };
    (status, Json(outcome)).into_response()
}

async fn print_raw_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RawTargetQuery>,
    body: axum::body::Bytes,
) -> Response {
    if !is_authorized(&state, &headers) {
        return unauthorized_response();
    }
    if let Some(blocked) = enforce_cross_site_policy(&state, &headers) {
        return blocked;
    }

    if body.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(PrintResponse::failure(
                "validation_error",
                "Body rỗng — không có dữ liệu để in",
                "",
                "raw",
            )),
        )
            .into_response();
    }

    let target = query.target.as_deref();
    let outcome = execute_print(&state, "rest", target, &body, "raw").await;
    let status = if outcome.success {
        StatusCode::OK
    } else if outcome.timed_out {
        StatusCode::GATEWAY_TIMEOUT
    } else if outcome.transport_failed {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    };
    (status, Json(outcome)).into_response()
}

#[derive(Deserialize)]
struct RawTargetQuery {
    target: Option<String>,
}

async fn ipp_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if is_browser_request(&headers) {
        if state.api_token.is_empty() {
            return cross_site_blocked_response();
        }
        if !is_authorized(&state, &headers) {
            return unauthorized_response();
        }
    }
    let (response_bytes, log_line) = crate::server::ipp::handle_ipp_request(&state, &body).await;
    tracing::info!("{}", log_line);
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/ipp")],
        response_bytes,
    )
        .into_response()
}

async fn jobs_handler(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !is_authorized(&state, &headers) {
        return unauthorized_response();
    }
    let jobs = state.jobs.lock().await;
    let listed: Vec<JobRecord> = jobs.iter().rev().take(100).cloned().collect();
    let total = state.prints_total.load(Ordering::SeqCst);
    Json(serde_json::json!({
        "jobs": listed,
        "count": listed.len(),
        "prints_total": total,
    }))
    .into_response()
}

pub async fn execute_print(
    state: &AppState,
    source: &str,
    target_override: Option<&str>,
    raw_bytes: &[u8],
    mode: &str,
) -> PrintResponse {
    let target = target_override
        .filter(|s| !s.is_empty())
        .unwrap_or(&state.default_printer)
        .to_string();

    if target.is_empty() {
        let response = PrintResponse::failure(
            "validation_error",
            "Chưa cấu hình máy in mục tiêu (target hoặc default_printer)",
            "",
            mode,
        );
        state
            .record_print(source, "", mode, false, &response.code, &response.message, 0)
            .await;
        return response;
    }

    let len = raw_bytes.len();
    let lock = state.printer_lock(&target).await;
    let _guard = lock.lock().await;

    let outcome = if target.starts_with("usb://") {
        let bytes = raw_bytes.to_vec();
        let target_for_task = target.clone();
        let opened =
            tokio::task::spawn_blocking(move || UsbTransport::open_uri(&target_for_task)).await;
        match opened {
            Ok(Ok(mut usb)) => {
                usb.timeout_secs = state.timeout_secs.max(30);
                match usb.send_raw(&bytes).await {
                    Ok(()) => PrintResponse {
                        success: true,
                        code: "ok".to_string(),
                        message: format!("Đã in thành công {} bytes qua USB Direct", len),
                        bytes_sent: len,
                        used_target: target.clone(),
                        mode: mode.to_string(),
                        transport_failed: false,
                        timed_out: false,
                    },
                    Err(e) => PrintResponse::transport_failure(
                        &format!("Lỗi ghi USB: {}", e),
                        &target,
                        mode,
                        matches!(e, crate::transport::TransportError::Timeout(_)),
                    ),
                }
            }
            Ok(Err(e)) => PrintResponse::transport_failure(
                &format!("Không thể mở thiết bị USB: {}", e),
                &target,
                mode,
                false,
            ),
            Err(e) => PrintResponse::transport_failure(
                &format!("Lỗi task mở USB: {}", e),
                &target,
                mode,
                false,
            ),
        }
    } else {
        match TcpTransport::connect(&target, state.timeout_secs).await {
            Ok(mut tcp) => match tcp.send_raw(raw_bytes).await {
                Ok(()) => PrintResponse {
                    success: true,
                    code: "ok".to_string(),
                    message: format!("Đã in thành công {} bytes qua TCP JetDirect", len),
                    bytes_sent: len,
                    used_target: target.clone(),
                    mode: mode.to_string(),
                    transport_failed: false,
                    timed_out: false,
                },
                Err(e) => PrintResponse::transport_failure(
                    &format!("Lỗi gửi mạng: {}", e),
                    &target,
                    mode,
                    matches!(e, crate::transport::TransportError::Timeout(_)),
                ),
            },
            Err(e) => PrintResponse::transport_failure(
                &format!("Không thể kết nối máy in mạng {}: {}", target, e),
                &target,
                mode,
                matches!(e, crate::transport::TransportError::Timeout(_)),
            ),
        }
    };

    drop(_guard);
    state.release_printer_lock(&target).await;

    tracing::info!(
        target = %outcome.used_target,
        source = %source,
        mode = %mode,
        success = outcome.success,
        bytes = outcome.bytes_sent,
        "{}",
        outcome.message
    );

    state
        .record_print(
            source,
            &outcome.used_target,
            mode,
            outcome.success,
            &outcome.code,
            &outcome.message,
            outcome.bytes_sent,
        )
        .await;

    outcome
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum WsCommand {
    Print {
        target: Option<String>,
        document: Option<PrintDocument>,
        raw: Option<String>,
        raw_base64: Option<String>,
        #[serde(default)]
        id: Option<serde_json::Value>,
    },
}

#[derive(Deserialize)]
struct WsAuthQuery {
    token: Option<String>,
}

fn ws_token_ok(state: &AppState, headers: &HeaderMap, query_token: Option<&str>) -> bool {
    if state.api_token.is_empty() {
        return true;
    }
    if query_token
        .map(|t| constant_time_eq(t, &state.api_token))
        .unwrap_or(false)
    {
        return true;
    }
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| constant_time_eq(t, &state.api_token))
        .unwrap_or(false)
}

async fn ws_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if !ws_token_ok(&state, &headers, query.token.as_deref()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "success": false,
                "code": "auth_error",
                "message": "Token API không hợp lệ hoặc thiếu. Truyền query '?token=' hoặc header 'Authorization: Bearer <token>'."
            })),
        )
            .into_response();
    }
    let handler_state = (*state).clone();
    ws.max_message_size(WS_MAX_MESSAGE_BYTES)
        .max_frame_size(WS_MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(handler_state, socket))
}

async fn handle_socket(state: AppState, mut socket: WebSocket) {
    let connected = serde_json::json!({
        "event": "connected",
        "protocol": 1,
        "version": env!("CARGO_PKG_VERSION"),
        "message": "GhitaPrint WebSocket Channel Ready"
    });
    if socket
        .send(Message::Text(connected.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

    loop {
        let next_incoming = tokio::time::timeout(
            std::time::Duration::from_secs(WS_IDLE_TIMEOUT_SECS),
            socket.recv(),
        );
        let next_outgoing = rx.recv();

        let (incoming, mut outgoing) = tokio::select! {
            result = next_incoming => match result {
                Err(_elapsed) => {
                    let _ = socket
                        .send(Message::Text(
                            r#"{"event":"closed","reason":"idle_timeout"}"#.into(),
                        ))
                        .await;
                    break;
                }
                Ok(None) => (None, None),
                Ok(Some(Err(_))) => (None, None),
                Ok(Some(Ok(msg))) => (Some(msg), None),
            },
            Some(out) = next_outgoing => (None, Some(out)),
        };

        if let Some(out) = outgoing.take()
            && socket.send(out).await.is_err()
        {
            return;
        }

        let Some(incoming) = incoming else {
            if rx.is_empty() && outgoing.is_none() {
                break;
            }
            continue;
        };

        match incoming {
            Message::Text(text) => handle_ws_text(&state, &text, &tx),
            Message::Binary(_) => {
                let _ = tx.send(Message::Text(
                    serde_json::json!({
                        "event": "error",
                        "code": "unsupported_frame",
                        "message": "Chỉ hỗ trợ Text frame — hãy gửi JSON dạng Text hoặc dùng REST /api/v1/print/raw cho dữ liệu nhị phân"
                    })
                    .to_string()
                    .into(),
                ));
            }
            _ => {}
        }
    }
}

fn classify_ping(text: &str) -> bool {
    text.trim() == "ping"
}

fn handle_ws_text(state: &AppState, text: &str, tx: &tokio::sync::mpsc::UnboundedSender<Message>) {
    if classify_ping(text) {
        let _ = tx.send(Message::Text(r#"{"event":"pong"}"#.into()));
        return;
    }

    match serde_json::from_str::<WsCommand>(text) {
        Ok(WsCommand::Print {
            target,
            document,
            raw,
            raw_base64,
            id,
        }) => {
            let target_capture = target.clone();
            let request = PrintRequest {
                target,
                document,
                raw,
                raw_base64,
            };
            let state_clone = state.clone();
            let tx_clone = tx.clone();
            let permit = match state_clone.ws_print_permits.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    let _ = tx.send(Message::Text(
                        serde_json::json!({
                            "event": "error",
                            "code": "busy",
                            "message": format!("Đang có quá nhiều lệnh in đồng thời (giới hạn {}), thử lại sau.", MAX_CONCURRENT_WS_PRINTS)
                        })
                        .to_string()
                        .into(),
                    ));
                    return;
                }
            };
            tokio::spawn(async move {
                let _permit = permit;
                let (bytes, mode) = match prepare_print_payload(request, &state_clone) {
                    Ok(prepared) => prepared,
                    Err(resp) => {
                        state_clone
                            .record_print("ws", "", &resp.mode, false, &resp.code, &resp.message, 0)
                            .await;
                        let _ = tx_clone.send(Message::Text(
                            serde_json::json!({
                                "event": "print_result",
                                "id": id,
                                "success": false,
                                "code": resp.code,
                                "message": resp.message,
                                "bytes_sent": 0
                            })
                            .to_string()
                            .into(),
                        ));
                        return;
                    }
                };
                let outcome = execute_print(
                    &state_clone,
                    "ws",
                    target_capture.as_deref(),
                    &bytes,
                    &mode,
                )
                .await;
                let _ = tx_clone.send(Message::Text(
                    serde_json::json!({
                        "event": "print_result",
                        "id": id,
                        "success": outcome.success,
                        "code": outcome.code,
                        "message": outcome.message,
                        "bytes_sent": outcome.bytes_sent,
                        "target": outcome.used_target,
                        "mode": outcome.mode
                    })
                    .to_string()
                    .into(),
                ));
            });
        }
        Err(e) => {
            let _ = tx.send(Message::Text(
                serde_json::json!({
                    "event": "error",
                    "code": "invalid_json",
                    "message": format!("JSON lệnh không hợp lệ: {}", e)
                })
                .to_string()
                .into(),
            ));
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
        AppState::new(
            String::new(),
            5,
            api_token.to_string(),
            "auto".to_string(),
            "escpos".to_string(),
        )
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
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "validation_error");
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
        assert_eq!(correct_token.status(), StatusCode::UNPROCESSABLE_ENTITY);
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
            raw_base64,
            id,
        } = parsed;
        assert_eq!(raw.as_deref(), Some("ESC @ test"));
        assert!(target.is_none());
        assert!(document.is_none());
        assert!(raw_base64.is_none());
        assert!(id.is_none());
    }

    #[test]
    fn test_ws_ping_exact_match_only() {
        assert!(classify_ping("ping"));
        assert!(classify_ping("  ping  "));
        assert!(!classify_ping("shipping"));
        assert!(!classify_ping("{\"event\":\"print\",\"raw\":\"ping\"}"));
        assert!(!classify_ping("PONG"));
    }

    #[tokio::test]
    async fn test_malformed_json_returns_json_envelope() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .body(Body::from("{không phải json"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "malformed_json");
        assert_eq!(json["success"], false);
    }

    #[tokio::test]
    async fn test_document_and_raw_conflict_rejected() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        r#"{"raw":"abc","raw_base64":"YWJj","document":{"title":null,"commands":[]}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["message"]
                .as_str()
                .unwrap()
                .contains("Chỉ được cung cấp một")
        );
    }

    #[tokio::test]
    async fn test_empty_raw_payload_rejected() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"raw":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_unknown_path_returns_json_404() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/không-tồn-tại")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "not_found");
    }

    #[tokio::test]
    async fn test_foreign_host_header_rejected() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .header("Host", "evil.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_cors_blocks_foreign_origin() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .header("Origin", "http://evil.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );
    }

    #[tokio::test]
    async fn test_oom_width_body_rejected_not_abort() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        r#"{"target":"127.0.0.1:1","document":{"title":null,"commands":[{"type":"table","columns":[{"title":"a","width":1000000000}],"rows":[["x"]]}]}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["message"].as_str().unwrap().contains("vượt giới hạn"));
    }

    #[tokio::test]
    async fn test_raw_base64_end_to_end_over_tcp() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; 1024];
            let n = socket.read(&mut buf).await.unwrap();
            buf.truncate(n);
            buf
        });

        let mut state = test_state("");
        state.default_printer = format!("127.0.0.1:{}", port);
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"raw_base64":"VEVTVF9QUklOVF9EQVRB"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let received = server_task.await.unwrap();
        assert_eq!(&received, b"TEST_PRINT_DATA");
    }

    #[tokio::test]
    async fn test_print_raw_octet_stream_endpoint() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; 1024];
            let n = socket.read(&mut buf).await.unwrap();
            buf.truncate(n);
            buf
        });

        let mut state = test_state("");
        state.default_printer = format!("127.0.0.1:{}", port);
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/print/raw?target=127.0.0.1:{}", port))
                    .header("Content-Type", "application/octet-stream")
                    .body(Body::from(b"RAW_BYTES_OK".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let received = server_task.await.unwrap();
        assert_eq!(&received, b"RAW_BYTES_OK");
    }

    #[tokio::test]
    async fn test_health_has_no_printer_info_leak() {
        let mut state = test_state("");
        state.default_printer = "usb://0416:5011".to_string();
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
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("default_printer").is_none());
        assert_eq!(json["status"], "ok");
    }

    #[test]
    fn test_constant_time_eq_matches_std() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abcd", "abc"));
        assert!(!constant_time_eq("", "a"));
        assert!(constant_time_eq("", ""));
    }

    #[tokio::test]
    async fn test_ipp_rejects_browser_origin_without_token() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/printers/ghitaprint")
                    .header("Origin", "http://evil.example.com")
                    .body(Body::from(vec![1, 1, 0, 2, 0, 0, 0, 1]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_ipp_rejects_sec_fetch_browser_marker() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/printers/ghitaprint")
                    .header("Sec-Fetch-Site", "same-origin")
                    .body(Body::from(vec![1, 1, 0, 2, 0, 0, 0, 1]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_ipp_allows_non_browser_client_without_token() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/printers/ghitaprint")
                    .body(Body::from(vec![1, 1, 0, 2, 0, 0, 0, 1]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_ipp_allows_browser_with_valid_token() {
        let app = create_router(test_state("secret123"));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/printers/ghitaprint")
                    .header("Origin", "http://evil.example.com")
                    .header("Authorization", "Bearer secret123")
                    .body(Body::from(vec![1, 1, 0, 2, 0, 0, 0, 1]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_print_cross_site_origin_blocked_without_token() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .header("Origin", "http://evil.example.com")
                    .body(print_request_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "cross_site_blocked");
    }

    #[tokio::test]
    async fn test_print_cross_site_origin_with_wrong_token_unauthorized() {
        let app = create_router(test_state("secret123"));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .header("Origin", "http://evil.example.com")
                    .body(print_request_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_print_cross_site_origin_with_valid_token_ok() {
        let app = create_router(test_state("secret123"));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .header("Origin", "http://evil.example.com")
                    .header("Authorization", "Bearer secret123")
                    .body(print_request_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::FORBIDDEN);
        assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_print_localhost_origin_allowed_without_token() {
        let app = create_router(test_state(""));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/print")
                    .header("Content-Type", "application/json")
                    .header("Origin", "http://localhost:3000")
                    .body(print_request_body())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "validation_error");
    }

    #[tokio::test]
    async fn test_jobs_endpoint_requires_token_when_configured() {
        let app = create_router(test_state("secret123"));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/jobs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_jobs_endpoint_lists_recorded_prints() {
        let state = test_state("");
        state
            .record_print(
                "rest", "127.0.0.1:9100", "raw", true, "ok", "In thành công", 42,
            )
            .await;
        state
            .record_print(
                "ws", "127.0.0.1:9101", "document", false, "transport_error", "Không thể kết nối", 0,
            )
            .await;
        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/jobs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 256 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 2);
        assert_eq!(json["jobs"][0]["code"], "transport_error");
        assert_eq!(json["jobs"][1]["code"], "ok");
        assert_eq!(json["prints_total"], 2);
    }

    #[tokio::test]
    async fn test_record_print_ring_buffer_caps_at_limit() {
        let state = test_state("");
        for index in 0..(MAX_JOBS_IN_MEMORY + 50) {
            state
                .record_print(
                    "rest",
                    "127.0.0.1:9100",
                    "raw",
                    true,
                    "ok",
                    "In thành công",
                    index,
                )
                .await;
        }
        let jobs = state.jobs.lock().await;
        assert_eq!(jobs.len(), MAX_JOBS_IN_MEMORY);
        assert_eq!(jobs.back().unwrap().id, MAX_JOBS_IN_MEMORY as u32 + 50);
        assert_eq!(state.prints_total.load(Ordering::SeqCst), (MAX_JOBS_IN_MEMORY + 50) as u64);
    }

    #[tokio::test]
    async fn test_printer_lock_released_after_use() {
        let state = test_state("");
        let lock = state.printer_lock("usb://0416:5011").await;
        assert!(state.printer_locks.lock().await.contains_key("usb://0416:5011"));
        drop(lock);
        state.release_printer_lock("usb://0416:5011").await;
        assert!(!state.printer_locks.lock().await.contains_key("usb://0416:5011"));
    }

    #[test]
    fn test_cross_site_detection() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::ORIGIN,
            HeaderValue::from_static("http://evil.example.com"),
        );
        assert!(is_cross_site_browser(&headers));
        assert!(is_browser_request(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::ORIGIN,
            HeaderValue::from_static("http://localhost:3000"),
        );
        assert!(!is_cross_site_browser(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:5500"),
        );
        assert!(!is_cross_site_browser(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(
            "sec-fetch-site",
            HeaderValue::from_static("cross-site"),
        );
        assert!(is_cross_site_browser(&headers));

        let headers = HeaderMap::new();
        assert!(!is_browser_request(&headers));
        assert!(!is_cross_site_browser(&headers));
    }
}
