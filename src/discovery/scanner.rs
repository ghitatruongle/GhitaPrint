use crate::cli::output::PrinterInfo;
use crate::transport::tcp::TcpTransport;
use crate::transport::usb::scan_usb_printers;
use std::net::Ipv4Addr;
use tracing::warn;

pub struct DeviceScanner;

#[allow(dead_code)]
impl DeviceScanner {
    pub fn scan_usb() -> Vec<PrinterInfo> {
        let mut results = Vec::new();
        match scan_usb_printers() {
            Ok(devices) => {
                for dev in devices {
                    results.push(PrinterInfo {
                        id: format!("usb-{:04x}-{:04x}", dev.vendor_id, dev.product_id),
                        name: format!("{} {}", dev.manufacturer, dev.product),
                        transport: "USB Direct".to_string(),
                        address: dev.uri(),
                        status: "Sẵn sàng".to_string(),
                    });
                }
            }
            Err(e) => {
                warn!("Quét USB thất bại: {}", e);
                eprintln!("⚠ Quét USB thất bại: {}", e);
            }
        }
        results.sort_by(|a, b| a.address.cmp(&b.address));
        results
    }

    pub async fn scan_network(candidates: &[&str]) -> Vec<PrinterInfo> {
        let mut set = tokio::task::JoinSet::new();

        for &addr in candidates {
            let addr_str = addr.to_string();
            set.spawn(async move {
                if TcpTransport::check_alive(&addr_str, 1).await {
                    let full_addr = TcpTransport::normalize_address(&addr_str);
                    Some(PrinterInfo {
                        id: format!("net-{}", full_addr.replace(['.', ':'], "-")),
                        name: format!("Máy in mạng ({})", full_addr),
                        transport: "TCP Network".to_string(),
                        address: full_addr,
                        status: "Sẵn sàng".to_string(),
                    })
                } else {
                    None
                }
            });
        }

        let mut results = Vec::new();
        while let Some(res) = set.join_next().await {
            if let Ok(Some(info)) = res {
                results.push(info);
            }
        }
        results.sort_by(|a, b| a.address.cmp(&b.address));
        results
    }

    pub fn parse_cidr(cidr: &str) -> Option<(Ipv4Addr, u32)> {
        let (ip_part, prefix_part) = cidr.split_once('/')?;
        let ip: Ipv4Addr = ip_part.trim().parse().ok()?;
        let prefix: u32 = prefix_part.trim().parse().ok()?;
        if prefix > 32 {
            return None;
        }
        Some((ip, prefix))
    }

    pub async fn scan_subnet(
        cidr: &str,
        port: u16,
        timeout_ms: u64,
    ) -> Result<Vec<PrinterInfo>, String> {
        let (base_ip, prefix) = Self::parse_cidr(cidr)
            .ok_or_else(|| format!("CIDR không hợp lệ: '{}' (mẫu: 192.168.1.0/24)", cidr))?;

        let total_hosts = 2u64.pow(32 - prefix);
        if total_hosts > 65_536 {
            return Err(format!(
                "Dải /{} quá lớn ({} máy) — hỗ trợ tối đa /16",
                prefix, total_hosts
            ));
        }

        let base: u32 = base_ip.into();
        let timeout_secs = (timeout_ms / 1000).max(1);
        let mut set = tokio::task::JoinSet::new();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(128));

        for offset in 0..total_hosts {
            let ip_int = base.wrapping_add(offset as u32);
            let ip = Ipv4Addr::from(ip_int);
            let octets = ip.octets();
            let is_network = offset == 0;
            let is_broadcast = offset + 1 == total_hosts;
            let is_special = octets[3] == 0 || octets[3] == 255;
            if (total_hosts > 2 && (is_network || is_broadcast)) || is_special {
                continue;
            }

            let permit = semaphore.clone();
            let target = format!("{}:{}", ip, port);
            set.spawn(async move {
                let _permit = permit.acquire().await;
                if TcpTransport::check_alive(&target, timeout_secs).await {
                    Some(PrinterInfo {
                        id: format!("net-{}", target.replace(['.', ':'], "-")),
                        name: format!("Máy in mạng ({})", target),
                        transport: "TCP Network".to_string(),
                        address: target,
                        status: "Sẵn sàng".to_string(),
                    })
                } else {
                    None
                }
            });
        }

        let mut results = Vec::new();
        while let Some(res) = set.join_next().await {
            if let Ok(Some(info)) = res {
                results.push(info);
            }
        }
        results.sort_by(|a, b| a.address.cmp(&b.address));
        Ok(results)
    }

    pub fn local_subnet_cidr() -> Option<String> {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect("8.8.8.8:80").ok()?;
        let local_ip = socket.local_addr().ok()?.ip();
        let std::net::IpAddr::V4(v4) = local_ip else {
            return None;
        };
        let octets = v4.octets();
        Some(format!("{}.{}.{}.0/24", octets[0], octets[1], octets[2]))
    }

    pub async fn scan_all(network_candidates: &[&str]) -> Vec<PrinterInfo> {
        let mut list = Self::scan_usb();
        let net_list = Self::scan_network(network_candidates).await;
        list.extend(net_list);
        list
    }
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cidr_valid_and_invalid() {
        assert_eq!(
            DeviceScanner::parse_cidr("192.168.1.0/24"),
            Some((Ipv4Addr::new(192, 168, 1, 0), 24))
        );
        assert_eq!(
            DeviceScanner::parse_cidr(" 10.0.0.5 / 30 "),
            Some((Ipv4Addr::new(10, 0, 0, 5), 30))
        );
        assert_eq!(DeviceScanner::parse_cidr("192.168.1.0"), None);
        assert_eq!(DeviceScanner::parse_cidr("192.168.1.0/33"), None);
        assert_eq!(DeviceScanner::parse_cidr("abc/24"), None);
    }

    #[test]
    fn test_local_subnet_format() {
        if let Some(cidr) = DeviceScanner::local_subnet_cidr() {
            assert!(
                cidr.ends_with(".0/24"),
                "subnet '{}' phải dạng x.y.z.0/24",
                cidr
            );
        }
    }

    #[tokio::test]
    async fn test_scan_subnet_finds_local_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let results = DeviceScanner::scan_subnet("127.0.0.0/32", port, 500)
            .await
            .expect("scan_subnet phải chạy được");
        assert!(
            results.is_empty()
                || results
                    .iter()
                    .all(|r| r.address.contains(&port.to_string()))
        );
    }

    #[tokio::test]
    async fn test_scan_subnet_rejects_oversized_range() {
        let err = DeviceScanner::scan_subnet("10.0.0.0/8", 9100, 500)
            .await
            .unwrap_err();
        assert!(err.contains("quá lớn"));
    }
}
