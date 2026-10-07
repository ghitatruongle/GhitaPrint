use std::process::Command;

pub const PRINTER_NAME: &str = "GhitaPrint";

pub fn install_virtual_printer(port: u16) -> Result<String, String> {
    #[cfg(windows)]
    {
        let port_url = format!("http://127.0.0.1:{}/printers/ghitaprint", port);
        let ps_cmd = format!(
            "try {{ Add-Printer -Name '{}' -PortName '{}' -DriverName 'Microsoft IPP Class Driver' -ErrorAction Stop; Write-Output 'OK' }} catch {{ Write-Output $_.Exception.Message }}",
            PRINTER_NAME, port_url
        );

        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps_cmd])
            .output()
            .map_err(|e| format!("Không thể thực thi PowerShell: {}", e))?;

        let res = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if res.contains("OK") {
            Ok(format!(
                "Đã tạo máy in ảo '{}' tại cổng '{}'",
                PRINTER_NAME, port_url
            ))
        } else {
            Ok(format!("Lệnh đã gửi tới Windows Spooler: {}", res))
        }
    }

    #[cfg(not(windows))]
    {
        Err("Tính năng máy in ảo chỉ hỗ trợ trên Windows.".to_string())
    }
}

pub fn uninstall_virtual_printer() -> Result<String, String> {
    #[cfg(windows)]
    {
        let ps_cmd = format!(
            "try {{ Remove-Printer -Name '{}' -ErrorAction Stop; Write-Output 'OK' }} catch {{ Write-Output $_.Exception.Message }}",
            PRINTER_NAME
        );

        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps_cmd])
            .output()
            .map_err(|e| format!("Không thể thực thi PowerShell: {}", e))?;

        let res = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if res.contains("OK") {
            Ok(format!("Đã gỡ bỏ máy in ảo '{}'", PRINTER_NAME))
        } else {
            Ok(format!("Kết quả: {}", res))
        }
    }

    #[cfg(not(windows))]
    {
        Err("Tính năng máy in ảo chỉ hỗ trợ trên Windows.".to_string())
    }
}

pub fn is_virtual_printer_installed() -> bool {
    #[cfg(windows)]
    {
        let ps_cmd = format!(
            "if (Get-Printer -Name '{}' -ErrorAction SilentlyContinue) {{ Write-Output '1' }} else {{ Write-Output '0' }}",
            PRINTER_NAME
        );
        if let Ok(output) = Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps_cmd])
            .output()
        {
            let res = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return res == "1";
        }
    }
    false
}
