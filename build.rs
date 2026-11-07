use sha2::{Digest, Sha256};
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=target/release/ghitaprint.exe");

    let exe = Path::new("target/release/ghitaprint.exe");
    if !exe.exists() {
        println!(
            "cargo:warning=target/release/ghitaprint.exe chưa tồn tại — tạo stub tạm. Bộ cài build từ stub sẽ TỪ CHỐI cài (hãy chạy 'cargo build --release --bin ghitaprint' trước khi build setup)"
        );
        if let Some(parent) = exe.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(exe, b"GHITAPRINT-STUB-NOT-A-REAL-EXECUTABLE");
    }

    let bytes = std::fs::read(exe).expect("không đọc được file exe nhúng cho setup");
    let digest = Sha256::digest(&bytes);
    let hex: String = digest.iter().map(|byte| format!("{:02x}", byte)).collect();
    println!("cargo:rustc-env=GHITAPRINT_EMBEDDED_SHA256={}", hex);
    println!("cargo:rustc-env=GHITAPRINT_EMBEDDED_SIZE={}", bytes.len());
}
