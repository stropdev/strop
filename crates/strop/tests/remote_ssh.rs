//! Real OpenSSH authentication + SFTP, over a private inetd connection: no
//! network, real HOME, host key discovery or sleeps. Required by Docker's gate.
#![cfg(unix)]
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
#[path = "remote_ssh/filesystem.rs"]
mod filesystem;
#[path = "remote_ssh/search.rs"]
mod search;
#[path = "remote_ssh/writes.rs"]
mod writes;

struct Fixture {
    directory: tempfile::TempDir,
    path: OsString,
}
fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}
fn successful(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap())
            .unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("bin")).unwrap();
        for name in ["host", "client", "denied"] {
            successful(
                Command::new("ssh-keygen")
                    .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                    .arg(root.join(name)),
            );
        }
        let public = std::fs::read_to_string(root.join("host.pub")).unwrap();
        std::fs::write(root.join("known_hosts"), format!("fixture {public}")).unwrap();
        std::fs::copy(root.join("client.pub"), root.join("authorized_keys")).unwrap();
        let username = String::from_utf8(successful(Command::new("id").arg("-un")).stdout).unwrap();
        let server = root.join("sshd_config");
        std::fs::write(&server, format!(
            "HostKey {}\nAuthorizedKeysFile {}\nStrictModes no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitRootLogin yes\nLogLevel ERROR\nSubsystem sftp internal-sftp\n",
            root.join("host").display(), root.join("authorized_keys").display()
        )).unwrap();
        // sshd -i speaks the REAL SSH protocol on stdin/stdout. OpenSSH's
        // ProxyCommand connects it without a network port or readiness race.
        let config = root.join("ssh_config");
        std::fs::write(&config, format!(
            "Host denied\n IdentityFile {}\nHost * !denied\n IdentityFile {}\nHost untrusted\n HostKeyAlias untrusted\nHost *\n HostName 127.0.0.1\n User {}\n IdentitiesOnly yes\n IdentityAgent none\n HostKeyAlias fixture\n UserKnownHostsFile {}\n GlobalKnownHostsFile /dev/null\n ProxyCommand /usr/sbin/sshd -i -e -f {}\n StrictHostKeyChecking no\n",
            root.join("denied").display(), root.join("client").display(), username.trim(),
            root.join("known_hosts").display(), quoted(&server)
        )).unwrap();
        // HOME alone does not isolate OpenSSH (it uses getpwuid). This launcher
        // selects the private config while executing the real system client.
        let wrapper = root.join("bin/ssh");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexec /usr/bin/ssh -F {} \"$@\"\n",
                quoted(&config)
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut paths = vec![root.join("bin")];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let path = std::env::join_paths(paths).unwrap();
        Self { directory, path }
    }
    fn root(&self) -> &Path {
        self.directory.path()
    }
    fn run(&self, args: &[OsString]) -> String {
        let output = successful(
            Command::new(env!("CARGO_BIN_EXE_strop"))
                .current_dir(self.root())
                .env("PATH", &self.path)
                .env("HOME", self.root().join("home"))
                .env("XDG_CONFIG_HOME", self.root().join("config"))
                .env("XDG_STATE_HOME", self.root().join("state"))
                .env_remove("STROP_LOG")
                .env_remove("STROP_REMOTE_PYTHON")
                .args(args),
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn script(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.root().join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }
    fn open(&self, uri: &str, steps: &str) -> String {
        let script = self.script("steps", steps);
        self.run(&["--headless".into(), script.into(), uri.into()])
    }
}
fn uri(host: &str, path: &Path) -> String {
    let mut uri = format!("ssh://{host}");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-') {
            uri.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(uri, "%{byte:02X}").unwrap();
        }
    }
    uri
}
fn states(output: &str) -> Vec<serde_json::Value> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("─── state "))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn openssh_reads_native_names_and_replay_needs_no_ssh() {
    if std::env::var_os("STROP_REQUIRE_SSH_TESTS").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let fixture = Fixture::new();
    let path = fixture
        .root()
        .join(std::ffi::OsStr::from_bytes(b"log \xff '$;\n.log"));
    // Crosses several SFTP read frames and contains a multi-byte grapheme.
    let body = format!("{}needle 日本語\nlast\n", "ordinary line\n".repeat(6000));
    std::fs::write(&path, &body).unwrap();
    let target = uri("fixture:2222", &path);
    let steps = fixture.script(
        "read.steps",
        "settle\nkeys /needle<cr>yy\nstate\nframe\nkeys :set noro<cr>dd\nstate\nkeys :q<cr>\n",
    );
    let trace = fixture.root().join("read.jsonl");
    let output = fixture.run(&[
        "--headless".into(),
        steps.into(),
        target.into(),
        "--log-file".into(),
        trace.clone().into(),
        "--log-content".into(),
    ]);
    let states = states(&output);
    assert_eq!(states[0]["line"], 6001);
    assert_eq!(states[1]["line"], 6001);
    assert!(!states[1]["dirty"].as_bool().unwrap());
    assert!(output.contains("needle 日本語"));
    // The capture is complete and replay consumes no external client at all.
    std::fs::remove_file(fixture.root().join("bin/ssh")).unwrap();
    std::fs::write(
        fixture.root().join("bin/ssh"),
        "#!/bin/sh\necho forbidden >&2\nexit 99\n",
    )
    .unwrap();
    std::fs::set_permissions(
        fixture.root().join("bin/ssh"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let replay = fixture.run(&["--replay".into(), trace.into()]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&replay).unwrap()["should_quit"],
        true
    );
}

#[test]
fn openssh_failures_keep_the_current_buffer_and_explain_the_cause() {
    if std::env::var_os("STROP_REQUIRE_SSH_TESTS").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let fixture = Fixture::new();
    let path = fixture.root().join("log.txt");
    std::fs::write(&path, "private log\n").unwrap();
    let missing = fixture.open(
        &uri("fixture", &fixture.root().join("missing")),
        "settle\nstate\n",
    );
    assert!(
        states(&missing)[0]["message"]
            .as_str()
            .unwrap()
            .contains("SFTP status 2"),
        "{missing}"
    );
    let refused = fixture.open(&uri("untrusted", &path), "settle\nstate\n");
    assert!(
        states(&refused)[0]["message"]
            .as_str()
            .unwrap()
            .contains("host key"),
        "{refused}"
    );
    let denied = fixture.open(&uri("denied", &path), "settle\nstate\n");
    assert!(
        states(&denied)[0]["message"]
            .as_str()
            .unwrap()
            .contains("noninteractively"),
        "{denied}"
    );
    let invalid = fixture.root().join("binary.log");
    std::fs::write(&invalid, [0xff]).unwrap();
    let invalid = fixture.open(&uri("fixture", &invalid), "settle\nstate\n");
    assert!(
        states(&invalid)[0]["message"]
            .as_str()
            .unwrap()
            .contains("invalid UTF-8"),
        "{invalid}"
    );
    let large = fixture.root().join("large.log");
    std::fs::File::create(&large)
        .unwrap()
        .set_len(256 * 1024 * 1024 + 1)
        .unwrap();
    let large = fixture.open(&uri("fixture", &large), "settle\nstate\n");
    assert!(
        states(&large)[0]["message"]
            .as_str()
            .unwrap()
            .contains("snapshot cap"),
        "{large}"
    );
}
