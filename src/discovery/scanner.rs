use crate::cli::output::PrinterInfo;
use crate::transport::tcp::TcpTransport;
use crate::transport::usb::scan_usb_printers;

pub struct DeviceScanner;

#[allow(dead_code)]
impl DeviceScanner {
    pub fn scan_usb() -> Vec<PrinterInfo> {
        let mut results = Vec::new();
        if let Ok(devices) = scan_usb_printers() {
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
                        id: format!("net-{}", addr_str.replace(['.', ':'], "-")),
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
        results
    }

    pub async fn scan_all(network_candidates: &[&str]) -> Vec<PrinterInfo> {
        let mut list = Self::scan_usb();
        let net_list = Self::scan_network(network_candidates).await;
        list.extend(net_list);
        list
    }
}
