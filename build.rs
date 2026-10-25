use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");

    let exe = Path::new("target/release/ghitaprint.exe");
    if exe.exists() {
        println!("cargo:rerun-if-changed=target/release/ghitaprint.exe");
    } else {
        println!(
            "cargo:warning=target/release/ghitaprint.exe chưa tồn tại — chạy 'cargo build --release --bin ghitaprint' trước khi build setup.exe"
        );
    }
}
