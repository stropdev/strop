//! `strop update` — self-update for tarball installs (0056 AR11/AR12):
//! channel identity from a private installation receipt (never path
//! substrings), latest-release resolution through the generated release
//! catalog, mandatory sha256 verification, staged write + atomic rename
//! over self. Network goes through curl (install.sh already assumes it;
//! no HTTP stack in the binary). Progress stages ride indicatif.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Private installation receipt written next to the binary by install.sh
/// and refreshed by every successful self-update. Channel identity comes
/// from this receipt alone; a missing or malformed receipt is an honest
/// unknown, never a guess (pre-receipt installs land there).
pub const RECEIPT_NAME: &str = ".strop-install.json";

/// The catalog is generated from build outputs by the release workflow
/// and attached to every GitHub release, so this URL always serves the
/// catalog of the latest release.
const CATALOG_URL: &str = "https://github.com/stropdev/strop/releases/latest/download/catalog.json";

/// How this binary was installed — decides the upgrade path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// install.sh / release tarball — self-updates.
    Tarball,
    Brew,
    Cargo,
    Mise,
    /// A receipt channel we do not recognize: honest unknown.
    Unknown,
}

impl Channel {
    fn named(name: &str) -> Channel {
        match name {
            "tarball" => Channel::Tarball,
            "brew" => Channel::Brew,
            "cargo" => Channel::Cargo,
            "mise" => Channel::Mise,
            _ => Channel::Unknown,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Channel::Tarball => "tarball",
            Channel::Brew => "brew",
            Channel::Cargo => "cargo",
            Channel::Mise => "mise",
            Channel::Unknown => "unknown",
        }
    }
}

/// Installation identity recorded at install/update time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub channel: Channel,
    pub method: String,
    pub version: String,
    pub install_root: PathBuf,
    /// Version this install replaced, when known — recovery/rollback
    /// facts stay truthful instead of inventing provenance.
    pub previous_version: Option<String>,
}

impl Receipt {
    /// Read the receipt beside `exe`; missing or malformed receipts are
    /// honest unknowns, not guessed provenance.
    pub fn read(exe: &Path) -> Option<Receipt> {
        Self::read_from(&exe.parent()?.join(RECEIPT_NAME))
    }

    fn read_from(path: &Path) -> Option<Receipt> {
        let body = std::fs::read(path).ok()?;
        let json: serde_json::Value = serde_json::from_slice(&body).ok()?;
        Some(Receipt {
            channel: Channel::named(json.get("channel")?.as_str()?),
            method: json.get("method")?.as_str()?.to_string(),
            version: json.get("version")?.as_str()?.to_string(),
            install_root: PathBuf::from(json.get("install_root")?.as_str()?),
            previous_version: json
                .get("previous_version")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        })
    }

    /// Publish the receipt atomically beside the binary: staged temp
    /// file + rename in the same directory, like the binary itself.
    fn write(&self, dir: &Path) -> Result<(), String> {
        use std::io::Write;
        let installed_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let mut body = serde_json::json!({
            "schema": 1,
            "channel": self.channel.name(),
            "method": self.method,
            "version": self.version,
            "install_root": self.install_root,
            "installed_at": installed_at,
        });
        if let Some(previous) = &self.previous_version {
            body["previous_version"] = serde_json::Value::String(previous.clone());
        }
        let mut staged = tempfile::NamedTempFile::new_in(dir)
            .map_err(|error| format!("stage receipt: {error}"))?;
        staged
            .write_all(body.to_string().as_bytes())
            .map_err(|error| format!("stage receipt: {error}"))?;
        staged
            .persist(dir.join(RECEIPT_NAME))
            .map_err(|error| format!("publish receipt: {error}"))?;
        Ok(())
    }
}

/// The generated release catalog — the single source of truth for
/// current version/artifact/digest facts. Installer, updater and site
/// all resolve through it instead of scraping APIs on their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    pub version: String,
    pub tag: String,
    pub artifacts: Vec<Artifact>,
}

/// One platform artifact as recorded by the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub target: String,
    pub name: String,
    pub sha256: String,
    pub url: String,
}

impl Catalog {
    fn parse(body: &[u8]) -> Result<Catalog, String> {
        let json: serde_json::Value =
            serde_json::from_slice(body).map_err(|e| format!("release catalog JSON: {e}"))?;
        let field = |entry: &serde_json::Value, key: &str| -> Result<String, String> {
            entry
                .get(key)
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("release catalog missing {key}"))
        };
        let mut artifacts = Vec::new();
        for entry in json["artifacts"]
            .as_array()
            .ok_or("release catalog missing artifacts")?
        {
            artifacts.push(Artifact {
                target: field(entry, "target")?,
                name: field(entry, "name")?,
                sha256: field(entry, "sha256")?,
                url: field(entry, "url")?,
            });
        }
        Ok(Catalog {
            version: field(&json, "version")?,
            tag: field(&json, "tag")?,
            artifacts,
        })
    }

    pub fn artifact(&self, target: &str) -> Option<&Artifact> {
        self.artifacts.iter().find(|entry| entry.target == target)
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
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let receipt_path = exe
        .parent()
        .map(|dir| dir.join(RECEIPT_NAME))
        .unwrap_or_else(|| PathBuf::from(RECEIPT_NAME));
    let Some(receipt) = Receipt::read(&exe) else {
        return Err(format!(
            "no installation receipt at {} — this install predates receipts or is managed \
             by a package manager; upgrade through that manager, or reinstall with \
             install.sh to establish a receipt",
            receipt_path.display()
        ));
    };
    match receipt.channel {
        Channel::Brew => {
            return Err(
                "installation receipt says homebrew — run: brew upgrade stropdev/tap/strop".into(),
            )
        }
        Channel::Cargo => {
            return Err(
                "installation receipt says cargo — run: cargo install strop-editor --locked".into(),
            )
        }
        Channel::Mise => {
            return Err("installation receipt says mise — run: mise upgrade strop".into())
        }
        Channel::Unknown => {
            return Err(format!(
                "installation receipt at {} names an unrecognized channel — not guessing \
                 provenance; reinstall with install.sh or upgrade through your package manager",
                receipt_path.display()
            ))
        }
        Channel::Tarball => {}
    }

    let pb = stage("resolving release catalog…");
    let body = curl(&[CATALOG_URL])?;
    let catalog = Catalog::parse(&body)?;
    done(&pb, &format!("latest is {}", catalog.tag));

    if !is_newer(&catalog.version) {
        println!("strop {current} is current");
        return Ok(());
    }
    if check_only {
        println!(
            "strop {current} → {} available (run: strop update)",
            catalog.version
        );
        return Ok(());
    }

    let triple = target_triple()?;
    let artifact = catalog
        .artifact(triple)
        .ok_or_else(|| format!("release catalog has no artifact for {triple}"))?;
    let archive = artifact.name.clone();

    let staging = tempfile::Builder::new()
        .prefix("strop-update-")
        .tempdir()
        .map_err(|error| format!("private update staging: {error}"))?;
    let tmp = staging.path();

    let pb = stage(&format!("downloading {archive}…"));
    curl_to(&artifact.url, &tmp.join(&archive))?;
    let pb2 = stage("verifying checksum…");
    // The catalog digest is the verified truth — check the downloaded
    // bytes against it in the classic sidecar form.
    let sha_path = tmp.join(format!("{archive}.sha256"));
    std::fs::write(&sha_path, format!("{}  {}\n", artifact.sha256, archive))
        .map_err(|e| e.to_string())?;
    let ok = Command::new("sha256sum")
        .args(["-c", &format!("{archive}.sha256")])
        .current_dir(tmp)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        || Command::new("shasum")
            .args(["-a", "256", "-c", &format!("{archive}.sha256")])
            .current_dir(tmp)
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
        .current_dir(tmp)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("tar: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let new_bin: PathBuf = tmp
        .join(format!("strop-{}-{triple}", catalog.version))
        .join("strop");
    done(&pb, "extracted");

    let pb = stage("installing over current binary…");
    // staged write + atomic rename over self (rootle 0017)
    let parent = exe.parent().ok_or("executable has no parent directory")?;
    let mut staged =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| format!("stage: {error}"))?;
    let mut source =
        std::fs::File::open(&new_bin).map_err(|error| format!("extracted binary: {error}"))?;
    std::io::copy(&mut source, staged.as_file_mut()).map_err(|error| format!("stage: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staged
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("permissions: {error}"))?;
    }
    staged
        .as_file()
        .sync_all()
        .map_err(|error| format!("sync staged binary: {error}"))?;
    staged
        .persist(&exe)
        .map_err(|error| format!("replace {}: {}", exe.display(), error.error))?;

    // Refresh the receipt: same channel/method, new version, remembered
    // previous — identity facts stay truthful for recovery/rollback.
    let refreshed = Receipt {
        channel: Channel::Tarball,
        method: receipt.method.clone(),
        version: catalog.version.clone(),
        install_root: parent.to_path_buf(),
        previous_version: Some(receipt.version.clone()),
    };
    if let Err(error) = refreshed.write(parent) {
        eprintln!("warning: installed, but could not refresh the installation receipt: {error}");
    }
    done(&pb, &format!("installed to {}", exe.display()));

    println!("strop {current} → {}", catalog.version);
    Ok(())
}

#[cfg(test)]
#[test]
fn version_ordering() {
    let (major, minor, patch) =
        parse_version(env!("CARGO_PKG_VERSION")).expect("crate version parses");
    // newer/older/same relative to whatever we currently are —
    // the test must not rot on every release
    assert!(is_newer(&format!("v{}.{}.{}", major, minor, patch + 1)));
    assert!(!is_newer(&format!("v{major}.{minor}.{patch}")));
    assert!(!is_newer(&format!(
        "v{}.{}.{}",
        major,
        minor.saturating_sub(1),
        patch
    )));
    assert!(!is_newer("garbage"));
}

#[cfg(test)]
#[test]
fn triples_cover_the_matrix() {
    assert!(target_triple().is_ok());
}

#[cfg(test)]
#[test]
fn receipt_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let receipt = Receipt {
        channel: Channel::Tarball,
        method: "install.sh".into(),
        version: "0.33.0".into(),
        install_root: dir.path().to_path_buf(),
        previous_version: Some("0.32.0".into()),
    };
    receipt.write(dir.path()).expect("receipt writes");
    let read = Receipt::read(&dir.path().join("strop")).expect("receipt reads back");
    assert_eq!(read, receipt);
}

#[cfg(test)]
#[test]
fn missing_receipt_is_unknown() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(Receipt::read(&dir.path().join("strop")).is_none());
}

#[cfg(test)]
#[test]
fn channel_identity_comes_from_the_receipt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let exe = dir.path().join("strop");
    for (name, want) in [
        ("tarball", Channel::Tarball),
        ("brew", Channel::Brew),
        ("cargo", Channel::Cargo),
        ("mise", Channel::Mise),
        ("flatpak", Channel::Unknown),
    ] {
        std::fs::write(
            dir.path().join(RECEIPT_NAME),
            format!(
                "{{\"channel\":\"{name}\",\"method\":\"test\",\"version\":\"1.0.0\",\
                 \"install_root\":\"/tmp/x\"}}"
            ),
        )
        .expect("fixture receipt");
        let receipt = Receipt::read(&exe).expect("receipt parses");
        assert_eq!(receipt.channel, want, "channel {name}");
    }
    // A malformed receipt is an honest unknown, not guessed provenance.
    std::fs::write(dir.path().join(RECEIPT_NAME), b"not json").expect("fixture");
    assert!(Receipt::read(&exe).is_none());
}

#[cfg(test)]
const CATALOG_FIXTURE: &str = r#"{
  "schema": 1,
  "product": "strop",
  "version": "9.9.9",
  "tag": "v9.9.9",
  "published_at": "2026-01-01T00:00:00Z",
  "artifacts": [
    {
      "target": "aarch64-apple-darwin",
      "name": "strop-9.9.9-aarch64-apple-darwin.tar.gz",
      "sha256": "aaaa",
      "url": "https://example.test/v9.9.9/strop-9.9.9-aarch64-apple-darwin.tar.gz"
    },
    {
      "target": "x86_64-unknown-linux-musl",
      "name": "strop-9.9.9-x86_64-unknown-linux-musl.tar.gz",
      "sha256": "bbbb",
      "url": "https://example.test/v9.9.9/strop-9.9.9-x86_64-unknown-linux-musl.tar.gz"
    }
  ]
}"#;

#[cfg(test)]
#[test]
fn catalog_resolves_artifacts_by_target() {
    let catalog = Catalog::parse(CATALOG_FIXTURE.as_bytes()).expect("catalog parses");
    assert_eq!(catalog.version, "9.9.9");
    assert_eq!(catalog.tag, "v9.9.9");
    let artifact = catalog
        .artifact("x86_64-unknown-linux-musl")
        .expect("linux artifact recorded");
    assert_eq!(
        artifact.name,
        "strop-9.9.9-x86_64-unknown-linux-musl.tar.gz"
    );
    assert_eq!(artifact.sha256, "bbbb");
    assert!(artifact.url.ends_with(&artifact.name));
    assert!(catalog.artifact("wasm32-unknown-unknown").is_none());
}

#[cfg(test)]
#[test]
fn malformed_catalog_is_rejected() {
    assert!(Catalog::parse(b"{}").is_err());
    assert!(Catalog::parse(b"not json").is_err());
}
