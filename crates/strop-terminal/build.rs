use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").ok_or("missing package root")?);
    let out = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing build output root")?);
    let zig = std::env::var_os("ZIG").unwrap_or_else(|| "zig".into());
    let version = Command::new(&zig).arg("version").output().map_err(|error| {
        format!("strop source builds require Zig 0.16.0; set ZIG to its path or use a prebuilt strop release. No compiler is downloaded: {error}")
    })?;
    if !version.status.success() || String::from_utf8(version.stdout)?.trim() != "0.16.0" {
        return Err("strop source builds require exactly Zig 0.16.0".into());
    }
    let source_root = out.join("upstream");
    std::fs::create_dir_all(&source_root)?;
    let archive = std::fs::File::open(root.join("vendor/ghostty-vt-5252b193.tar.gz"))?;
    tar::Archive::new(flate2::read::GzDecoder::new(archive)).unpack(&source_root)?;
    let source = source_root.join("libghostty-vt-1.3.2-HEAD-+5252b19");
    let prefix = out.join("native");
    let target = match std::env::var("TARGET")?.as_str() {
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gnu",
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu",
        "x86_64-unknown-linux-musl" => "x86_64-linux-musl",
        "aarch64-unknown-linux-musl" => "aarch64-linux-musl",
        "x86_64-apple-darwin" => "x86_64-macos",
        "aarch64-apple-darwin" => "aarch64-macos",
        _ => {
            return Err("embedded terminals require a supported Linux or macOS build target".into())
        }
    };
    let status = Command::new(&zig)
        .current_dir(&source)
        .args([
            "build",
            "-Demit-lib-vt=true",
            "-Doptimize=ReleaseFast",
            "-Dvt-features=-kitty-graphics",
        ])
        .arg(format!("-Dtarget={target}"))
        .arg("--prefix")
        .arg(&prefix)
        .status()?;
    if !status.success() {
        return Err(format!("pinned terminal library build failed: {status}").into());
    }
    cc::Build::new()
        .file(root.join("native/bridge.c"))
        .include(source.join("include"))
        .define(
            "STROP_TERMINAL_VERSION",
            format!("\"strop {}\"", std::env::var("CARGO_PKG_VERSION")?).as_str(),
        )
        .flag_if_supported("-std=c11")
        .compile("strop_terminal_bridge");
    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=ghostty-vt");
    println!("cargo:rerun-if-env-changed=ZIG");
    println!("cargo:rerun-if-env-changed=ZIG_GLOBAL_CACHE_DIR");
    for file in [
        "vendor/ghostty-vt-5252b193.tar.gz",
        "native/bridge.c",
        "native/bridge.h",
        "native/memory.h",
    ] {
        println!("cargo:rerun-if-changed={file}");
    }
    Ok(())
}
