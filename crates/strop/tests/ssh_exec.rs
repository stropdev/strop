//! Plan 0090 §5 over real OpenSSH: one multiplexed master per endpoint,
//! quoted argv on the host, and the pooled SFTP session's reads, saves
//! and listings with the changed-on-disk refusal. Nothing of strop is
//! installed on the host.
//!
//! Loopback TCP carries the real SSH protocol to a scoped OpenSSH daemon.
//! Gated by STROP_REQUIRE_SSH_TESTS=1.
#![cfg(unix)]
#[path = "ssh_exec/bench.rs"]
mod bench;
#[path = "ssh_exec/journeys.rs"]
mod journeys;

use std::io::Write as _;
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use strop_workspace::RemoteEndpoint;

/// Every test owns its private sshd listener and PATH wrapper. The lock
/// keeps process-global PATH scoped until the daemon is reaped.
static SERIAL: Mutex<()> = Mutex::new(());

struct ScopedFixture {
    fixture: Fixture,
    _serial: MutexGuard<'static, ()>,
}

/// `None` outside the Docker gate (`STROP_REQUIRE_SSH_TESTS=1`), with a
/// skip notice naming the caller: a host run never passes silently.
#[track_caller]
fn serial() -> Option<ScopedFixture> {
    if std::env::var_os("STROP_REQUIRE_SSH_TESTS").as_deref() != Some(std::ffi::OsStr::new("1")) {
        // Straight to stderr: libtest captures eprintln!, which would hide it.
        writeln!(
            std::io::stderr().lock(),
            "skipped ssh_exec journey at {}: set STROP_REQUIRE_SSH_TESTS=1 to run it",
            std::panic::Location::caller()
        )
        .unwrap();
        return None;
    }
    let serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    Some(ScopedFixture {
        fixture: Fixture::new(),
        _serial: serial,
    })
}

fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn successful(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

struct Sshd {
    child: Option<Child>,
}

impl Sshd {
    fn start(root: &Path, config: &Path) -> (Self, u16) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let log = root.join("sshd.log");
        let stderr = std::fs::File::create(&log).unwrap();
        let child = Command::new("/usr/sbin/sshd")
            .args(["-D", "-e", "-f"])
            .arg(config)
            .args(["-p", &port.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .spawn()
            .unwrap();
        let mut daemon = Self { child: Some(child) };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return (daemon, port);
            }
            if daemon.child.as_mut().unwrap().try_wait().unwrap().is_some()
                || Instant::now() >= deadline
            {
                panic!(
                    "scoped sshd did not bind loopback: {}",
                    std::fs::read_to_string(&log).unwrap_or_default()
                );
            }
            std::thread::yield_now();
        }
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    sshd: Sshd,
    original_path: std::ffi::OsString,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap())
            .unwrap();
        let root = directory.path();
        let original_path = std::env::var_os("PATH").unwrap();
        std::fs::create_dir(root.join("bin")).unwrap();
        std::fs::create_dir(root.join("home")).unwrap();
        for name in ["host", "client"] {
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
        let server_config = root.join("sshd_config");
        std::fs::write(
            &server_config,
            format!(
                "ListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\nStrictModes no\nUsePAM no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitRootLogin yes\nLogLevel ERROR\nSubsystem sftp internal-sftp\n",
                root.join("host").display(),
                root.join("authorized_keys").display()
            ),
        )
        .unwrap();
        // A real listening sshd, not `sshd -i` on a ProxyCommand pipe.
        let (sshd, port) = Sshd::start(root, &server_config);
        let config = root.join("ssh_config");
        std::fs::write(&config, format!(
            "Host *\n IdentityFile {}\n HostName 127.0.0.1\n Port {port}\n User {}\n IdentitiesOnly yes\n IdentityAgent none\n HostKeyAlias fixture\n UserKnownHostsFile {}\n GlobalKnownHostsFile /dev/null\n StrictHostKeyChecking no\n",
            root.join("client").display(),
            username.trim(),
            root.join("known_hosts").display(),
        )).unwrap();
        // The wrapper selects the private config without touching the
        // real $HOME.
        let wrapper = root.join("bin/ssh");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexport HOME={}\nexec /usr/bin/ssh -F {} \"$@\"\n",
                quoted(&root.join("home")),
                quoted(&config)
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut paths = vec![root.join("bin")];
        paths.extend(std::env::split_paths(&original_path));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        Self {
            directory,
            sshd,
            original_path,
        }
    }

    fn root(&self) -> &Path {
        self.directory.path()
    }

    /// The private control-socket directory (short: `sun_path` bound).
    fn control(&self) -> PathBuf {
        self.root().join("ctl")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Masters left by a failed assertion would outlive the daemon.
        let _ = Command::new("pkill")
            .arg("-f")
            .arg(self.control().as_os_str())
            .status();
        self.sshd.stop();
        std::env::set_var("PATH", &self.original_path);
    }
}

fn endpoint(alias: &str) -> RemoteEndpoint {
    RemoteEndpoint::parse(&format!("ssh://{alias}")).unwrap()
}

/// A live token: the handle must be held for the whole test (dropping
/// it cancels).
fn token() -> (
    strop_core::worker::CancelToken,
    strop_core::worker::CancelHandle,
) {
    strop_core::worker::CancelToken::standalone()
}

#[test]
fn scoped_loopback_ssh_daemon_is_reaped_after_each_fixture() {
    let Some(scope) = serial() else { return };
    let pid = scope.fixture.sshd.child.as_ref().unwrap().id();
    drop(scope);
    // SAFETY: signal 0 only observes the PID; the owning Child was
    // killed and waited by this fixture's Drop before this check.
    let alive = unsafe { libc::kill(pid as libc::pid_t, 0) };
    assert_eq!(alive, -1, "this fixture left an sshd child alive");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH),
        "the exact child PID must have been reaped"
    );
}
