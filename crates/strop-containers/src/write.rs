//! Scoped file transfer *into* the container (0058 WK08): one regular
//! file, one positively named destination, through the engine's own
//! file facility — `docker cp - <id>:<parent>` extracts a tar archive
//! from stdin inside the container's mount namespace. No `sh`/`cat`/
//! `install` has to exist in the image, nothing passes through a shell,
//! and no bind mount is guessed: the bytes cross the same boundary the
//! read path uses, in reverse.
//!
//! Ownership and mode are tar-header facts set to the *selected
//! principal's* numeric ids: the daemon extracts as container root, so
//! the header is the only thing standing between the transfer and a
//! root-owned cache — the plan's "the daemon's wider access is not
//! authorization to write as root". This module never creates
//! directories, never follows or writes through symlinks (the archive
//! names exactly one regular file), and never touches the image.
//!
//! Every transfer first re-checks the container's incarnation
//! ([`refresh`]): a restarted container is [`ContainerError::StaleIdentity`],
//! never bytes written to the wrong filesystem.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use strop_core::process::OwnedProcess;
use strop_core::worker::CancelToken;

use crate::engine::{refresh, stderr_tail, EngineRef};
use crate::identity::ContainerRef;
use crate::ContainerError;

/// Wall-clock budget for one transfer. The worker-object ceiling is
/// 256 MiB; this bounds a stalled daemon, not honest throughput.
const WRITE_DEADLINE: Duration = Duration::from_secs(120);
/// Retained stderr for diagnostics (tails only ever reach errors).
const STDERR_LIMIT: usize = 64 * 1024;
const POLL: Duration = Duration::from_millis(20);
const BLOCK: usize = 512;

/// One POSIX ustar header for a regular file. Deployment paths are
/// cache-layout names (digests, staging hex, record files) — always far
/// under the 100-byte name field; anything longer is refused, never a
/// truncated name.
fn header(
    name: &str,
    size: u64,
    mode: u32,
    uid: u32,
    gid: u32,
) -> Result<[u8; BLOCK], ContainerError> {
    if name.is_empty() || name.len() > 100 || name.contains(['/', '\0']) {
        return Err(ContainerError::CapabilityRefused {
            what: format!("write_file: unsupported archive name {name:?}"),
        });
    }
    let mut block = [0u8; BLOCK];
    block[..name.len()].copy_from_slice(name.as_bytes());
    let mut put_octal = |range: std::ops::Range<usize>, value: u64| {
        let text = format!("{:0>width$o}\0", value, width = range.len() - 1);
        block[range].copy_from_slice(text.as_bytes());
    };
    put_octal(100..108, mode as u64);
    put_octal(108..116, uid as u64);
    put_octal(116..124, gid as u64);
    put_octal(124..136, size);
    put_octal(136..148, 0); // mtime: deployments are content-addressed
    block[156] = b'0';
    block[257..262].copy_from_slice(b"ustar");
    block[263..265].copy_from_slice(b"00");
    block[148..156].copy_from_slice(b"        ");
    let sum: u64 = block.iter().map(|&byte| byte as u64).sum();
    let text = format!("{sum:06o}\0 ");
    block[148..156].copy_from_slice(text.as_bytes());
    Ok(block)
}

/// The tar-header facts one transfer stamps: permission bits and the
/// selected principal's numeric ownership. Bundled because they are
/// one concept — the identity the extracted file will carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferMeta {
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

/// Write `bytes` as the regular file `path` inside the pinned
/// container, carrying `meta`'s ownership and permission bits. The
/// parent directory must already exist (deployment creates it through
/// the scoped exec surface first); a missing parent, a stopped
/// container or a read-only destination are typed engine failures with
/// the daemon's bounded stderr tail.
pub fn write_file(
    engine: &EngineRef,
    id: &ContainerRef,
    path: &str,
    bytes: &[u8],
    meta: TransferMeta,
    token: &CancelToken,
) -> Result<(), ContainerError> {
    if !path.starts_with('/') || path.ends_with('/') {
        return Err(ContainerError::CapabilityRefused {
            what: format!("write_file: not an absolute file path: {path:?}"),
        });
    }
    let (parent, name) = path.rsplit_once('/').unwrap_or(("/", path));
    let parent = if parent.is_empty() { "/" } else { parent };
    let head = header(name, bytes.len() as u64, meta.mode, meta.uid, meta.gid)?;
    refresh(engine, id, token)?;

    let mut pinned = engine.context_args();
    let target = format!("{}:{parent}", id.id());
    pinned.extend_from_slice(&["cp", "-", &target]);
    let mut command = Command::new("docker");
    command
        .args(&pinned)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut process =
        OwnedProcess::spawn(&mut command, token).map_err(|error| ContainerError::Io {
            detail: format!("docker cp spawn: {}", error.message),
        })?;
    let mut stdin = process.take_stdin().ok_or_else(|| ContainerError::Io {
        detail: "docker cp stdin pipe unavailable".into(),
    })?;
    let mut stderr_pipe = process.take_stderr().ok_or_else(|| ContainerError::Io {
        detail: "docker cp stderr pipe unavailable".into(),
    })?;

    // The archive streams in from a writer thread while stderr drains
    // concurrently, bounded — a blocked daemon never deadlocks the
    // transfer on a full pipe. The writer is a scoped thread so the
    // caller's bytes are borrowed, never copied (the worker object can
    // be hundreds of MiB).
    let pad = (BLOCK - bytes.len() % BLOCK) % BLOCK;
    let ((status, writer_result), stderr) = std::thread::scope(|scope| {
        let stderr_drain = scope.spawn(move || {
            let mut kept: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match stderr_pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if kept.len() < STDERR_LIMIT {
                            let room = STDERR_LIMIT - kept.len();
                            kept.extend_from_slice(&chunk[..count.min(room)]);
                        }
                    }
                }
            }
            kept
        });
        let writer = scope.spawn(move || {
            // `stdin` drops when this thread returns, closing the pipe:
            // the daemon's extractor sees end-of-archive and finishes.
            // A broken pipe just means the daemon failed first — the
            // exit status classifies below.
            stdin
                .write_all(&head)
                .and_then(|()| stdin.write_all(bytes))
                .and_then(|()| stdin.write_all(&vec![0u8; pad]))
                .and_then(|()| stdin.write_all(&[0u8; 2 * BLOCK]))
        });
        let deadline = Instant::now() + WRITE_DEADLINE;
        let status = loop {
            if token.is_cancelled() || Instant::now() >= deadline {
                let cancelled = token.is_cancelled();
                let _ = process.terminate();
                break if cancelled {
                    Err(ContainerError::Cancelled)
                } else {
                    Err(ContainerError::Io {
                        detail: format!("docker cp timed out after {}s", WRITE_DEADLINE.as_secs()),
                    })
                };
            }
            match process.has_exited() {
                Ok(true) => {
                    break process.wait().map_err(|error| ContainerError::Io {
                        detail: format!("docker cp wait: {}", error.message),
                    });
                }
                Ok(false) => std::thread::park_timeout(POLL),
                Err(error) => {
                    let _ = process.terminate();
                    break Err(ContainerError::Io {
                        detail: format!("docker cp observe: {}", error.message),
                    });
                }
            }
        };
        let writer_result = writer
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("archive writer panicked")));
        let stderr = stderr_drain.join().unwrap_or_default();
        ((status, writer_result), stderr)
    });

    let exit = status?;
    if !exit.success() {
        let tail = stderr_tail(&stderr);
        if tail.contains("Could not find the file") {
            return Err(ContainerError::NoSuchPath {
                id: id.id().to_string(),
                path: parent.to_string(),
            });
        }
        if tail.contains("is not running") {
            return Err(ContainerError::NotRunning {
                id: id.id().to_string(),
            });
        }
        return Err(ContainerError::Io {
            detail: format!("docker cp into {parent} failed ({exit:?}): {tail}"),
        });
    }
    // A successful extraction with a broken writer is a torn transfer:
    // surface it rather than trust the exit code alone.
    writer_result.map_err(|error| ContainerError::Io {
        detail: format!("docker cp archive write: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tar::{parse, TarKind};

    #[test]
    fn the_header_round_trips_through_the_strict_reader() {
        let head = header("obj", 5, 0o500, 1000, 1001).unwrap();
        let mut bytes = head.to_vec();
        bytes.extend_from_slice(b"hello");
        bytes.extend(std::iter::repeat_n(0, BLOCK - 5));
        bytes.extend(std::iter::repeat_n(0, 2 * BLOCK));
        let entries = parse(&bytes, true).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, TarKind::File);
        assert_eq!(&bytes[entries[0].data.clone()], b"hello");
    }

    #[test]
    fn the_header_carries_principal_ownership_and_mode() {
        // The same offsets the stream parser reads: the deployment
        // provider's principal, never root.
        let head = header("stage", 0, 0o600, 1000, 1002).unwrap();
        let octal = |range: std::ops::Range<usize>| {
            let field = &head[range];
            let end = field
                .iter()
                .position(|&b| b == 0 || b == b' ')
                .unwrap_or(field.len());
            u64::from_str_radix(std::str::from_utf8(&field[..end]).unwrap(), 8).unwrap()
        };
        assert_eq!(octal(100..108), 0o600);
        assert_eq!(octal(108..116), 1000);
        assert_eq!(octal(116..124), 1002);
    }

    #[test]
    fn oversized_or_slash_carrying_names_are_refused() {
        assert!(header(&"n".repeat(101), 0, 0o600, 0, 0).is_err());
        assert!(header("a/b", 0, 0o600, 0, 0).is_err());
        assert!(header("", 0, 0o600, 0, 0).is_err());
    }
}
