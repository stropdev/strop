//! Symlink-conscious metadata for one path inside the container
//! (0058 WK08): the worker-deployment provider's `lstat`.
//!
//! The facts come from the engine's own file facility — `docker cp
//! <id>:<path> -` streams a tar archive whose first entry IS the path
//! itself, with its kind, permission bits and numeric ownership in the
//! header. `docker cp` does not resolve a symlinked source path, so a
//! symlink is reported as a symlink: exactly `lstat` semantics, and the
//! deployment symlink policy is enforced on that fact. No `stat`/`ls`/
//! `sh` has to exist inside the container — a genuinely shellless image
//! answers the same way.
//!
//! Only the first header is consumed: the transfer is aborted as soon
//! as it lands, so stating a large file or a whole subtree costs one
//! header block, never the bulk. Every call first re-checks the
//! container's incarnation ([`refresh`]), exactly like the read path.

use crate::engine::{refresh, stream, EngineRef, READ_DEADLINE};
use crate::identity::ContainerRef;
use crate::read::{classify_cp, kind, DirEntryKind};
use crate::tar::StreamParser;
use crate::ContainerError;
use strop_core::worker::CancelToken;

/// `lstat`-class metadata for one in-container path: kind, permission
/// bits, numeric owner/group and length. Ownership is numeric because
/// the tar header's facts are — a name would come from the image's
/// `/etc/passwd`, which is not an identity the engine guarantees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathStat {
    pub kind: DirEntryKind,
    /// Unix permission bits (the header's mode field).
    pub mode: u32,
    pub uid: u64,
    pub gid: u64,
    pub len: u64,
}

/// Metadata for `path` inside the pinned container, never following a
/// final symlink; `Ok(None)` when the path does not exist.
///
/// The stream is cut the moment the first entry header lands (the
/// consumer's early-stop is caught here); anything else the engine
/// reports is classified exactly as the read path classifies it.
pub fn lstat(
    engine: &EngineRef,
    id: &ContainerRef,
    path: &str,
    token: &CancelToken,
) -> Result<Option<PathStat>, ContainerError> {
    refresh(engine, id, token)?;
    let target = format!("{}:{path}", id.id());
    let mut parser = StreamParser::default();
    let mut found: Option<PathStat> = None;
    let mut structural: Option<String> = None;
    let outcome = stream(
        engine,
        &["cp", &target, "-"],
        READ_DEADLINE,
        token,
        |chunk| {
            if found.is_some() || structural.is_some() {
                return Err(ContainerError::StreamStop);
            }
            match parser.feed(chunk, &mut |entry| {
                if found.is_none() {
                    found = Some(PathStat {
                        kind: kind(entry.kind),
                        mode: entry.mode,
                        uid: entry.uid,
                        gid: entry.gid,
                        len: entry.size,
                    });
                }
            }) {
                Ok(()) if found.is_some() => Err(ContainerError::StreamStop),
                Ok(()) => Ok(()),
                Err(detail) => {
                    structural = Some(detail);
                    Err(ContainerError::StreamStop)
                }
            }
        },
    );
    if let Some(detail) = structural {
        return Err(ContainerError::Protocol {
            detail: format!("archive of {path}: {detail}"),
        });
    }
    if let Some(stat) = found {
        return Ok(Some(stat));
    }
    // No entry arrived. `StreamStop` is only raised once an entry or a
    // structural error exists, so here the archive simply ended: an
    // empty one is the daemon misframing, a failed `cp` classifies as
    // on the read path (a missing path is typed there).
    match outcome {
        Ok(streamed) if streamed.code == Some(0) => Err(ContainerError::Protocol {
            detail: format!("archive of {path} holds no entry"),
        }),
        Ok(streamed) => match classify_cp(id, path, &streamed.stderr) {
            ContainerError::NoSuchPath { .. } => Ok(None),
            error => Err(error),
        },
        Err(ContainerError::StreamStop) => Err(ContainerError::Protocol {
            detail: format!("archive of {path} holds no entry"),
        }),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One ustar header + content for a fabricated archive: the same
    /// grammar the engine emits, so the consumer fold is exercised
    /// without a live daemon.
    fn header(name: &str, typeflag: u8, size: u64, mode: u32, uid: u64, gid: u64) -> Vec<u8> {
        let mut block = [0u8; 512];
        block[..name.len()].copy_from_slice(name.as_bytes());
        let mut put_octal = |range: std::ops::Range<usize>, value: u64| {
            let text = format!("{:0>width$o}\0", value, width = range.len() - 1);
            block[range].copy_from_slice(text.as_bytes());
        };
        put_octal(100..108, mode as u64);
        put_octal(108..116, uid);
        put_octal(116..124, gid);
        put_octal(124..136, size);
        put_octal(136..148, 0); // mtime
        block[156] = typeflag;
        block[257..262].copy_from_slice(b"ustar");
        block[148..156].copy_from_slice(b"        ");
        let sum: u64 = block.iter().map(|&b| b as u64).sum();
        let text = format!("{sum:06o}\0 ");
        block[148..156].copy_from_slice(text.as_bytes());
        block.to_vec()
    }

    fn archive(name: &str, typeflag: u8, content: &[u8], mode: u32, uid: u64, gid: u64) -> Vec<u8> {
        let mut bytes = header(name, typeflag, content.len() as u64, mode, uid, gid);
        bytes.extend_from_slice(content);
        let pad = (512 - content.len() % 512) % 512;
        bytes.extend(std::iter::repeat_n(0, pad));
        bytes.extend(std::iter::repeat_n(0, 1024)); // end marker
        bytes
    }

    /// The lstat consumer fold, mirroring the engine boundary: entries
    /// land in `found`, iteration stops the moment one does.
    fn first_entry(chunks: &[&[u8]]) -> Result<Option<PathStat>, String> {
        let mut parser = StreamParser::default();
        let mut found = None;
        for chunk in chunks {
            parser.feed(chunk, &mut |entry| {
                if found.is_none() {
                    found = Some(PathStat {
                        kind: kind(entry.kind),
                        mode: entry.mode,
                        uid: entry.uid,
                        gid: entry.gid,
                        len: entry.size,
                    });
                }
            })?;
            if found.is_some() {
                return Ok(found);
            }
        }
        Ok(found)
    }

    #[test]
    fn the_first_header_carries_lstat_facts() {
        let bytes = archive("worker-obj", b'0', b"payload-bytes", 0o500, 1000, 1001);
        let stat = first_entry(&[&bytes]).unwrap().unwrap();
        assert_eq!(stat.kind, DirEntryKind::File);
        assert_eq!(stat.mode, 0o500);
        assert_eq!(stat.uid, 1000);
        assert_eq!(stat.gid, 1001);
        assert_eq!(stat.len, 13);
    }

    #[test]
    fn a_symlink_is_reported_never_followed() {
        let mut bytes = header("link", b'2', 0, 0o777, 0, 0);
        bytes[157..167].copy_from_slice(b"target.txt");
        bytes[148..156].copy_from_slice(b"        ");
        let sum: u64 = bytes[..512].iter().map(|&b| b as u64).sum();
        let text = format!("{sum:06o}\0 ");
        bytes[148..156].copy_from_slice(text.as_bytes());
        bytes.extend(std::iter::repeat_n(0, 1024));
        let stat = first_entry(&[&bytes]).unwrap().unwrap();
        assert_eq!(stat.kind, DirEntryKind::Symlink);
    }

    #[test]
    fn the_abort_never_reads_past_the_first_header() {
        // A directory archive: the first entry is the dir itself; the
        // subtree behind it must never be consulted for the answer.
        let mut bytes = archive("data", b'5', b"", 0o700, 1000, 1000);
        bytes.extend(archive("data/huge.bin", b'0', &[7u8; 600], 0o600, 0, 0));
        let stat = first_entry(&[&bytes]).unwrap().unwrap();
        assert_eq!(stat.kind, DirEntryKind::Dir);
        assert_eq!(stat.mode, 0o700);
        assert_eq!(stat.uid, 1000);
    }

    #[test]
    fn split_delivery_across_the_header_still_parses() {
        let bytes = archive("obj", b'0', b"abc", 0o400, 33, 44);
        let mut stat = None;
        for split in [100usize, 511, 512, 513, 700] {
            stat = first_entry(&[&bytes[..split], &bytes[split..]]).unwrap();
            assert!(stat.is_some(), "split at {split}");
        }
        let found = stat.unwrap();
        assert_eq!(found.uid, 33);
        assert_eq!(found.gid, 44);
    }
}
