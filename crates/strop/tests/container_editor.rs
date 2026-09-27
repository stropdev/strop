//! Real headless editor journey: selected container browse stays readonly;
//! explicit :container-worker admits a verified artifact inside that
//! incarnation, and :explain reports its target and digest.
#![cfg(unix)]

use sha2::Digest;
use std::ffi::OsStr;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use strop_core::worker::CancelToken;
use strop_worker_deploy::container::{ContainerProvider, ShellPolicy};
use strop_worker_deploy::provider::DeployProvider;

fn required() -> bool {
    std::env::var_os("STROP_CONTAINER_TESTS").as_deref() == Some(OsStr::new("1"))
}

fn unique_tag() -> String {
    let mut bytes = [0_u8; 8];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .unwrap();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct Fixture {
    id: String,
    directory: tempfile::TempDir,
    artifact: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert!(
            Command::new("docker")
                .arg("info")
                .output()
                .is_ok_and(|output| output.status.success()),
            "STROP_CONTAINER_TESTS=1 requires an accessible Docker engine"
        );
        let directory = tempfile::tempdir().unwrap();
        let tag = unique_tag();
        let container = Command::new("docker")
            .args([
                "run",
                "-d",
                "--label",
                &format!("strop-test-run={tag}"),
                "--name",
                &format!("strop-editor-worker-{tag}"),
                "busybox",
                "sleep",
                "300",
            ])
            .output()
            .unwrap();
        assert!(
            container.status.success(),
            "docker fixture could not start: {}",
            String::from_utf8_lossy(&container.stderr)
        );
        let id = String::from_utf8(container.stdout)
            .unwrap()
            .trim()
            .to_owned();
        let artifact = directory.path().join("strop-worker");
        let source = std::env::var_os("STROP_WORKER_BINARY")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_strop")));
        std::fs::copy(source, &artifact).unwrap();
        assert!(Command::new("strip")
            .arg(&artifact)
            .status()
            .unwrap()
            .success());
        assert!(
            std::fs::metadata(&artifact).unwrap().len() <= strop_worker_deploy::MAX_WORKER_BYTES,
            "native worker supply must fit the deployment bound"
        );
        Self {
            id,
            directory,
            artifact,
        }
    }

    fn script(&self, steps: &str) -> PathBuf {
        let path = self.directory.path().join("container.steps");
        std::fs::write(&path, steps).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.id]).output();
    }
}

fn states(output: &str) -> Vec<serde_json::Value> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("─── state "))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn container_worker_requires_explicit_command_and_never_grants_file_writes() {
    if !required() {
        eprintln!("skipping container editor journey: STROP_CONTAINER_TESTS is not 1");
        return;
    }
    let fixture = Fixture::new();
    // An older unleased verified cache entry is seeded in the selected
    // container context. The normal :container-worker command must
    // retire it through its actual worker, not a host-side fake GC.
    let (token, _handle) = CancelToken::standalone();
    let engine = strop_containers::engine(&token).unwrap();
    let identity = strop_containers::inspect(&engine, &fixture.id, &token).unwrap();
    let provider =
        ContainerProvider::capture(&engine, &identity, ShellPolicy::Required, &token).unwrap();
    let layout = strop_worker_deploy::cache::resolve(&provider).unwrap();
    let old_bytes = b"obsolete container worker";
    let old_sha = sha2::Sha256::digest(old_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    provider.write(&layout.object(&old_sha), old_bytes).unwrap();
    provider.set_mode(&layout.object(&old_sha), 0o500).unwrap();
    let receipt = strop_core::worker::cache_record::CacheReceipt {
        schema: strop_core::worker::cache_record::RECEIPT_SCHEMA,
        context: provider.endpoint().context.clone(),
        principal: provider.endpoint().principal.clone(),
        version: "0.34.0".into(),
        target: provider.endpoint().target.clone(),
        object_sha256: old_sha.clone(),
        object_bytes: old_bytes.len() as u64,
        tarball_sha256: "b".repeat(64),
    };
    provider
        .write(
            &layout.receipt(&old_sha),
            &serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
    // Positional arguments are native local paths. The explicit URI goes
    // through :e, which resolves and attaches the selected container.
    let steps = fixture.script(&format!(
        "settle\nkeys :e container:{}/etc/hostname<cr>\nsettle\nframe\nkeys :container-worker<cr>\nsettle\nstate\nkeys :explain<cr>\nframe\nkeys q\nkeys iFORBIDDEN<esc>\nstate\n",
        fixture.id
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_strop"))
        .arg("--headless")
        .arg(steps)
        .arg(fixture.directory.path())
        .env("STROP_WORKER_BINARY", &fixture.artifact)
        .env("HOME", fixture.directory.path().join("home"))
        .env("XDG_STATE_HOME", fixture.directory.path().join("state"))
        .env("XDG_CONFIG_HOME", fixture.directory.path().join("config"))
        .current_dir(fixture.directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "headless container editor: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8(output.stdout).unwrap();
    let observed = states(&output);
    let message = observed[0]["message"].as_str().unwrap();
    assert!(
        message.contains("container worker") && message.contains("ready for"),
        "{output}"
    );
    assert!(
        output.contains("[RO]"),
        "container file must stay readonly: {output}"
    );
    assert!(
        output.contains("[workers]") && output.contains("sha256:"),
        "{output}"
    );
    assert_eq!(
        observed[1]["dirty"], false,
        "readonly insertion cannot mutate: {output}"
    );
    assert!(provider.lstat(&layout.object(&old_sha)).unwrap().is_none());
    assert!(provider.lstat(&layout.receipt(&old_sha)).unwrap().is_none());
}
