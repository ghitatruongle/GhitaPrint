use std::process::Command;

pub const PRINTER_NAME: &str = "GhitaPrint";
pub const RAW_PORT_NAME: &str = "GhitaPrintRaw";

pub fn raw_port_number(port: u16) -> u16 {
    port.wrapping_add(1)
}

pub fn add_printer_command(port: u16) -> String {
    let raw_port = raw_port_number(port);
    format!(
        "try {{ if (-not (Get-PrinterPort -Name '{}' -ErrorAction SilentlyContinue)) {{ Add-PrinterPort -Name '{}' -PrinterHostAddress '127.0.0.1' -PortNumber {} -ErrorAction Stop }}; Add-Printer -Name '{}' -PortName '{}' -DriverName 'Microsoft IPP Class Driver' -ErrorAction Stop; Write-Output 'OK' }} catch {{ Write-Output $_.Exception.Message }}",
        RAW_PORT_NAME, RAW_PORT_NAME, raw_port, PRINTER_NAME, RAW_PORT_NAME
    )
}

pub fn remove_printer_command(silent: bool) -> String {
    let error_action = if silent { "SilentlyContinue" } else { "Stop" };
    format!(
        "try {{ Remove-Printer -Name '{}' -ErrorAction {}; Write-Output 'OK' }} catch {{ Write-Output $_.Exception.Message }}",
        PRINTER_NAME, error_action
    )
}

pub fn printer_exists_command() -> String {
    format!(
        "if (Get-Printer -Name '{}' -ErrorAction SilentlyContinue) {{ Write-Output '1' }} else {{ Write-Output '0' }}",
        PRINTER_NAME
    )
}

pub fn printer_port_command() -> String {
    format!(
        "try {{ $p = Get-Printer -Name '{}' -ErrorAction Stop; Write-Output $p.PortName }} catch {{ Write-Output 'ERR' }}",
        PRINTER_NAME
    )
}

#[cfg(windows)]
pub fn run_powershell_utf8(script: &str) -> Result<String, String> {
    let wrapped = format!(
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8; {}",
        script
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command", &wrapped])
        .output()
        .map_err(|e| format!("Không thể thực thi PowerShell: {}", e))?;

    let mut combined = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr_text = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr_text.is_empty() {
        combined.push(' ');
        combined.push_str(&stderr_text);
    }
    Ok(combined.trim().to_string())
}

#[cfg(not(windows))]
pub fn run_powershell_utf8(_script: &str) -> Result<String, String> {
    Err("Tính năng máy in ảo chỉ hỗ trợ trên Windows.".to_string())
}

fn ps_ok(result: &str) -> bool {
    result == "OK"
}

pub fn install_virtual_printer(port: u16) -> Result<String, String> {
    let raw_port = raw_port_number(port);
    let output = run_powershell_utf8(&add_printer_command(port))?;

    if ps_ok(&output) {
        Ok(format!(
            "Đã tạo máy in ảo '{}' tại cổng TCPMON '{}' → 127.0.0.1:{} (raw)",
            PRINTER_NAME, RAW_PORT_NAME, raw_port
        ))
    } else {
        Err(format!(
            "Windows Spooler từ chối lệnh cài đặt: {} (gợi ý: máy in trùng tên đã tồn tại hoặc policy chặn)",
            output
        ))
    }
}

pub fn reinstall_virtual_printer(port: u16) -> Result<String, String> {
    if is_virtual_printer_installed() {
        let _ = run_powershell_utf8(&remove_printer_command(true));
    }
    install_virtual_printer(port)
}

pub fn uninstall_virtual_printer() -> Result<String, String> {
    let output = run_powershell_utf8(&remove_printer_command(false))?;

    if ps_ok(&output) {
        let _ = run_powershell_utf8(&remove_printer_port_command());
        Ok(format!("Đã gỡ bỏ máy in ảo '{}'", PRINTER_NAME))
    } else if output.contains("không tồn tại") || output.to_lowercase().contains("not found") {
        Ok(format!(
            "Máy in ảo '{}' không có trên hệ thống — không cần gỡ",
            PRINTER_NAME
        ))
    } else {
        Err(format!("Windows Spooler từ chối lệnh gỡ bỏ: {}", output))
    }
}

pub fn remove_printer_port_command() -> String {
    format!(
        "try {{ Get-PrinterPort | Where-Object {{ $_.Name -eq '{}' }} | Remove-PrinterPort -ErrorAction SilentlyContinue; Write-Output 'OK' }} catch {{ Write-Output $_.Exception.Message }}",
        RAW_PORT_NAME
    )
}

pub fn is_virtual_printer_installed() -> bool {
    match run_powershell_utf8(&printer_exists_command()) {
        Ok(result) => result == "1",
        Err(_) => false,
    }
}

pub fn get_installed_printer_port() -> Option<String> {
    let result = run_powershell_utf8(&printer_port_command()).ok()?;
    if result == "ERR" || result.is_empty() {
        None
    } else {
        Some(result)
    }
}

pub fn ensure_virtual_printer_matches(port: u16) -> Result<Option<String>, String> {
    if !is_virtual_printer_installed() {
        return Ok(None);
    }
    let installed_port = get_installed_printer_port();
    let expected = RAW_PORT_NAME;
    match installed_port {
        Some(actual) if actual == expected => Ok(None),
        Some(actual) => match reinstall_virtual_printer(port) {
            Ok(_) => Ok(Some(format!(
                "Máy in ảo 'GhitaPrint' đã trỏ sai cổng ({}) — đã tự cập nhật về {} (127.0.0.1:{})",
                actual,
                expected,
                raw_port_number(port)
            ))),
            Err(e) => Err(format!(
                "Máy in ảo 'GhitaPrint' đang trỏ sai cổng ({}) nhưng không thể tự cập nhật: {} — hãy chạy lại 'ghitaprint virtual-printer install'",
                actual, e
            )),
        },
        None => Ok(Some(
            "Máy in ảo 'GhitaPrint' tồn tại nhưng không đọc được cổng — kiểm tra bằng 'ghitaprint virtual-printer status'".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_printer_command_creates_tcpmon_port_then_printer() {
        let cmd = add_printer_command(9123);
        assert!(cmd.contains("Add-PrinterPort"));
        assert!(cmd.contains("-PrinterHostAddress '127.0.0.1'"));
        assert!(cmd.contains("-PortNumber 9124"));
        assert!(cmd.contains("Add-Printer"));
        assert!(cmd.contains("'GhitaPrintRaw'"));
        assert!(cmd.contains("'GhitaPrint'"));
        assert!(cmd.contains("Microsoft IPP Class Driver"));
    }

    #[test]
    fn test_raw_port_number_wraps_safely() {
        assert_eq!(raw_port_number(9123), 9124);
        assert_eq!(raw_port_number(u16::MAX), 0);
    }

    #[test]
    fn test_remove_printer_command_modes() {
        assert!(remove_printer_command(true).contains("SilentlyContinue"));
        assert!(remove_printer_command(false).contains("ErrorAction Stop"));
    }

    #[test]
    fn test_raw_port_display_format() {
        assert_eq!(raw_port_number(9500), 9501);
    }

    #[test]
    fn test_ps_ok_requires_exact_match() {
        assert!(ps_ok("OK"));
        assert!(!ps_ok("OK và lỗi lẫn vào"));
        assert!(!ps_ok("Lỗi truy cập bị từ chối"));
    }

    #[test]
    fn test_port_command_reads_portname() {
        let cmd = printer_port_command();
        assert!(cmd.contains("Get-Printer"));
        assert!(cmd.contains("PortName"));
    }
}
