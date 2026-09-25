//! The container deployment provider (0058 WK08): the WK06 state
//! machine's [`DeployProvider`] over a selected, incarnation-pinned
//! container, plus the wiring from the captured context to a live
//! worker lease.
//!
//! Two admission paths, per the plan's container contract:
//!
//! - **Automatic deployment** ([`ShellPolicy::Required`]): the verified
//!   local worker artifact crosses as a scoped tar transfer
//!   ([`strop_containers::write_file`]) stamped with the selected
//!   principal's numeric uid/gid — the daemon extracts as container
//!   root, so the header is what keeps the cache principal-owned. Cache
//!   directory/mode/rename/remove operations are scoped single-program
//!   execs through the AR07 supervisor, as the selected principal,
//!   never root. The worker is then exec'd supervised (relay mode): its
//!   stdin is the AR07 lifetime lease.
//! - **Verified preinstalled worker** ([`ShellPolicy::Absent`]): the
//!   shellless/distroless case. Nothing is deployed; WK06 validates the
//!   object in place (regular file, no symlink, owner-exec, hashed in
//!   place) and the worker is exec'd *directly* through
//!   [`AdmittedExec::worker_command`] — the worker binary is its own
//!   supervisor and needs no shell utilities. No claim that every
//!   distroless image can be auto-provisioned: a `Required` endpoint on
//!   a shell-less image refuses truthfully at the first cache exec.
//!
//! Identity and elevation rules held here:
//! - The endpoint context is `docker:<canonical-id>@<started-at>`; a
//!   renamed/restarted container is a different endpoint and every
//!   operation re-checks the incarnation at admission (AR07).
//! - The principal is the selected user's *numeric* uid: tar headers
//!   carry numeric ids and the image's `/etc/passwd` is data, not an
//!   identity guarantee. Names resolve shell-free through a bounded
//!   read of `/etc/passwd`.
//! - The cache base comes from the selected context's environment
//!   (XDG_CACHE_HOME, else HOME) or the passwd entry — never guessed,
//!   never a global `/tmp` name.
//! - No image mutation, no bind mount, no container restart/rebuild,
//!   no chown/chmod-into-compliance: the provider surface has no such
//!   operation.

use std::io;
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Duration;

use strop_containers::{
    image_platform, lstat, read_file, write_file, AdmittedExec, ContainerError, ContainerIdentity,
    ContainerRef, DirEntryKind, EngineRef, ExecSpec, PathStat, TransferMeta,
};
use strop_core::worker::{CancelHandle, CancelToken};
use strop_worker_client::{StderrCapture, Transport, Worker};
use strop_worker_protocol::codec::{self, Incoming};
use strop_worker_protocol::frame::{self, FrameDecoder};
use strop_worker_protocol::{
    ClientMessage, EndpointInfo, WorkerMessage, PROTOCOL_VERSION, TARGET_TRIPLE,
};

use crate::provider::{
    DeployProvider, EndpointIdentity, HandshakeReport, ProviderError, RemoteKind, RemoteStat,
};

/// Handshake bound: spawn plus hello/welcome, mirroring the client.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Retained worker/supervisor stderr for typed diagnostics.
const STDERR_LIMIT: usize = 64 * 1024;

/// Whether the image's POSIX `sh` may carry the AR07 supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellPolicy {
    /// Automatic deployment: cache/exec operations run through the
    /// supervisor, and the worker is exec'd supervised. A shell-less
    /// image fails truthfully at the first exec.
    Required,
    /// Verified preinstalled worker: no deployment happens, no shell is
    /// touched; the worker execs directly and is its own supervisor.
    Absent,
}

/// A container endpoint captured from the selected context: probe-
/// pinned engine, incarnation-pinned container, resolved principal and
/// cache base, image-bound target triple. Construct through
/// [`ContainerProvider::capture`]; everything after is the WK06 machine.
pub struct ContainerProvider {
    engine: EngineRef,
    reference: ContainerRef,
    endpoint: EndpointIdentity,
    /// `Config.User` passthrough for `docker exec --user` (the daemon
    /// resolves names and applies supplementary groups); `None` when
    /// the image default applies.
    user_arg: Option<String>,
    /// The principal's numeric ids, stamped on every tar transfer.
    uid: u32,
    gid: u32,
    cache_base: String,
    /// The selected working directory for the worker process.
    workdir: String,
    shell: ShellPolicy,
    token: CancelToken,
    /// Dropping the provider cancels its token: in-flight operations
    /// unwind typed instead of leaking daemon sessions.
    #[allow(dead_code)]
    handle: CancelHandle,
}

/// One `/etc/passwd` record's facts, decimal strings.
struct PasswdEntry {
    uid: u32,
    gid: u32,
    home: String,
}

/// Parse `/etc/passwd` bytes for `name`. Malformed lines are skipped;
/// an absent name is `None`, never a guessed uid.
fn passwd_entry(bytes: &[u8], name: &str) -> Option<PasswdEntry> {
    let text = std::str::from_utf8(bytes).ok()?;
    for line in text.lines() {
        let mut fields = line.split(':');
        if fields.next() != Some(name) {
            continue;
        }
        let _password = fields.next();
        let uid: u32 = fields.next()?.parse().ok()?;
        let gid: u32 = fields.next()?.parse().ok()?;
        let _gecos = fields.next();
        let entry = PasswdEntry {
            uid,
            gid,
            home: fields.next()?.to_string(),
        };
        return Some(entry);
    }
    None
}

/// An environment entry's value (`KEY=value`), exact match only.
fn env_value(env: &[String], key: &str) -> Option<String> {
    env.iter()
        .find_map(|entry| {
            entry
                .strip_prefix(key)
                .and_then(|rest| rest.strip_prefix('='))
        })
        .map(str::to_string)
}

/// The image platform's catalog target: WK05's Linux worker is the
/// static musl artifact for the image's architecture (musl-static runs
/// on any Linux libc). Anything else is no-artifact truth, surfaced by
/// the caller as a typed refusal.
fn catalog_target(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "amd64") => Some("x86_64-unknown-linux-musl"),
        ("linux", "arm64") => Some("aarch64-unknown-linux-musl"),
        _ => None,
    }
}

/// Classify a failed scoped exec op into the provider taxonomy. The
/// strings are the POSIX tools' own diagnostics on stderr, bounded by
/// the capture; an unrecognized failure is transport-class detail,
/// never silently retried.
fn classify_op(program: &str, code: Option<i32>, stderr: &[u8]) -> ProviderError {
    let tail = String::from_utf8_lossy(stderr);
    let tail = tail.trim();
    let detail = format!("{program} exited {code:?}: {tail}");
    if tail.contains("Read-only file system") {
        ProviderError::ReadOnly(detail)
    } else if tail.contains("No space left on device") {
        ProviderError::DiskFull(detail)
    } else if tail.contains("Permission denied") {
        ProviderError::Permission(detail)
    } else if tail.contains("No such file or directory") {
        ProviderError::NotFound(detail)
    } else {
        ProviderError::Transport(detail)
    }
}

/// Container conversation failures fold into provider failures. The
/// stale-incarnation/incarnation-gone cases name themselves in the
/// message — the deploy report carries the precise reason.
fn map_engine(error: ContainerError) -> ProviderError {
    match error {
        ContainerError::NoSuchPath { path, .. } => ProviderError::NotFound(path),
        ContainerError::ExecLaunch { detail } => ProviderError::NoExec(detail),
        other => ProviderError::Transport(other.to_string()),
    }
}

fn io_failed(error: ContainerError) -> io::Error {
    io::Error::other(error.to_string())
}

impl ContainerProvider {
    /// Capture the endpoint from the selected context: the already
    /// inspected identity (canonical id + `StartedAt` incarnation,
    /// user, cwd, image environment) pins everything deployment,
    /// execution and cleanup will use. Resolution is shell-free (it
    /// reads `/etc/passwd` through the read-only tar facility when the
    /// principal is a name), so the preinstalled path works on a
    /// genuinely shell-less image.
    pub fn capture(
        engine: &EngineRef,
        identity: &ContainerIdentity,
        shell: ShellPolicy,
        token: &CancelToken,
    ) -> Result<Self, ProviderError> {
        let reference = ContainerRef::of(identity).map_err(map_engine)?;
        // Principal resolution: empty is the engine's default (root);
        // numeric is itself; a name resolves through the container's
        // own /etc/passwd, read shell-free.
        let (uid, gid, passwd_home) = if identity.user.is_empty() {
            (0, 0, None)
        } else if identity.user.bytes().all(|b| b.is_ascii_digit()) {
            let uid: u32 = identity
                .user
                .parse()
                .map_err(|_| ProviderError::Permission(format!("bad uid {:?}", identity.user)))?;
            (uid, uid, None)
        } else {
            let passwd = read_file(engine, &reference, "/etc/passwd", 1024 * 1024, token)
                .map_err(map_engine)?;
            let entry = passwd_entry(&passwd, &identity.user).ok_or_else(|| {
                ProviderError::Permission(format!(
                    "principal {:?} does not resolve in the container's /etc/passwd",
                    identity.user
                ))
            })?;
            (entry.uid, entry.gid, Some(entry.home))
        };
        let home = env_value(&identity.env, "HOME")
            .filter(|home| home.starts_with('/'))
            .or(passwd_home.filter(|home| home.starts_with('/')));
        let base = env_value(&identity.env, "XDG_CACHE_HOME")
            .filter(|base| base.starts_with('/'))
            .map(|base| format!("{base}/strop"))
            .or_else(|| home.map(|home| format!("{home}/.cache")))
            .or_else(|| (uid == 0).then(|| "/root/.cache".to_string()));
        let cache_base = base.ok_or_else(|| {
            ProviderError::Permission(format!(
                "no usable cache location for uid {uid}: the image sets neither HOME nor XDG_CACHE_HOME"
            ))
        })?;
        let platform = image_platform(engine, &identity.image, token).map_err(map_engine)?;
        let target = catalog_target(&platform.os, &platform.arch)
            .ok_or_else(|| {
                ProviderError::NoExec(format!(
                    "no worker artifact for container platform {}/{}",
                    platform.os, platform.arch
                ))
            })?
            .to_string();
        let (op_token, handle) = CancelToken::standalone();
        Ok(Self {
            engine: engine.clone(),
            endpoint: EndpointIdentity {
                context: format!("docker:{}@{}", reference.id(), reference.started_at()),
                principal: uid.to_string(),
                target,
            },
            reference,
            user_arg: (!identity.user.is_empty()).then(|| identity.user.clone()),
            uid,
            gid,
            cache_base,
            workdir: if identity.workdir.is_empty() {
                "/".to_string()
            } else {
                identity.workdir.clone()
            },
            shell,
            token: op_token,
            handle,
        })
    }

    /// The transfer stamp for every byte deployment writes: the
    /// selected principal's numeric ownership.
    fn meta(&self, mode: u32) -> TransferMeta {
        TransferMeta {
            mode,
            uid: self.uid,
            gid: self.gid,
        }
    }

    /// One scoped single-program exec through the AR07 supervisor, as
    /// the selected principal, in `/`. Admission re-checks the
    /// incarnation; a non-zero exit classifies into the provider
    /// taxonomy.
    fn run(&self, program: &str, args: &[String]) -> Result<(), ProviderError> {
        let spec = ExecSpec::new(&self.engine, &self.reference, program, args, Path::new("/"))
            .map_err(map_engine)?;
        let spec = match &self.user_arg {
            Some(user) => spec.with_user(user).map_err(map_engine)?,
            None => spec,
        };
        let admitted = spec.admit(&self.token).map_err(map_engine)?;
        let output = admitted
            .capture(STDERR_LIMIT as u64, &self.token)
            .map_err(map_engine)?;
        if output.code == Some(0) {
            Ok(())
        } else {
            Err(classify_op(program, output.code, &output.stderr))
        }
    }

    /// The admitted exec for the worker object: supervised on a
    /// `Required` endpoint (the AR07 relay lease), direct on an
    /// `Absent` one (the worker is its own supervisor). Admission
    /// re-checks the incarnation *now* — the lease factory calls this
    /// on every (re)connect, so a restarted container can never be
    /// retargeted by a stale lease.
    fn worker_exec(
        &self,
        object: &str,
        token: &CancelToken,
    ) -> Result<AdmittedExec, ProviderError> {
        let spec = ExecSpec::new(
            &self.engine,
            &self.reference,
            object,
            &["--worker-stdio".to_string()],
            Path::new(&self.workdir),
        )
        .map_err(map_engine)?;
        let spec = match &self.user_arg {
            Some(user) => spec.with_user(user).map_err(map_engine)?,
            None => spec,
        };
        spec.admit(token).map_err(map_engine)
    }

    /// A worker lease on the verified object (0058 WK08 engine wiring):
    /// the editor's read/write/notify for this container's workspace
    /// ride this connection. The handshake binds the worker's reported
    /// target to the endpoint's catalog target (a foreign-platform
    /// worker is a typed mismatch, never a downgrade). Cleanup rides
    /// the AR07 supervised lease: dropping the lease's last clone kills
    /// the local exec, the daemon closes stdin, and the in-container
    /// supervisor tears the group down (TERM, grace, KILL).
    pub fn worker(&self, object: &str) -> Worker {
        let engine = self.engine.clone();
        let reference = self.reference.clone();
        let user_arg = self.user_arg.clone();
        let workdir = self.workdir.clone();
        let shell = self.shell;
        let program = object.to_string();
        Worker::connect_deployed(self.endpoint.target.clone(), move || {
            let (token, handle) = CancelToken::standalone();
            let spec = ExecSpec::new(
                &engine,
                &reference,
                &program,
                &["--worker-stdio".to_string()],
                Path::new(&workdir),
            )
            .map_err(io_failed)?;
            let spec = match &user_arg {
                Some(user) => spec.with_user(user).map_err(io_failed)?,
                None => spec,
            };
            let admitted = spec.admit(&token).map_err(io_failed)?;
            let mut command = match shell {
                ShellPolicy::Required => admitted.command(),
                ShellPolicy::Absent => admitted.worker_command(),
            };
            let spawned = command.spawn();
            drop(handle); // admission is done; the op token retires here
            let mut child = spawned?;
            let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::other("worker stdio pipes unavailable"));
            };
            let stderr = child.stderr.take().map(StderrCapture::spawn);
            Ok(Transport {
                reader: Box::new(stdout),
                writer: Box::new(stdin),
                child: Some(child),
                stderr,
            })
        })
    }
}

impl DeployProvider for ContainerProvider {
    fn endpoint(&self) -> &EndpointIdentity {
        &self.endpoint
    }

    fn cache_base(&self) -> Result<String, ProviderError> {
        Ok(self.cache_base.clone())
    }

    fn lstat(&self, path: &str) -> Result<Option<RemoteStat>, ProviderError> {
        let stat = lstat(&self.engine, &self.reference, path, &self.token).map_err(map_engine)?;
        Ok(stat.map(
            |PathStat {
                 kind,
                 mode,
                 uid,
                 len,
                 ..
             }| RemoteStat {
                kind: match kind {
                    DirEntryKind::File => RemoteKind::File,
                    DirEntryKind::Dir => RemoteKind::Dir,
                    DirEntryKind::Symlink => RemoteKind::Symlink,
                    DirEntryKind::Other => RemoteKind::Other,
                },
                mode,
                owner: uid.to_string(),
                len,
            },
        ))
    }

    fn mkdir_private(&self, path: &str) -> Result<(), ProviderError> {
        // `mkdir -p` leaves existing components untouched; the final
        // component is the caller's to create private (WK06 resolved it
        // absent first). Fresh intermediate parents get the container's
        // umask default and are validated individually by the machine.
        self.run("mkdir", &["-p".to_string(), path.to_string()])?;
        self.run("chmod", &["700".to_string(), path.to_string()])
    }

    fn upload(&self, source: &Path, dest: &str) -> Result<(), ProviderError> {
        let bytes = std::fs::read(source)
            .map_err(|error| ProviderError::Transport(format!("read local artifact: {error}")))?;
        write_file(
            &self.engine,
            &self.reference,
            dest,
            &bytes,
            self.meta(0o600),
            &self.token,
        )
        .map_err(map_engine)
    }

    fn write(&self, dest: &str, bytes: &[u8]) -> Result<(), ProviderError> {
        write_file(
            &self.engine,
            &self.reference,
            dest,
            bytes,
            self.meta(0o600),
            &self.token,
        )
        .map_err(map_engine)
    }

    fn fetch(&self, path: &str, max: u64) -> Result<Vec<u8>, ProviderError> {
        read_file(&self.engine, &self.reference, path, max, &self.token).map_err(map_engine)
    }

    fn set_mode(&self, path: &str, mode: u32) -> Result<(), ProviderError> {
        self.run("chmod", &[format!("{mode:o}"), path.to_string()])
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), ProviderError> {
        // One `mv` within the cache filesystem: atomic same-directory
        // rename, content-addressed destination (a concurrent identical
        // publish is harmless), never a cross-device copy.
        self.run("mv", &[from.to_string(), to.to_string()])
    }

    fn remove(&self, path: &str) -> Result<(), ProviderError> {
        // One positively identified file, never recursive, never a glob.
        self.run("rm", &[path.to_string()])
    }

    fn list(&self, dir: &str) -> Result<Vec<String>, ProviderError> {
        let entries = strop_containers::list_dir(&self.engine, &self.reference, dir, &self.token)
            .map_err(map_engine)?;
        Ok(entries.into_iter().map(|entry| entry.name).collect())
    }
    fn handshake(&self, object: &str) -> Result<HandshakeReport, ProviderError> {
        let admitted = self.worker_exec(object, &self.token)?;
        let mut command = match self.shell {
            ShellPolicy::Required => admitted.command(),
            ShellPolicy::Absent => admitted.worker_command(),
        };
        let mut child = command
            .spawn()
            .map_err(|error| ProviderError::Transport(format!("docker exec spawn: {error}")))?;
        let result = (|| {
            let (Some(mut writer), Some(reader)) = (child.stdin.take(), child.stdout.take()) else {
                return Err(ProviderError::Handshake(
                    "worker stdio pipes unavailable".to_string(),
                ));
            };
            let stderr = child.stderr.take().map(StderrCapture::spawn);
            codec::write_envelope(
                &mut writer,
                &ClientMessage::Hello {
                    protocol: PROTOCOL_VERSION,
                    client: EndpointInfo {
                        name: "strop".into(),
                        version: env!("CARGO_PKG_VERSION").into(),
                        build: None,
                        target: TARGET_TRIPLE.to_string(),
                    },
                },
            )
            .map_err(|error| ProviderError::Handshake(format!("cannot send hello: {error}")))?;
            // The welcome read is bounded: a half-alive worker can never
            // park deployment. Dropping the writer after the welcome is
            // the lease close — EOF ends the in-container session.
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let mut reader = reader;
                let mut decoder = FrameDecoder::default();
                let read = frame::read_frame(&mut reader, &mut decoder)
                    .map_err(|error| error.to_string())
                    .and_then(|body| {
                        body.ok_or_else(|| "worker closed the stream before welcome".to_string())
                    })
                    .and_then(|body| {
                        codec::decode_body::<WorkerMessage>(&body)
                            .map_err(|error| error.to_string())
                    });
                let _ = tx.send(read);
            });
            let welcome = rx.recv_timeout(HANDSHAKE_TIMEOUT).map_err(|_| {
                let mut detail = format!("no welcome within {}s", HANDSHAKE_TIMEOUT.as_secs());
                if let Some(capture) = &stderr {
                    let text = capture.text();
                    if !text.is_empty() {
                        detail.push_str("; worker stderr: ");
                        detail.push_str(&text);
                    }
                }
                ProviderError::Handshake(detail)
            })?;
            // Shell/interpreter diagnostics can never impersonate a
            // welcome here: only the decoded frame counts.
            match welcome.map_err(ProviderError::Handshake)? {
                Incoming::Envelope(WorkerMessage::Welcome {
                    protocol,
                    worker,
                    session,
                    ..
                }) => Ok(HandshakeReport {
                    protocol,
                    worker,
                    session,
                }),
                Incoming::Envelope(WorkerMessage::Error { error, .. }) => {
                    Err(ProviderError::Handshake(format!("refused: {error}")))
                }
                _ => Err(ProviderError::Handshake(
                    "the first worker message was not a welcome".to_string(),
                )),
            }
        })();
        // Teardown: close the lease (stdin EOF), then reap the local
        // exec. The in-container group teardown follows on the daemon
        // side (AR07); activation never leaves a worker running.
        let _ = child.kill();
        let _ = child.wait();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwd_entries_resolve_names_to_numeric_principals() {
        let passwd =
            b"root:x:0:0:root:/root:/bin/sh\nalice:x:1000:1001:Alice:/home/alice:/bin/sh\n";
        let alice = passwd_entry(passwd, "alice").unwrap();
        assert_eq!((alice.uid, alice.gid), (1000, 1001));
        assert_eq!(alice.home, "/home/alice");
        assert_eq!(passwd_entry(passwd, "root").unwrap().uid, 0);
        assert!(passwd_entry(passwd, "mallory").is_none());
    }

    #[test]
    fn cache_location_prefers_xdg_then_home_then_passwd() {
        let env = vec!["HOME=/home/alice".to_string(), "PATH=/bin".to_string()];
        assert_eq!(env_value(&env, "HOME").as_deref(), Some("/home/alice"));
        assert!(env_value(&env, "XDG_CACHE_HOME").is_none());
        assert!(env_value(&env, "HOM").is_none(), "exact keys only");
        let xdg = vec![
            "HOME=/home/alice".to_string(),
            "XDG_CACHE_HOME=/cache".to_string(),
        ];
        assert_eq!(env_value(&xdg, "XDG_CACHE_HOME").as_deref(), Some("/cache"));
    }

    #[test]
    fn platform_maps_to_the_musl_catalog_targets() {
        assert_eq!(
            catalog_target("linux", "amd64"),
            Some("x86_64-unknown-linux-musl")
        );
        assert_eq!(
            catalog_target("linux", "arm64"),
            Some("aarch64-unknown-linux-musl")
        );
        assert_eq!(catalog_target("linux", "riscv64"), None);
        assert_eq!(catalog_target("windows", "amd64"), None);
    }

    #[test]
    fn op_failures_classify_truthfully() {
        assert!(matches!(
            classify_op(
                "mkdir",
                Some(1),
                b"mkdir: can't create directory '/x': Read-only file system"
            ),
            ProviderError::ReadOnly(_)
        ));
        assert!(matches!(
            classify_op("mv", Some(1), b"mv: can't rename: No space left on device"),
            ProviderError::DiskFull(_)
        ));
        assert!(matches!(
            classify_op("rm", Some(1), b"rm: /x: Permission denied"),
            ProviderError::Permission(_)
        ));
        assert!(matches!(
            classify_op("stat", Some(1), b"stat: /x: No such file or directory"),
            ProviderError::NotFound(_)
        ));
        assert!(matches!(
            classify_op("mv", Some(2), b"mv: something unusual"),
            ProviderError::Transport(_)
        ));
    }

    #[test]
    fn engine_failures_map_to_provider_taxonomy() {
        assert!(matches!(
            map_engine(ContainerError::StaleIdentity {
                name: "c".into(),
                expected: "a".into(),
                found: "b".into(),
            }),
            ProviderError::Transport(_)
        ));
        assert!(matches!(
            map_engine(ContainerError::ExecLaunch {
                detail: "no sh".into()
            }),
            ProviderError::NoExec(_)
        ));
        assert!(matches!(
            map_engine(ContainerError::NoSuchPath {
                id: "c".into(),
                path: "/x".into()
            }),
            ProviderError::NotFound(_)
        ));
    }
}
