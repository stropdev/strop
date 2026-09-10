//! The local engine conversation: probe, discovery and identity
//! resolution. Every `docker` invocation is a supervised capture — argv
//! arrays, a caller's [`CancelToken`], a deadline and bounded retention —
//! and every non-zero exit is classified into a typed refusal.

use crate::identity::{validate_name, ContainerIdentity, ContainerRef};
use crate::ContainerError;
use std::process::Command;
use std::time::Duration;
use strop_core::process::{
    capture_with, stream_with, CaptureError, CapturePolicy, StdinPolicy, StreamError, StreamPolicy,
};
use strop_core::worker::CancelToken;

/// Wall-clock budget for the `docker info` probe.
const INFO_DEADLINE: Duration = Duration::from_secs(10);
/// Budget for discovery/inspect metadata commands.
const META_DEADLINE: Duration = Duration::from_secs(15);
/// Budget for one filesystem read (`docker cp` tar stream).
pub(crate) const READ_DEADLINE: Duration = Duration::from_secs(30);
/// Retained stderr for diagnostics (tails only ever reach errors).
const STDERR_LIMIT: u64 = 64 * 1024;
/// Retained stdout for metadata commands (`info`/`ps`/`inspect`).
const META_LIMIT: u64 = 4 * 1024 * 1024;
/// Retained metadata for one directory listing: the direct children's
/// names and kinds. The listing's tar stream itself is consumed
/// incrementally and never retained, so a subtree's bulk no longer
/// counts — only a listing with an absurd direct-child set is refused,
/// never presented partially.
pub(crate) const LIST_LIMIT: u64 = 16 * 1024 * 1024;

/// The local Docker engine, proven reachable by a bounded probe.
///
/// v1 is local-only by construction: there is no endpoint field to
/// parse, and there is deliberately no way to construct one without the
/// probe — a remote-engine field is earned with DC5, not reserved.
#[derive(Debug, Clone)]
pub struct EngineRef {
    server_version: String,
}

impl EngineRef {
    /// `ServerVersion` as the probe reported it.
    pub fn server_version(&self) -> &str {
        &self.server_version
    }
}

/// One supervised `docker` run's retained bytes — public for the
/// `exec_capture` boundary.
pub struct Captured {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_dropped: u64,
}

/// Run `docker <args>` under supervision: own process group, the caller's
/// token, `deadline`, bounded pipes, no stdin. Spawn failure means the
/// CLI itself is missing — that is the engine being unavailable.
pub(crate) fn capture(
    args: &[&str],
    stdout_limit: u64,
    deadline: Duration,
    token: &CancelToken,
) -> Result<Captured, ContainerError> {
    let mut command = Command::new("docker");
    command.args(args);
    let policy = CapturePolicy {
        stdout_limit,
        stderr_limit: STDERR_LIMIT,
        stderr_tail: 0,
        deadline,
        stdin: StdinPolicy::Null,
    };
    let output = capture_with(&mut command, token, &policy).map_err(|error| match error {
        CaptureError::Spawn(detail) => ContainerError::EngineUnavailable { detail },
        CaptureError::Cancelled => ContainerError::Cancelled,
        CaptureError::TimedOut(deadline) => ContainerError::Io {
            detail: format!(
                "docker {} timed out after {}s",
                args.first().copied().unwrap_or("<none>"),
                deadline.as_secs()
            ),
        },
        CaptureError::Failure(failure) => ContainerError::Io {
            detail: failure.message,
        },
    })?;
    Ok(Captured {
        code: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        stdout_dropped: output.stdout_dropped,
    })
}

/// One supervised `docker` run whose stdout streamed through a consumer.
pub(crate) struct Streamed {
    pub code: Option<i32>,
    pub stderr: Vec<u8>,
}

/// Run `docker <args>` under [`capture`]'s supervision while stdout
/// streams through `consume` chunk by chunk: nothing is retained by the
/// supervisor, so a transfer larger than any retention limit stays
/// bounded by what the consumer keeps. Stderr retention, the deadline
/// and cancellation behave exactly as in [`capture`]; a consumer error
/// kills the child and surfaces as its own typed [`ContainerError`].
pub(crate) fn stream(
    args: &[&str],
    deadline: Duration,
    token: &CancelToken,
    consume: impl FnMut(&[u8]) -> Result<(), ContainerError>,
) -> Result<Streamed, ContainerError> {
    let mut command = Command::new("docker");
    command.args(args);
    let policy = StreamPolicy {
        stderr_limit: STDERR_LIMIT,
        stderr_tail: 0,
        deadline,
    };
    let output =
        stream_with(&mut command, token, &policy, consume).map_err(|error| match error {
            StreamError::Spawn(detail) => ContainerError::EngineUnavailable { detail },
            StreamError::Cancelled => ContainerError::Cancelled,
            StreamError::TimedOut(deadline) => ContainerError::Io {
                detail: format!(
                    "docker {} timed out after {}s",
                    args.first().copied().unwrap_or("<none>"),
                    deadline.as_secs()
                ),
            },
            StreamError::Failure(failure) => ContainerError::Io {
                detail: failure.message,
            },
            StreamError::Consumer(error) => error,
        })?;
    Ok(Streamed {
        code: output.status.code(),
        stderr: output.stderr,
    })
}

/// The last bytes of stderr, lossy-decoded and single-lined — bounded
/// context for a typed failure, never an unbounded dump.
pub(crate) fn stderr_tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let tail: String = text.chars().rev().take(300).collect::<String>();
    tail.chars().rev().collect::<String>().replace('\n', " ")
}

/// Probe the local engine: `docker info` must succeed and report a
/// server version. CLI missing, daemon down or a timed-out probe are all
/// [`ContainerError::EngineUnavailable`].
pub fn engine(token: &CancelToken) -> Result<EngineRef, ContainerError> {
    let output = capture(
        &["info", "--format", "{{json .ServerVersion}}"],
        META_LIMIT,
        INFO_DEADLINE,
        token,
    )?;
    if output.code != Some(0) {
        return Err(ContainerError::EngineUnavailable {
            detail: stderr_tail(&output.stderr),
        });
    }
    let server_version: String =
        serde_json::from_slice(&output.stdout).map_err(|error| ContainerError::Protocol {
            detail: format!("docker info reported no server version: {error}"),
        })?;
    if server_version.is_empty() {
        return Err(ContainerError::Protocol {
            detail: "docker info reported an empty server version".into(),
        });
    }
    Ok(EngineRef { server_version })
}

/// The engine's running containers as full identities: `ps` for the id
/// set, one batched `inspect` for the records. Containers that stop in
/// between are dropped — this lists running containers only.
pub fn list_running(
    engine: &EngineRef,
    token: &CancelToken,
) -> Result<Vec<ContainerIdentity>, ContainerError> {
    let _ = engine;
    let output = capture(
        &["ps", "--quiet", "--no-trunc"],
        META_LIMIT,
        META_DEADLINE,
        token,
    )?;
    if output.code != Some(0) {
        return Err(ContainerError::EngineUnavailable {
            detail: stderr_tail(&output.stderr),
        });
    }
    let text = String::from_utf8(output.stdout).map_err(|_| ContainerError::Protocol {
        detail: "docker ps answered in non-UTF-8".into(),
    })?;
    let ids: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    for id in &ids {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ContainerError::Protocol {
                detail: format!("docker ps reported a non-canonical id {id:?}"),
            });
        }
    }
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec!["inspect"];
    args.extend(ids);
    let output = capture(&args, META_LIMIT, META_DEADLINE, token)?;
    inspect_records("docker ps ids", &output)?
        .into_iter()
        .filter(|record| record.state.running)
        .map(identity)
        .collect()
}

/// Resolve a name or id prefix to the container's identity, canonical
/// 64-hex id included. Names are validated before the engine sees them;
/// an unknown name is [`ContainerError::NoSuchContainer`], never a
/// best-effort guess.
pub fn inspect(
    engine: &EngineRef,
    name_or_id: &str,
    token: &CancelToken,
) -> Result<ContainerIdentity, ContainerError> {
    let _ = engine;
    validate_name(name_or_id)?;
    let output = capture(&["inspect", name_or_id], META_LIMIT, META_DEADLINE, token)?;
    identity(inspect_record(name_or_id, &output)?)
}

/// Re-resolve a previously inspected identity by name. The stale-identity
/// refusal lives here: a name that now maps to a different id, or the same
/// id restarted since, must not silently inherit the old identity's
/// reads, caches or completions.
pub fn revalidate(
    engine: &EngineRef,
    held: &ContainerIdentity,
    token: &CancelToken,
) -> Result<ContainerRef, ContainerError> {
    let current = inspect(engine, &held.name, token)?;
    if current.id != held.id || current.started_at != held.started_at {
        return Err(ContainerError::StaleIdentity {
            name: held.name.clone(),
            expected: format!("{}@{}", held.id, held.started_at),
            found: format!("{}@{}", current.id, current.started_at),
        });
    }
    ContainerRef::of(&current)
}

/// The cheap pre-read re-check: the container behind `reference` must
/// still exist, still be the same incarnation (`StartedAt`), and still be
/// running. One bounded `inspect` per read — the price of never serving
/// bytes from a restarted container as if they were the old one's.
pub(crate) fn refresh(
    engine: &EngineRef,
    reference: &ContainerRef,
    token: &CancelToken,
) -> Result<(), ContainerError> {
    let _ = engine;
    let id = reference.id().as_str();
    let output = capture(&["inspect", id], META_LIMIT, META_DEADLINE, token)?;
    let record = inspect_record(id, &output)?;
    if record.state.started_at != reference.started_at() {
        return Err(ContainerError::StaleIdentity {
            name: id.to_string(),
            expected: reference.incarnation(),
            found: format!("{id}@{}", record.state.started_at),
        });
    }
    if !record.state.running {
        return Err(ContainerError::NotRunning { id: id.to_string() });
    }
    Ok(())
}

/// The fields of `docker inspect`'s JSON record this backend consumes.
#[derive(serde::Deserialize)]
struct InspectRecord {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Config", default)]
    config: InspectConfig,
    #[serde(rename = "State", default)]
    state: InspectState,
}

#[derive(Default, serde::Deserialize)]
struct InspectConfig {
    #[serde(rename = "Image", default)]
    image: String,
    #[serde(rename = "User", default)]
    user: String,
    /// The container's working directory (Config.WorkingDir); empty means
    /// the image default ("/").
    #[serde(rename = "WorkingDir", default)]
    workdir: String,
}

#[derive(Default, serde::Deserialize)]
struct InspectState {
    #[serde(rename = "Running", default)]
    running: bool,
    #[serde(rename = "StartedAt", default)]
    started_at: String,
}

/// Decode the single-record inspect answer, classifying failure output.
fn inspect_record(name_or_id: &str, output: &Captured) -> Result<InspectRecord, ContainerError> {
    Ok(inspect_records(name_or_id, output)?.remove(0))
}

/// Decode an inspect answer's JSON array; a non-zero exit naming a
/// missing object is [`ContainerError::NoSuchContainer`].
fn inspect_records(what: &str, output: &Captured) -> Result<Vec<InspectRecord>, ContainerError> {
    if output.code != Some(0) {
        let tail = stderr_tail(&output.stderr);
        // Older daemons print "No such object", newer ones "no such object".
        if tail.to_ascii_lowercase().contains("no such object") {
            return Err(ContainerError::NoSuchContainer {
                name: what.to_string(),
            });
        }
        return Err(ContainerError::Io {
            detail: format!("docker inspect failed: {tail}"),
        });
    }
    if output.stdout_dropped > 0 {
        return Err(ContainerError::OutputTooLarge {
            what: "docker inspect output".into(),
        });
    }
    let records: Vec<InspectRecord> =
        serde_json::from_slice(&output.stdout).map_err(|error| ContainerError::Protocol {
            detail: format!("docker inspect answered malformed JSON: {error}"),
        })?;
    if records.is_empty() {
        return Err(ContainerError::Protocol {
            detail: format!("docker inspect of {what} returned no record"),
        });
    }
    Ok(records)
}

/// The identity view of one inspect record; the id must already be the
/// canonical 64-hex form (a short-id answer would mean the engine broke
/// its own contract).
fn identity(record: InspectRecord) -> Result<ContainerIdentity, ContainerError> {
    strop_workspace::ContainerId::canonical(record.id.clone()).map_err(|_| {
        ContainerError::Protocol {
            detail: format!("inspect id {:?} is not the canonical 64-hex id", record.id),
        }
    })?;
    Ok(ContainerIdentity {
        id: record.id,
        name: record.name.trim_start_matches('/').to_string(),
        image: record.config.image,
        started_at: record.state.started_at,
        user: record.config.user,
        workdir: record.config.workdir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captured(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Captured {
        Captured {
            code,
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
            stdout_dropped: 0,
        }
    }

    const INSPECT_JSON: &str = r#"[{
        "Id": "5b04229f99d2c4b8ae3c4e38b7a887cf1c5c1ea51f2a1b2c3d4e5f60718293a4",
        "Name": "/fixture",
        "Config": {"Image": "busybox:latest", "User": "root"},
        "State": {"Running": true, "StartedAt": "2026-09-10T08:00:00.123Z"}
    }]"#;

    #[test]
    fn inspect_json_decodes_to_identity() {
        let output = captured(Some(0), INSPECT_JSON.as_bytes(), b"");
        let identity = identity(inspect_record("fixture", &output).unwrap()).unwrap();
        assert_eq!(identity.id.len(), 64);
        assert_eq!(identity.name, "fixture", "leading slash stripped");
        assert_eq!(identity.image, "busybox:latest");
        assert_eq!(identity.started_at, "2026-09-10T08:00:00.123Z");
        assert_eq!(identity.user, "root");
    }

    #[test]
    fn missing_object_and_daemon_errors_classify() {
        let missing = captured(
            Some(1),
            b"[]",
            b"Error response from daemon: No such object: ghost\n",
        );
        assert!(matches!(
            inspect_record("ghost", &missing),
            Err(ContainerError::NoSuchContainer { .. })
        ));
        // Docker 29's lowercase daemon spelling.
        let lowercase = captured(Some(1), b"[]", b"error: no such object: ghost\n");
        assert!(matches!(
            inspect_record("ghost", &lowercase),
            Err(ContainerError::NoSuchContainer { .. })
        ));
        let down = captured(
            Some(1),
            b"",
            b"Cannot connect to the Docker daemon at unix:///var/run/docker.sock",
        );
        assert!(matches!(
            inspect_record("ghost", &down),
            Err(ContainerError::Io { .. })
        ));
        let malformed = captured(Some(0), b"[{]", b"");
        assert!(matches!(
            inspect_record("ghost", &malformed),
            Err(ContainerError::Protocol { .. })
        ));
    }

    #[test]
    fn non_canonical_ids_are_protocol_violations() {
        let json = INSPECT_JSON.replace(
            "5b04229f99d2c4b8ae3c4e38b7a887cf1c5c1ea51f2a1b2c3d4e5f60718293a4",
            "5b04229f99d2",
        );
        let output = captured(Some(0), json.as_bytes(), b"");
        assert!(matches!(
            inspect_record("x", &output).and_then(identity),
            Err(ContainerError::Protocol { .. })
        ));
    }

    #[test]
    fn stderr_tail_is_bounded_and_single_line() {
        let noisy = format!("{}\nfinal line", "x".repeat(1000));
        let tail = stderr_tail(noisy.as_bytes());
        assert!(tail.len() <= 300);
        assert!(tail.ends_with("final line"));
        assert!(!tail.contains('\n'));
    }
}
