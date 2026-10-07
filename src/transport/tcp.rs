use super::{PrinterTransport, TransportError};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

#[allow(dead_code)]
pub struct TcpTransport {
    stream: TcpStream,
    address: String,
}

#[allow(dead_code)]
impl TcpTransport {
    pub fn normalize_address(addr: &str) -> String {
        if addr.contains(':') {
            addr.to_string()
        } else {
            format!("{}:9100", addr)
        }
    }

    pub async fn connect(address: &str, timeout_secs: u64) -> Result<Self, TransportError> {
        let full_addr = Self::normalize_address(address);
        let duration = Duration::from_secs(timeout_secs);

        let stream = timeout(duration, TcpStream::connect(&full_addr))
            .await
            .map_err(|_| TransportError::Timeout(timeout_secs))?
            .map_err(TransportError::Io)?;

        let _ = stream.set_nodelay(true);

        Ok(Self {
            stream,
            address: full_addr,
        })
    }

    pub async fn check_alive(address: &str, timeout_secs: u64) -> bool {
        let full_addr = Self::normalize_address(address);
        let duration = Duration::from_secs(timeout_secs);
        matches!(
            timeout(duration, TcpStream::connect(&full_addr)).await,
            Ok(Ok(_))
        )
    }

    pub fn address(&self) -> &str {
        &self.address
    }
}

impl PrinterTransport for TcpTransport {
    async fn send_raw(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.stream
            .write_all(data)
            .await
            .map_err(TransportError::Io)?;
        self.stream.flush().await.map_err(TransportError::Io)?;
        Ok(())
    }

    async fn is_alive(&self) -> bool {
        Self::check_alive(&self.address, 2).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_address() {
        assert_eq!(
            TcpTransport::normalize_address("192.168.1.100"),
            "192.168.1.100:9100"
        );
        assert_eq!(
            TcpTransport::normalize_address("192.168.1.100:9100"),
            "192.168.1.100:9100"
        );
        assert_eq!(
            TcpTransport::normalize_address("localhost:9105"),
            "localhost:9105"
        );
    }

    #[tokio::test]
    async fn test_tcp_transport_real_socket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let addr = format!("127.0.0.1:{}", port);

        let server_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; 1024];
            let n = socket.read(&mut buf).await.unwrap();
            buf.truncate(n);
            buf
        });

        let mut client = TcpTransport::connect(&addr, 2).await.unwrap();
        client.send_raw(b"TEST_PRINT_DATA").await.unwrap();

        let received = server_task.await.unwrap();
        assert_eq!(&received, b"TEST_PRINT_DATA");
    }
}
