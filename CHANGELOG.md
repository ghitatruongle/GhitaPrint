# 📋 Changelog

Toàn bộ thay đổi đáng chú ý của **GhitaPrint** được ghi nhận trong file này.
Định dạng dựa trên [Keep a Changelog](https://keepachangelog.com/), phiên bản tuân theo [Semantic Versioning](https://semver.org/).

---

## [0.0.3-preview] — 2026-11-10

### 🛡️ Bảo mật — vá 6 lỗ hổng từ audit sâu (S1–S6)
- **S1 (cao):** IPP `/printers/ghitaprint` giờ chặn request mang dấu hiệu trình duyệt (`Origin`/`Sec-Fetch-Site`)
  khi chưa có `api_token` — hết CSRF-print từ web; spooler Windows (không gửi Origin) in bình thường.
  `/api/v1/print*` chặn cross-site origin khi chưa cấu hình token; có token thì yêu cầu token như cũ.
- **S2 (cao):** chặn zlib bomb PWG: giải nén cap 64 MB, chặn trang raster > 40 triệu điểm ảnh **trước** khi cấp
  phát RGBA (trước đây `Vec::with_capacity` tới 16 GB → abort daemon), cap tổng 256 MB raster tích lũy.
- **S3:** raw spooler listener cap payload 256 MB (trước đây stream vô hạn → OOM).
- **S4:** chống PID-reuse khi `daemon --stop`: xác minh tên process qua `QueryFullProcessImageNameW` trước khi
  `TerminateProcess`; panic hook xóa `daemon.pid`; ghi pid file cảnh báo khi ghi đè daemon sống.
- **S5:** `printer_locks`/`jobs` có giới hạn (256 lock / 200 job ring buffer); WS giới hạn 8 lệnh in đồng thời
  (phản hồi `busy` khi vượt).
- **S6:** log rotate dọn file cũ hơn 7 ngày + crash.log > 5 MB ép xoay; daemon **từ chối khởi động** khi
  `ghitaprint.toml` trong thư mục hiện tại đè config hệ thống; `spawn_detached` quote argument chuẩn Windows.

### ⚡ Khởi động tức thì
- Kiểm tra máy in ảo (PowerShell) rời khỏi đường khởi động — chạy nền **sau khi bind**, daemon chỉ cảnh báo
  tự cài lại; REST API sẵn sàng chỉ sau **~3 ms** boot→ready (đo thực tế; trước đây 1–4 s do PowerShell chặn).
- Bootstrap `daemon --start --hidden` thoát ngay sau spawn (**72 ms**, trước đây chờ health ~8 s) — hết flash
  console lúc boot; setup.exe giữ health probe riêng.
- `/api/v1/health` + status.json ghi `ready_after_ms` đo boot→ready thực tế.

### 📡 Kênh giao tiếp người dùng (status.json + watch companion)
- Mọi lệnh in REST/WS/IPP/spooler được **log đầy đủ** (trước đây `execute_print` không log) + record vào vòng
  đời job thống nhất; endpoint mới `GET /api/v1/jobs`.
- **`status.json`** tại `%APPDATA%\GhitaPrint\` ghi atomic mỗi sự kiện: pid/port/version, boot→ready, bộ đếm
  in, last_error, 20 job gần nhất, trạng thái máy in ảo.
- **`ghitaprint watch`** — companion siêu nhẹ (~455 KB, start tức thì, không runtime tokio): theo dõi
  status.json → ghi nhật ký dễ đọc `events.log` + **toast Windows** khi in lỗi; phát hiện daemon chết theo PID.
  `startup install` đăng ký kèm watch (Registry Run `GhitaPrintWatch`); bộ cài tự đăng ký cả hai.
- `ghitaprint status` tức thì từ status.json — hiện boot→ready, thống kê in, 5 job gần nhất.

### 📄 Khác
- Cargo version `0.0.3-preview`; 132 automated tests (từ 107) — thêm test cho IPP guard, zlib bomb, spooler
  cap, PID-reuse, status file, jobs API, ring buffer, quote argument; clippy 0 warning.
- `ghitaprint.exe` release ~2.4 MB; `watch.exe` ~455 KB.

### 🔍 Review & debug toàn bộ mã nguồn (pass cuối trước phát hành)
- **watch là subcommand thật:** logic watch chuyển vào library (`src/watch.rs`); `ghitaprint watch` chạy
  trực tiếp trên exe chính (trước đây `startup install` đăng ký `ghitaprint.exe watch` nhưng subcommand
  chưa tồn tại — autostart sẽ lỗi); `watch.exe` standalone giữ nguyên như wrapper mỏng.
- **status.json được ghi ngay khi daemon ready** (trước đây chỉ ghi khi có lệnh in đầu tiên — `status`/watch
  hiển thị dữ liệu cũ); `updated_at_ms` luôn được set.
- **watch không replay job cũ** khi khởi động lại (baseline theo job mới nhất, không spam events.log);
  **phát hiện daemon chết cả khi idle** (kiểm tra PID mỗi tick, trước đây chỉ kiểm tra khi status đổi);
  bỏ qua kiểm tra liveness trên non-Windows.
- Chống ghi đè chéo status.json giữa nhiều daemon (tmp file theo PID); toast Windows có throttle 3s;
  events.log tự xoay khi > 5 MB; refactor `enforce_cross_site_policy` trả `Option<Response>`;
  bỏ dead-code trong `read_connection`; xác minh `image::open` đã có giới hạn cấp phát mặc định 512 MiB
  (chặn decompression bomb từ CLI, không cần đổi code).
- 133 tests pass (129 lib + 4 setup), clippy 0 warning; installer build lại, SHA-256 mới
  `AA0F563BDD446088B009E5F0577B7238F3B15A5948D7DE057ADFDECCBE693F91`.

---

## [0.0.2] — 2026-11-07

### 🎉 Bản hoàn thiện cuối cùng của chu kỳ sản phẩm

Chu kỳ `0.0.2` kết thúc sau đợt review + debug toàn bộ mã nguồn (5 lớp: runtime · deployment · API/CLI ·
encoders · testing) với 7 lỗi P0 và hàng chục P1 được đóng, kèm kiểm chứng E2E trên máy thật.

#### 🖨️ Máy in ảo in thật được từ Word/Chrome (P0)
- Raw spooler listener: daemon lắng nghe TCP raw trên cổng api+1 (mặc định 9124), tự nhận diện payload
  (`%PDF` passthrough / IPP binary / raw ESC-POS) — in từ Word/Chrome qua máy in ảo "GhitaPrint" hoạt động thật,
  đã verify chuỗi thật: spooler Windows → PDF 46.075 bytes → daemon → máy in đích nguyên vẹn.
- Bộ cài tạo cổng TCPMON `GhitaPrintRaw` (→127.0.0.1:9124) **trước** khi Add-Printer — sửa lỗi "The specified port
  does not exist" khi cài, hoạt động cả trên tài khoản không phải Administrator.
- IPP 1.1 endpoint tại `/printers/ghitaprint` (Print-Job, Validate-Job, Create-Job/Send-Document, Cancel-Job,
  Get-Printer-Attributes trả capabilities PWG raster đầy đủ) + PWG Raster decoder (sRGB_8/sGray_8/1-bit, flate).

#### 🛡️ Daemon không bao giờ chết im lặng (P0)
- USB `bulk_out` có timeout từng chunk (trước đây treo vĩnh viễn khi máy in lỗi).
- Clamp `TableColumn.width` (1..=128) + giới hạn số phần tử — chặn OOM kill daemon từ body JSON ~70 byte.
- Single-instance qua PID file, `daemon --stop` kill đúng PID (không còn taskkill giết lan cả CLI khác).
- Bind-then-announce, graceful shutdown Ctrl+C, spawn daemon kèm health-probe trước khi báo thành công.
- File log rotate theo ngày tại `%APPDATA%\GhitaPrint\logs\` + panic hook ghi crash.log.
- Exit code đúng cho mọi error path (0/1/2/3/4) + `--json` đầy đủ 15 operations.

#### 🔐 Bảo mật Local API
- CORS whitelist localhost thay cho `Any`; validate `Host`/`Origin` (chặn DNS rebinding); WS nhận token qua
  header `Authorization` (không chỉ URL query); giới hạn message WS 1MB + idle timeout 300s; so sánh token
  constant-time; health endpoint không còn lộ cấu hình; `config set` không echo secret.

#### 📄 API & CLI
- REST: status code đúng (400/422/502/504) + envelope thống nhất có `code`, `used_target`, `mode`;
  `raw_base64` sửa lỗi hỏng byte ≥0x80 của `raw`; thêm `POST /api/v1/print/raw` (octet-stream).
- WS: correlation `id` + `protocol` version trong `connected`.
- CLI: `--config-path`, `scan --network --subnet CIDR` (tự dò /24), `config reset --yes`, `status` health-probe thật.

#### 🖋️ Encoders & tiếng Việt
- Sửa state leakage giữa các phần tử (divider sau text 2x/center bị lệch — có sẵn trong sample_bill.json).
- ESC t codepage TCVN3 cho `diacritics = keep`, fallback bỏ dấu an toàn; TSPL thêm `CODEPAGE UTF-8`, sửa
  `gap()` emit sau `CLS`, chống command injection CR/LF; `compile_tspl` + chọn protocol theo config.
- Image: mặc định clamp 576 dots, chặn ảnh cao >65.535 dots và >100 triệu pixel.

#### 💿 Bộ cài
- Verify SHA-256 + kích thước payload sau extract (chặn AV can thiệp/thiếu exe); taskkill retry + test-open;
  exit code chuẩn + `--json` summary cho MDM; broadcast `WM_SETTINGCHANGE` trực tiếp; uninstall xóa sạch
  config/token/env var/printer port; build.rs tự tạo stub + nhúng hash — không thể build bộ cài nhầm exe cũ.

#### ✅ Chất lượng
- 107 automated tests (103 lib + 4 setup) — tăng từ 36; clippy 0 warning; fmt clean; cargo audit 0 cảnh báo.
- E2E đã chạy thật: cài đè 0.0.1 trên máy, spooler Windows thật → daemon → máy in đích; IPP/PWG/TCVN3 wire-level.

#### 📦 Tải xuống
- Bộ cài 1-Click: `GhitaPrint_0.0.2.exe` (~2.86 MB) — cài thường / `--silent` / `--uninstall` / `--json`.
- SHA-256: `2d57391db69a1504147e3a65efea5a1705a09a248b911dc19ae1219c16ba26c9`

## [0.0.1] — 2026-11-05

### 🎉 Phát hành chính thức chu kỳ 0.0.1

Kết thúc chu kỳ `0.0.1` sau bản preview: các lỗi lớp cài đặt và daemon lifecycle phát hiện qua đợt phân tích sâu
toàn bộ mã nguồn đã được sửa, cộng thêm bảo mật API và WebSocket nhận job in thực tế.

#### 🛠️ Sửa lỗi bộ cài & daemon lifecycle
- Bộ cài setup.exe trước đây khởi chạy daemon kế thừa console của trình cài — cài bằng double-click thì console đóng
  là daemon bị kill ngay sau cài dù giao diện báo thành công. Daemon giờ được spawn detached, không cửa sổ, stdio
  trỏ về NUL nên sống độc lập với tiến trình cài đặt.
- Daemon tự khởi động cùng Windows trước đây chạy foreground trong Registry Run — mỗi lần boot hiện một cửa sổ console
  đen tồn tại cả phiên làm việc. Giờ `startup install` và bộ cài đăng ký lệnh `daemon --start --hidden`: tiến trình
  bootstrap re-exec daemon detached ẩn rồi thoát ngay, chỉ còn nhấp nháy dưới 1 giây lúc boot. Người dùng nâng cấp cần
  chạy lại `ghitaprint startup install` (bộ cài mới tự ghi đè registry).
- Bộ cài trước đây hardcode cổng 9123 khi đăng ký máy in ảo — cài lại trên máy đã cấu hình `api_port` khác thì máy in
  ảo trỏ sai cổng, in từ Word/Chrome thất bại âm thầm. Setup giờ đọc `api_port` từ `%APPDATA%\GhitaPrint\ghitaprint.toml`.

#### 🛠️ Bảo mật & API
- Local REST API trước đây cho phép bất kỳ website nào browser mở cũng POST được tới `/api/v1/print` (drive-by
  printing). Thêm khóa cấu hình `api_token`: khi đặt, `/api/v1/print` và `/api/v1/printers` yêu cầu
  `Authorization: Bearer <token>` (trả 401 nếu sai/thiếu); WebSocket yêu cầu `?token=<token>` khi connect. Token được
  mask trong `config list`. Không đặt token = hành vi cũ (mở cho web POS localhost).
- WebSocket `/api/v1/ws` trước đây chỉ trả pong khi nhận "ping" — giờ nhận lệnh `{"event":"print", target, document,
  raw}` và trả `{"event":"print_result", success, message, bytes_sent}`; JSON sai định dạng trả `{"event":"error"}`.
  Ping/pong giữ nguyên để tương thích.

#### 🛠️ Sửa khác
- Gỡ cờ `startup install --service` (trước đây là placeholder không làm gì) — Windows Service sẽ quay lại ở phiên bản sau.
- `print --image --image-width` giờ clamp 1..=2048 dots ở cả CLI và bên trong encoder raster — guard kép chống tràn
  `u16` và chống phân bổ bộ nhớ khổng lồ khi truyền giá trị cực lớn.
- Sửa bộ cài parse `api_port` sai khi config chứa key tương tự (ví dụ `api_port_backup = 5` trước đây bị nhận là cổng
  5) — giờ so khớp chính xác key sau dấu `=`; logic tách thành hàm thuần với 5 unit tests.
- Config `ghitaprint.toml` một phần (thiếu field) giờ vẫn giữ giá trị có sẵn thay vì reset toàn bộ về mặc định.
- Loại bỏ dependency `indicatif` không sử dụng; cập nhật `tokio` 1.53.2 → 1.54.1 — `cargo audit` sạch hoàn toàn
  (0 vulnerability, 0 warning) trên 182 crates; `cargo fmt` áp dụng toàn bộ mã nguồn.

#### 📦 Tải xuống
- Bộ cài 1-Click: `GhitaPrint_0.0.1.exe` — cài thường / `--silent` / `--uninstall`, tự thêm PATH, đăng ký tự khởi động
  ẩn, cài máy in ảo đúng cổng cấu hình và chạy daemon nền ẩn.
- SHA-256: `cfde256192d808f7d1fa7d3ea9db0830e4b83186441aad462fb3fc76158cff25`

#### ✅ Chất lượng
- 36 automated tests: 36 passed, 0 failed, 0 warnings; Clippy: 0 warning toàn bộ target; `cargo audit` 0 cảnh báo.
- E2E verified: daemon `--hidden` bind đúng cổng cấu hình, token 401/200 trên `/print` + `/printers`, WS print
  end-to-end, `config list` mask token.

## [0.0.1-preview] — 2026-10-28

### 🔍 Bản preview sau đợt review & debug toàn bộ mã nguồn

Bản phát hành preview đầu tiên của chu kỳ `0.0.1`, đánh dấu phiên bản làm việc mới sau bản phát hành chính thức `0.0.0`. Toàn bộ mã nguồn (15 tệp, ~3.400 dòng) đã được review và debug; 24 automated tests và Clippy chạy sạch hoàn toàn.

#### 🛠️ Sửa lỗi đầu ra máy in
- Bảng trong JSON Document: ô dài hơn chiều rộng cột trước đây làm trôi lệch toàn bộ layout phía sau — giờ tự cắt đúng độ rộng cột; ô thừa ngoài số cột khai báo được bỏ qua thay vì in với width mặc định 10.
- TSPL: chuỗi chứa dấu nháy kép được escape dạng `\"` không đúng chuẩn TSPL (máy in phát ra ký tự rác) — giờ thay `"` bằng `'` trong tham số có nháy.
- ESC/POS `divider` style "solid" dùng ký tự Unicode `═` (3 byte UTF-8 mỗi ký tự) khiến máy in nhiệt phát ~96 ký tự rác — đổi về ASCII chuẩn.
- USB Direct Engine: endpoint Bulk OUT trước đây hardcode `0x01` với nhiều máy in không in được — giờ tự dò địa chỉ endpoint từ USB descriptor, fallback `0x01`.

#### 🛠️ Sửa báo cáo trạng thái sai lệch
- `daemon --stop` trước đây chỉ in thông báo thành công giả — giờ thật sự kết thúc tiến trình daemon (`taskkill` loại trừ PID của chính lệnh gọi).
- `daemon --restart` trước đây không làm gì — giờ dừng daemon cũ rồi khởi chạy lại.
- `status` hiển thị hardcode "Sẵn sàng" — giờ health-check thật cổng API, báo "Đang chạy / Không chạy".
- Cài/gỡ máy in ảo thất bại trước đây bị trả về dạng thành công — giờ trả lỗi kèm thông báo từ Windows Spooler.
- `scan --bluetooth` âm thầm trả về danh sách rỗng — giờ cảnh báo tính năng chưa hỗ trợ; flag `--target` chết của lệnh `test` bị loại bỏ; help `scan --network` bỏ nhắc "mDNS" chưa triển khai.

#### 🛠️ Sửa độ bền hệ thống
- `daemon --start` trước đây mặc định hardcode cổng 9123 bỏ qua `api_port` trong cấu hình — giờ ưu tiên `api_port` khi không truyền `--port`.
- `GET /api/v1/printers` trước đây chỉ quét USB — giờ quét thêm mạng (localhost:9100 + máy in mặc định cấu hình).
- Bộ cài setup.exe ghi PATH registry bằng REG_SZ phá khả năng mở rộng `%VAR%` của PATH người dùng — giờ ghi đúng REG_EXPAND_SZ.
- Lưu cấu hình `ghitaprint.toml` trước đây ghi trực tiếp (crash giữa chừng làm hỏng file) — giờ ghi atomic qua file tạm rồi rename.

#### 📦 Tải xuống
- Bộ cài 1-Click: `GhitaPrint_0.0.1-preview.exe` (~2.45 MB) — cài thường / `--silent` / `--uninstall`, tự thêm PATH, đăng ký tự khởi động, cài máy in ảo và chạy daemon.
- SHA-256: `4c3bb59aeeee9c557292a10cdb5f83c30b64848c5c0f27a5bea085f03f49fea9`

#### ✅ Chất lượng
- 24 automated tests: 24 passed, 0 failed, 0 warnings; Clippy: 0 warning toàn bộ target.
- File bộ cài không track trong mã nguồn git (`/release` trong `.gitignore`).

## [0.0.0] — 2026-10-11

### 🎉 Phát hành chính thức đầu tiên

Bản phát hành chính thức đầu tiên của **GhitaPrint — Universal, Driverless & Ultra-fast Print Engine** viết bằng Rust, sau chu kỳ `0.0.0-preview` với toàn bộ 4 Track phát triển đã hoàn thành.

#### Track 1 — CLI Framework, Configuration & Daemon Lifecycle
- Khung lệnh CLI phân cấp với Clap v4: `scan`, `test`, `print`, `config`, `startup`, `daemon`, `virtual-printer`, `status`.
- Quản lý cấu hình tự động qua `ghitaprint.toml` (`%APPDATA%\GhitaPrint\ghitaprint.toml`).
- Đăng ký/gỡ tự khởi động cùng Windows qua Registry `HKCU\...\Run`.
- Giao diện Terminal bo góc UTF-8, cờ `--json` toàn cục.

#### Track 2 — Driverless Hardware Transports & Device Discovery
- Network JetDirect Transport: raw TCP cổng 9100, tắt Nagle, port ping an toàn.
- Driverless USB Direct Engine: Bulk OUT Endpoint qua `nusb`, không cần cài driver hãng.
- Định danh URI thống nhất: `usb://<VID>:<PID>`, `tcp://<IP>:<PORT>`.
- Bộ quét thiết bị: `ghitaprint scan`, `ghitaprint scan --all` (chẩn đoán toàn bộ cổng USB bo mạch chủ).

#### Track 3 — Universal Protocol Encoders & Graphics CLI
- Universal Document Schema (UIR): tài liệu in JSON chuẩn hóa (`text`, `table`, `divider`, `qrcode`, `feed`, `cut`).
- ESC/POS Engine: mã nhị phân đầy đủ (font size, bold, align, QR code, cắt giấy).
- TSPL Label Engine: tem nhãn mã vạch (kích thước mm, Code 128, QR, khung `BOX`).
- Floyd-Steinberg Monochrome Rasterizer: ảnh màu → 1-bit siêu mịn (`GS v 0`).
- Lệnh in thực tế: `test --receipt`, `test --label`, `print -f bill.json`, `print --image logo.png`.

#### Track 4 — Virtual Printer, Local REST/WebSocket Server & Ecosystem
- Local HTTP REST & WebSocket Server (axum): `/api/v1/health`, `/api/v1/printers`, `/api/v1/print`, `/api/v1/ws`.
- Virtual Printer Bridge: máy in ảo "GhitaPrint" trên Windows Spooler (Microsoft IPP Class Driver).
- Điều khiển nền: `ghitaprint daemon --start --port 9123`.
- Single Binary tối ưu: ~1.92 MB (LTO, strip, abort panic), RAM 5–8 MB.

#### 🛠️ Sửa lỗi trong đợt review mã nguồn phát hành
- Bộ cài setup.exe nhúng nhầm bản `ghitaprint.exe` cũ khi build song song (`include_bytes!` đọc file chưa được ghi xong) — bổ sung `build.rs` theo dõi `target/release/ghitaprint.exe`, buộc build 2 bước đúng thứ tự.
- `startup install` ghi Registry lệnh `daemon --silent` (flag không tồn tại) khiến daemon không thể tự chạy khi bật máy — đổi thành `daemon --start`.
- ESC/POS `barcode_128` tràn `u8` khi dữ liệu > 253 byte và `qrcode` tràn `u16` khi > 65.532 byte — giới hạn dữ liệu an toàn.
- Bảng trong JSON Document bỏ qua thuộc tính `align` của cột — hỗ trợ đầy đủ left/center/right.
- Gửi dữ liệu qua TCP không có timeout — request daemon có thể treo vĩnh viễn nếu máy in ngắt giữa chừng.
- Lệnh `status` hiển thị hardcode cổng 9123 thay vì `api_port` cấu hình thực tế.
- Scheme URI `tcp://<IP>:<PORT>` (chuẩn trong tài liệu kiến trúc) không được xử lý khi kết nối TCP — kết nối luôn thất bại; giờ được tự động strip prefix.
- Luồng `print --image` thiếu lệnh khởi tạo `ESC @` đầu dữ liệu — bổ sung để máy in luôn bắt đầu ở trạng thái mặc định.

#### 📦 Tải xuống
- Bộ cài 1-Click: `GhitaPrint_0.0.0.exe` — cài thường / `--silent` / `--uninstall`, tự thêm PATH, đăng ký tự khởi động, cài máy in ảo và chạy daemon.
- SHA-256: `2531dc5c39b74ada297aedae6f45ed00ddda02b988a0479e4e942d373eabb97d`

#### ✅ Chất lượng
- 23 automated tests: 23 passed, 0 failed, 0 warnings.
- File bộ cài phát hành qua **GitHub Releases** và không còn được track trong mã nguồn git.
