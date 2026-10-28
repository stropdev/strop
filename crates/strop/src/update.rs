//! `strop update` — self-update for tarball installs (0056 AR11/AR12):
//! channel identity from a private installation receipt (never path
//! substrings), latest-release resolution through the generated release
//! catalog, mandatory sha256 verification, staged write + atomic rename
//! over self. Downloads use rustls, checksums use sha2, and extraction is
//! constrained to the release archive's single-binary layout. Progress
//! stages ride indicatif.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_CATALOG_BYTES: u64 = 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;

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

fn target_triple(os: &str, arch: &str) -> Result<&'static str, String> {
    Ok(match (os, arch) {
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        (os, arch) => return Err(format!("no prebuilt binary for {os}/{arch}")),
    })
}

fn http_response(url: &str) -> Result<ureq::Response, String> {
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .get(url)
        .call()
        .map_err(|error| format!("download failed: {url}: {error}"))
}

fn http_get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    http_response(url)?
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|error| format!("read download: {url}: {error}"))?;
    if body.len() as u64 > limit {
        return Err(format!("download exceeds {limit} bytes: {url}"));
    }
    Ok(body)
}

fn download_to(url: &str, dest: &Path, limit: u64) -> Result<(), String> {
    let mut output = std::fs::File::create(dest)
        .map_err(|error| format!("create download {}: {error}", dest.display()))?;
    let copied = std::io::copy(
        &mut http_response(url)?.into_reader().take(limit + 1),
        &mut output,
    )
    .map_err(|error| format!("download failed: {url}: {error}"))?;
    if copied > limit {
        return Err(format!("download exceeds {limit} bytes: {url}"));
    }
    output
        .sync_all()
        .map_err(|error| format!("sync download {}: {error}", dest.display()))
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let mut input =
        std::fs::File::open(path).map_err(|error| format!("open downloaded archive: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|error| format!("read downloaded archive: {error}"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if format!("{digest:x}", digest = digest.finalize()) != expected {
        return Err("checksum mismatch — aborting".into());
    }
    Ok(())
}

fn extract_archive(
    archive_path: &Path,
    destination: &Path,
    version: &str,
    triple: &str,
) -> Result<PathBuf, String> {
    let package = format!("strop-{version}-{triple}");
    let package_path = Path::new(&package);
    let binary_path = package_path.join("strop");
    let archive_file = std::fs::File::open(archive_path)
        .map_err(|error| format!("open downloaded archive: {error}"))?;
    let decoder = flate2::read::GzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| format!("read release archive: {error}"))?;
    let mut found_binary = false;

    for entry in entries {
        let mut entry = entry.map_err(|error| format!("read release archive: {error}"))?;
        let path = entry
            .path()
            .map_err(|error| format!("release archive path: {error}"))?
            .into_owned();
        if path == package_path {
            if !entry.header().entry_type().is_dir() {
                return Err("release archive package entry is not a directory".into());
            }
        } else if path == binary_path {
            if !entry.header().entry_type().is_file() || found_binary {
                return Err("release archive has an invalid or duplicate binary entry".into());
            }
            found_binary = true;
        } else {
            return Err(format!(
                "release archive has unexpected path: {}",
                path.display()
            ));
        }
        if !entry
            .unpack_in(destination)
            .map_err(|error| format!("extract {}: {error}", path.display()))?
        {
            return Err(format!("refusing archive path: {}", path.display()));
        }
    }

    if !found_binary {
        return Err(format!(
            "release archive is missing {}",
            binary_path.display()
        ));
    }
    Ok(destination.join(binary_path))
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

    let triple = target_triple(std::env::consts::OS, std::env::consts::ARCH)?;
    let pb = stage("resolving release catalog…");
    let body = http_get(CATALOG_URL, MAX_CATALOG_BYTES)?;
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
    let archive_path = tmp.join(&archive);
    download_to(&artifact.url, &archive_path, MAX_ARCHIVE_BYTES)?;
    let pb2 = stage("verifying checksum…");
    verify_sha256(&archive_path, &artifact.sha256)?;
    done(&pb, &format!("downloaded {archive}"));
    done(&pb2, "checksum verified");

    let pb = stage("extracting…");
    let new_bin = extract_archive(&archive_path, tmp, &catalog.version, triple)?;
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
fn update_targets_retain_linux_and_apple_silicon_but_refuse_intel_macos() {
    for (os, arch, expected) in [
        ("linux", "x86_64", "x86_64-unknown-linux-musl"),
        ("linux", "aarch64", "aarch64-unknown-linux-musl"),
        ("macos", "aarch64", "aarch64-apple-darwin"),
    ] {
        assert_eq!(target_triple(os, arch).unwrap(), expected);
    }
    for arch in ["x86_64", "amd64"] {
        assert!(target_triple("macos", arch).is_err());
    }
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

#[cfg(test)]
#[test]
fn digest_mismatch_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let archive = dir.path().join("release.tar.gz");
    std::fs::write(&archive, b"release bytes").expect("fixture archive");
    let expected = format!("{:x}", Sha256::digest(b"different bytes"));
    assert_eq!(
        verify_sha256(&archive, &expected).unwrap_err().as_str(),
        "checksum mismatch — aborting"
    );
    let expected = format!("{:x}", Sha256::digest(b"release bytes"));
    verify_sha256(&archive, &expected).expect("matching digest");
}

#[cfg(test)]
fn write_test_archive(path: &Path, entry_path: &str, entry_type: tar::EntryType, contents: &[u8]) {
    let file = std::fs::File::create(path).expect("archive file");
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(entry_type);
    header.set_mode(0o755);
    header.set_size(contents.len() as u64);
    header.set_cksum();
    builder
        .append_data(&mut header, entry_path, contents)
        .expect("archive entry");
    let encoder = builder.into_inner().expect("finish tar");
    encoder.finish().expect("finish gzip");
}

#[cfg(test)]
#[test]
fn archive_layout_and_entry_types_are_enforced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let archive = dir.path().join("release.tar.gz");
    write_test_archive(
        &archive,
        "strop-9.9.9-other-target/strop",
        tar::EntryType::Regular,
        b"binary",
    );
    let error = extract_archive(&archive, dir.path(), "9.9.9", "test-target")
        .expect_err("wrong package layout");
    assert!(error.contains("unexpected path"), "{error}");

    write_test_archive(
        &archive,
        "strop-9.9.9-test-target/strop",
        tar::EntryType::Symlink,
        b"",
    );
    let error =
        extract_archive(&archive, dir.path(), "9.9.9", "test-target").expect_err("symlink binary");
    assert!(error.contains("invalid or duplicate binary"), "{error}");
}

#[cfg(test)]
#[test]
fn archive_extracts_binary_and_executable_permissions_in_process() {
    let dir = tempfile::tempdir().expect("tempdir");
    let archive = dir.path().join("release.tar.gz");
    let destination = dir.path().join("extracted");
    std::fs::create_dir(&destination).expect("destination");
    write_test_archive(
        &archive,
        "strop-9.9.9-test-target/strop",
        tar::EntryType::Regular,
        b"new strop binary",
    );
    let binary =
        extract_archive(&archive, &destination, "9.9.9", "test-target").expect("release extracts");
    assert_eq!(binary, destination.join("strop-9.9.9-test-target/strop"));
    assert_eq!(
        std::fs::read(&binary).expect("extracted binary"),
        b"new strop binary"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&binary)
                .expect("binary metadata")
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
}
