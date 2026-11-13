use super::ipp;
use super::routes::{AppState, execute_print};

pub const RAW_READ_TIMEOUT_SECS: u64 = 60;
pub const MAX_SPOOLER_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;

pub enum SpoolerPayload<'a> {
    Pdf(&'a [u8]),
    Ipp(&'a [u8]),
    Raw(&'a [u8]),
}

pub fn classify_spooler_payload(data: &[u8]) -> SpoolerPayload<'_> {
    if data.starts_with(b"%PDF") {
        SpoolerPayload::Pdf(data)
    } else if data.len() >= 8 && data[0] == 0x02 && data[1] == 0x00 {
        SpoolerPayload::Ipp(data)
    } else {
        SpoolerPayload::Raw(data)
    }
}

pub async fn handle_spooler_connection(state: &AppState, payload: &[u8]) -> String {
    if payload.is_empty() {
        return "Spooler connection rỗng — bỏ qua".to_string();
    }

    match classify_spooler_payload(payload) {
        SpoolerPayload::Pdf(data) => {
            let outcome = execute_print(state, "spooler", None, data, "pdf_passthrough").await;
            format!(
                "Spooler PDF passthrough: {} bytes → {}",
                data.len(),
                if outcome.success {
                    format!("in thành công ({})", outcome.used_target)
                } else {
                    format!("thất bại: {}", outcome.message)
                }
            )
        }
        SpoolerPayload::Ipp(data) => {
            let (response, log_line) = ipp::handle_ipp_request(state, data).await;
            tracing::info!("Raw IPP từ spooler: {} bytes response", response.len());
            log_line
        }
        SpoolerPayload::Raw(data) => {
            let outcome = execute_print(state, "spooler", None, data, "raw_passthrough").await;
            format!(
                "Spooler raw passthrough: {} bytes → {}",
                data.len(),
                if outcome.success {
                    format!("in thành công ({})", outcome.used_target)
                } else {
                    format!("thất bại: {}", outcome.message)
                }
            )
        }
    }
}

pub async fn run_raw_listener(state: AppState, port: u16) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    run_raw_listener_on(listener, state).await
}

pub async fn run_raw_listener_on(
    listener: tokio::net::TcpListener,
    state: AppState,
) -> std::io::Result<()> {
    tracing::info!(
        "Raw spooler listener sẵn sàng tại 127.0.0.1:{}",
        listener
            .local_addr()
            .map(|addr| addr.port())
            .unwrap_or_default()
    );

    loop {
        let (mut socket, peer) = match listener.accept().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!("Raw listener accept lỗi: {}", error);
                continue;
            }
        };

        let state_clone = state.clone();
        tokio::spawn(async move {
            let read_result = tokio::time::timeout(
                std::time::Duration::from_secs(RAW_READ_TIMEOUT_SECS),
                read_connection(&mut socket, MAX_SPOOLER_PAYLOAD_BYTES),
            )
            .await;

            let payload = match read_result {
                Ok(Ok(data)) => data,
                Ok(Err(error)) => {
                    tracing::warn!("Raw connection {} đọc lỗi: {}", peer, error);
                    return;
                }
                Err(_elapsed) => {
                    tracing::warn!(
                        "Raw connection {} hết giờ đọc ({} giây)",
                        peer,
                        RAW_READ_TIMEOUT_SECS
                    );
                    return;
                }
            };

            let log_line = handle_spooler_connection(&state_clone, &payload).await;
            tracing::info!("{}", log_line);
        });
    }
}

async fn read_connection(
    socket: &mut tokio::net::TcpStream,
    max_bytes: usize,
) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;

    let mut payload = Vec::with_capacity(64 * 1024);
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read_bytes = socket.read(&mut buffer).await?;
        if read_bytes == 0 {
            break;
        }
        if payload.len().saturating_add(read_bytes) > max_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Payload spooler vượt giới hạn {} MB — ngắt kết nối",
                    max_bytes / (1024 * 1024)
                ),
            ));
        }
        payload.extend_from_slice(&buffer[..read_bytes]);
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_pdf_by_magic() {
        let payload = b"%PDF-1.7\n...";
        assert!(matches!(
            classify_spooler_payload(payload),
            SpoolerPayload::Pdf(_)
        ));
    }

    #[test]
    fn test_classify_ipp_by_version_header() {
        let mut payload = vec![0x02u8, 0x00, 0x00, 0x02, 0, 0, 0, 1];
        payload.extend_from_slice(b"attributes");
        assert!(matches!(
            classify_spooler_payload(&payload),
            SpoolerPayload::Ipp(_)
        ));
    }

    #[test]
    fn test_classify_raw_escpos() {
        let payload = [0x1Bu8, 0x40, b'H', b'i'];
        assert!(matches!(
            classify_spooler_payload(&payload),
            SpoolerPayload::Raw(_)
        ));
    }

    #[test]
    fn test_classify_short_payload_is_raw() {
        let payload = [0x02u8];
        assert!(matches!(
            classify_spooler_payload(&payload),
            SpoolerPayload::Raw(_)
        ));
        let empty: [u8; 0] = [];
        assert!(matches!(
            classify_spooler_payload(&empty),
            SpoolerPayload::Raw(_)
        ));
    }

    #[tokio::test]
    async fn test_read_connection_enforces_payload_cap() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let writer = tokio::spawn(async move {
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            use tokio::io::AsyncWriteExt;
            let chunk = vec![0u8; 1024];
            for _ in 0..4 {
                socket.write_all(&chunk).await.unwrap();
            }
        });

        let (mut socket, _) = listener.accept().await.unwrap();
        let result = read_connection(&mut socket, 2048).await;
        assert!(result.is_err(), "vượt 2048 bytes phải bị từ chối");
        let message = result.err().unwrap().to_string();
        assert!(message.contains("vượt giới hạn"));
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn test_read_connection_accepts_payload_under_cap() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let writer = tokio::spawn(async move {
            let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            use tokio::io::AsyncWriteExt;
            socket.write_all(&[7u8; 100]).await.unwrap();
        });

        let (mut socket, _) = listener.accept().await.unwrap();
        let payload = read_connection(&mut socket, 2048)
            .await
            .expect("payload nhỏ hơn cap phải được chấp nhận");
        assert_eq!(payload.len(), 100);
        writer.await.unwrap();
    }
}
