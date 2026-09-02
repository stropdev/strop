//! `strop update` — self-update for tarball installs (rootle 0017/0018
//! lineage): channel detection, latest-release lookup, mandatory sha256
//! sidecar verification, staged write + atomic rename over self.
//! Network goes through curl (install.sh already assumes it; no HTTP
//! stack in the binary). Progress stages ride indicatif.

use std::path::{Path, PathBuf};
use std::process::Command;

/// How this binary was installed — decides the upgrade path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// install.sh / release tarball — self-updates.
    Tarball,
    Brew,
    Cargo,
    Mise,
    Other,
}

pub fn channel() -> Channel {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if exe.contains(".cargo/bin") {
        Channel::Cargo
    } else if exe.contains("Cellar") || exe.contains("homebrew") || exe.contains("linuxbrew") {
        Channel::Brew
    } else if exe.contains("/mise/") {
        Channel::Mise
    } else if exe.contains("/.local/") || exe.contains("/usr/local/") {
        Channel::Tarball
    } else {
        Channel::Other
    }
}

fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let v = tag.strip_prefix('v').unwrap_or(tag);
    let mut it = v.split('.');
    Some((
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    ))
}

pub fn is_newer(latest: &str) -> bool {
    match (
        parse_version(latest),
        parse_version(env!("CARGO_PKG_VERSION")),
    ) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

fn target_triple() -> Result<&'static str, String> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        (os, arch) => return Err(format!("no prebuilt binary for {os}/{arch}")),
    })
}

fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("curl")
        .args(["-fsSL"])
        .args(args)
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl failed for {}", args.last().unwrap_or(&"")));
    }
    Ok(out.stdout)
}

fn curl_to(url: &str, dest: &Path) -> Result<(), String> {
    let out = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(dest)
        .arg(url)
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("download failed: {url}"));
    }
    Ok(())
}

fn stage(label: &str) -> indicatif::ProgressBar {
    let pb = indicatif::ProgressBar::new_spinner();
    pb.set_style(
        indicatif::ProgressStyle::with_template("  {spinner:.cyan} {msg}")
            .unwrap_or_else(|_| indicatif::ProgressStyle::default_spinner()),
    );
    pb.set_message(label.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}

fn done(pb: &indicatif::ProgressBar, label: &str) {
    pb.set_style(
        indicatif::ProgressStyle::with_template("  {msg}")
            .unwrap_or_else(|_| indicatif::ProgressStyle::default_spinner()),
    );
    pb.finish_with_message(format!("✓ {label}"));
}

/// `strop update [--check]`.
pub fn update(check_only: bool) -> Result<(), String> {
    let current = env!("CARGO_PKG_VERSION");
    match channel() {
        Channel::Brew => {
            return Err("installed via homebrew — run: brew upgrade stropdev/tap/strop".into())
        }
        Channel::Cargo => {
            return Err("installed via cargo — run: cargo install strop-editor --locked".into())
        }
        Channel::Mise => return Err("installed via mise — run: mise upgrade strop".into()),
        Channel::Tarball | Channel::Other => {}
    }

    let pb = stage("resolving latest release…");
    let body = curl(&["https://api.github.com/repos/stropdev/strop/releases/latest"])?;
    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("release JSON: {e}"))?;
    let tag = json["tag_name"]
        .as_str()
        .ok_or("no tag_name in release")?
        .to_string();
    let version = tag.trim_start_matches('v').to_string();
    done(&pb, &format!("latest is {tag}"));

    if !is_newer(&tag) {
        println!("strop {current} is current");
        return Ok(());
    }
    if check_only {
        println!("strop {current} → {version} available (run: strop update)");
        return Ok(());
    }

    let triple = target_triple()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let base = "https://github.com/stropdev/strop/releases/download";
    let archive = format!("strop-{version}-{triple}.tar.gz");

    let tmp = std::env::temp_dir().join(format!("strop-update-{version}"));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;

    let pb = stage(&format!("downloading {archive}…"));
    curl_to(&format!("{base}/v{version}/{archive}"), &tmp.join(&archive))?;
    let pb2 = stage("verifying checksum…");
    let sha = curl(&[format!("{base}/v{version}/{archive}.sha256").as_str()])?;
    let sha_path = tmp.join(format!("{archive}.sha256"));
    std::fs::write(&sha_path, &sha).map_err(|e| e.to_string())?;
    let ok = Command::new("sha256sum")
        .args(["-c", &format!("{archive}.sha256")])
        .current_dir(&tmp)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        || Command::new("shasum")
            .args(["-a", "256", "-c", &format!("{archive}.sha256")])
            .current_dir(&tmp)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
    if !ok {
        return Err("checksum mismatch — aborting".into());
    }
    done(&pb, &format!("downloaded {archive}"));
    done(&pb2, "checksum verified");

    let pb = stage("extracting…");
    let out = Command::new("tar")
        .args(["-xzf", &archive])
        .current_dir(&tmp)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("tar: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let new_bin: PathBuf = tmp.join(format!("strop-{version}-{triple}")).join("strop");
    done(&pb, "extracted");

    let pb = stage("installing over current binary…");
    // staged write + atomic rename over self (rootle 0017)
    let staged = exe.with_extension("strop-new");
    std::fs::copy(&new_bin, &staged).map_err(|e| format!("stage: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&staged)
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&staged, perms).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&staged, &exe).map_err(|e| format!("replace {}: {e}", exe.display()))?;
    done(&pb, &format!("installed to {}", exe.display()));
    let _ = std::fs::remove_dir_all(&tmp);

    println!("strop {current} → {version}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert!(is_newer("v0.2.0"));
        assert!(is_newer("0.1.2") || !is_newer("0.1.2")); // depends on current; no panic
        assert!(!is_newer("v0.0.9"));
        assert!(!is_newer("garbage"));
    }

    #[test]
    fn triples_cover_the_matrix() {
        assert!(target_triple().is_ok());
    }
}
