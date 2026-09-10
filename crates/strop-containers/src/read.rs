//! Read-only filesystem access: directory listings and bounded file
//! reads, both over the engine's own file facility — `docker cp
//! <id>:<path> -` streams a tar archive to stdout, so no `ls`/`stat`/`sh`
//! has to exist inside the container (distroless included) and nothing
//! user-controlled passes through a shell anywhere.
//!
//! Every read first re-inspects the container ([`refresh`]): the cheap
//! incarnation check that turns "restarted between inspect and read"
//! into [`ContainerError::StaleIdentity`] instead of wrong bytes.

use crate::engine::{capture, refresh, stderr_tail, EngineRef, LIST_LIMIT, READ_DEADLINE};
use crate::identity::ContainerRef;
use crate::tar::{self, TarEntry, TarKind};
use crate::ContainerError;
use strop_core::worker::CancelToken;

/// Slack above `max` in a read's capture bound: the tar header, padding
/// and any longname/pax records around the content itself.
const HEADER_SLACK: u64 = 64 * 1024;

/// What one directory entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DirEntryKind {
    File,
    Dir,
    Symlink,
    /// Hard links, devices, fifos — kinds this backend does not
    /// distinguish further.
    Other,
}

/// One direct child of a listed directory.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub kind: DirEntryKind,
    /// The file's size in bytes; `None` for non-files.
    pub size: Option<u64>,
}

/// The direct children of `path` inside the container, parsed strictly
/// from the tar archive the engine streams for the directory.
///
/// A listing whose archive exceeds [`LIST_LIMIT`] is refused as
/// [`ContainerError::OutputTooLarge`] — a partial listing is never
/// presented as complete. `path` naming a non-directory is a capability
/// refusal, not an empty listing. A symlinked `path` is resolved one hop
/// (`docker cp` reports the link itself); chains are refused.
pub fn list_dir(
    engine: &EngineRef,
    id: &ContainerRef,
    path: &str,
    token: &CancelToken,
) -> Result<Vec<DirEntry>, ContainerError> {
    refresh(engine, id, token)?;
    let mut archive = fetch_archive(engine, id, path, LIST_LIMIT, true, token)?;
    let mut spelling = path.to_string();
    if let Some(resolved) = symlink_target(path, &archive.entries)? {
        archive = fetch_archive(engine, id, &resolved, LIST_LIMIT, true, token)?;
        if symlink_target(&resolved, &archive.entries)?.is_some() {
            return Err(ContainerError::CapabilityRefused {
                what: format!("list_dir: {path} is a symlink chain"),
            });
        }
        spelling = resolved;
    }
    children(&archive.entries, &spelling)
}

/// The first `max` bytes of the file at `path` inside the container
/// (`head -c` semantics: a larger file yields exactly `max` bytes, never
/// an error for size alone).
///
/// The capture bound is `max` plus tar framing slack; content past `max`
/// is dropped by the bounded capture, not buffered whole. `path` naming a
/// directory (or any non-regular entry) is a capability refusal. A
/// symlinked `path` is resolved one hop; chains are refused.
pub fn read_file(
    engine: &EngineRef,
    id: &ContainerRef,
    path: &str,
    max: u64,
    token: &CancelToken,
) -> Result<Vec<u8>, ContainerError> {
    refresh(engine, id, token)?;
    let limit = max.saturating_add(HEADER_SLACK);
    let mut archive = fetch_archive(engine, id, path, limit, false, token)?;
    let mut spelling = path.to_string();
    if let Some(resolved) = symlink_target(path, &archive.entries)? {
        archive = fetch_archive(engine, id, &resolved, limit, false, token)?;
        if symlink_target(&resolved, &archive.entries)?.is_some() {
            return Err(ContainerError::CapabilityRefused {
                what: format!("read_file: {path} is a symlink chain"),
            });
        }
        spelling = resolved;
    }
    let first = archive
        .entries
        .first()
        .ok_or_else(|| ContainerError::Protocol {
            detail: format!("read archive of {spelling} holds no entry"),
        })?;
    if first.kind != TarKind::File {
        return Err(ContainerError::CapabilityRefused {
            what: format!("read_file: {spelling} is a {}", describe(first.kind)),
        });
    }
    let content = &archive.bytes[first.data.clone()];
    let keep = (max as usize).min(content.len());
    Ok(content[..keep].to_vec())
}

/// One captured `docker cp` archive: the retained bytes and the parsed
/// entry headers (whose content offsets index into `bytes`).
struct Archive {
    bytes: Vec<u8>,
    entries: Vec<TarEntry>,
}

/// Capture and strictly parse the archive for `path`. With
/// `refuse_partial`, a listing that overflowed the output bound is
/// [`ContainerError::OutputTooLarge`]; without it (reads), truncation is
/// the caller's explicit `max` semantics.
fn fetch_archive(
    engine: &EngineRef,
    id: &ContainerRef,
    path: &str,
    limit: u64,
    refuse_partial: bool,
    token: &CancelToken,
) -> Result<Archive, ContainerError> {
    let _ = engine;
    let output = capture(&["cp", &target(id, path), "-"], limit, READ_DEADLINE, token)?;
    if output.code != Some(0) {
        return Err(classify_cp(id, path, &output.stderr));
    }
    if refuse_partial && output.stdout_dropped > 0 {
        return Err(ContainerError::OutputTooLarge {
            what: format!("listing of {path}"),
        });
    }
    let entries = tar::parse(&output.stdout, output.stdout_dropped == 0).map_err(|detail| {
        ContainerError::Protocol {
            detail: format!("archive of {path}: {detail}"),
        }
    })?;
    Ok(Archive {
        bytes: output.stdout,
        entries,
    })
}

/// When the archive is a lone symlink (`docker cp` does not resolve a
/// symlinked source path), the resolved absolute target path.
fn symlink_target(path: &str, entries: &[TarEntry]) -> Result<Option<String>, ContainerError> {
    let Some(first) = entries.first() else {
        return Ok(None);
    };
    if first.kind != TarKind::Symlink {
        return Ok(None);
    }
    let target = first
        .link_target
        .as_deref()
        .filter(|target| !target.is_empty())
        .ok_or_else(|| ContainerError::Protocol {
            detail: format!("archive of {path} carries a symlink without a target"),
        })?;
    Ok(Some(resolve_link(path, target)))
}

/// Resolve a symlink target against the link's location, the POSIX way:
/// absolute targets replace the path, relative ones join the link's
/// parent directory; `.` and `..` are normalized lexically. The result
/// is always absolute.
fn resolve_link(link_path: &str, target: &str) -> String {
    let mut resolved: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        components(link_path.rsplit_once('/').map_or("", |(parent, _)| parent))
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                resolved.pop();
            }
            part => resolved.push(part),
        }
    }
    format!("/{}", resolved.join("/"))
}

/// `docker cp`'s source spelling: the canonical id pins the container,
/// the path rides in the same argv element — no shell ever parses it.
fn target(id: &ContainerRef, path: &str) -> String {
    format!("{}:{path}", id.id())
}

/// Classify a failed `docker cp`: a missing path is typed, a container
/// that stopped mid-read is typed, anything else is bounded diagnostics.
fn classify_cp(id: &ContainerRef, path: &str, stderr: &[u8]) -> ContainerError {
    let tail = stderr_tail(stderr);
    if tail.contains("Could not find the file") {
        ContainerError::NoSuchPath {
            id: id.id().to_string(),
            path: path.to_string(),
        }
    } else if tail.contains("is not running") {
        ContainerError::NotRunning {
            id: id.id().to_string(),
        }
    } else {
        ContainerError::Io {
            detail: format!("docker cp failed: {tail}"),
        }
    }
}

/// The direct children among a directory archive's entries.
///
/// The archive's first entry is the listed directory itself (a file
/// answer means `path` was not a directory); every other entry must sit
/// under it, and only exactly-one-level-deeper entries are children.
/// Anything escaping that shape is a protocol violation, not a guess.
fn children(entries: &[TarEntry], path: &str) -> Result<Vec<DirEntry>, ContainerError> {
    let Some(root_entry) = entries.first() else {
        return Ok(Vec::new());
    };
    if root_entry.kind != TarKind::Dir {
        return Err(ContainerError::CapabilityRefused {
            what: format!("list_dir: {path} is a {}", describe(root_entry.kind)),
        });
    }
    let root = components(&root_entry.name);
    let mut children = Vec::new();
    for entry in &entries[1..] {
        let parts = components(&entry.name);
        if parts.len() <= root.len() || parts[..root.len()] != root[..] {
            return Err(ContainerError::Protocol {
                detail: format!(
                    "listing archive entry {:?} escapes the listed directory",
                    entry.name
                ),
            });
        }
        if parts.len() == root.len() + 1 {
            children.push(DirEntry {
                name: parts[root.len()].to_string(),
                kind: kind(entry.kind),
                size: (entry.kind == TarKind::File).then_some(entry.size),
            });
        }
    }
    Ok(children)
}

/// Path components of an archive name: `/`, `.` and empty segments carry
/// no meaning in `docker cp` archives.
fn components(name: &str) -> Vec<&str> {
    name.split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect()
}

fn kind(tar_kind: TarKind) -> DirEntryKind {
    match tar_kind {
        TarKind::File => DirEntryKind::File,
        TarKind::Dir => DirEntryKind::Dir,
        TarKind::Symlink => DirEntryKind::Symlink,
        TarKind::Other => DirEntryKind::Other,
    }
}

fn describe(tar_kind: TarKind) -> &'static str {
    match tar_kind {
        TarKind::File => "file",
        TarKind::Dir => "directory",
        TarKind::Symlink => "symlink",
        TarKind::Other => "special entry",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tar_entry(name: &str, kind: TarKind) -> TarEntry {
        TarEntry {
            name: name.to_string(),
            kind,
            size: if kind == TarKind::File { 42 } else { 0 },
            data: 0..0,
            link_target: None,
        }
    }

    #[test]
    fn symlink_targets_resolve_posix_style() {
        assert_eq!(resolve_link("/data/link", "hello.txt"), "/data/hello.txt");
        assert_eq!(resolve_link("/data/link", "/etc/hostname"), "/etc/hostname");
        assert_eq!(
            resolve_link("/data/sub/link", "../hello.txt"),
            "/data/hello.txt"
        );
        assert_eq!(resolve_link("/link", "a/./b"), "/a/b");
        assert_eq!(resolve_link("/a/b/link", "../../x"), "/x", "clamps at root");
    }

    #[test]
    fn a_symlink_archive_offers_its_target_once() {
        let mut link = tar_entry("link", TarKind::Symlink);
        link.link_target = Some("hello.txt".into());
        assert_eq!(
            symlink_target("/data/link", &[link]).unwrap(),
            Some("/data/hello.txt".to_string())
        );
        let file = tar_entry("hello.txt", TarKind::File);
        assert_eq!(symlink_target("/data/hello.txt", &[file]).unwrap(), None);
        let mut empty = tar_entry("link", TarKind::Symlink);
        empty.link_target = Some(String::new());
        assert!(matches!(
            symlink_target("/data/link", &[empty]),
            Err(ContainerError::Protocol { .. })
        ));
    }

    #[test]
    fn direct_children_only_with_sizes_on_files() {
        let entries = vec![
            tar_entry("data", TarKind::Dir),
            tar_entry("data/hello.txt", TarKind::File),
            tar_entry("data/sub", TarKind::Dir),
            tar_entry("data/sub/inner.bin", TarKind::File),
            tar_entry("data/link", TarKind::Symlink),
            tar_entry("data/fifo", TarKind::Other),
        ];
        let children = children(&entries, "/data").unwrap();
        let by_name = |name: &str| children.iter().find(|e| e.name == name);
        assert_eq!(children.len(), 4, "grandchildren are not children");
        assert_eq!(
            by_name("hello.txt").map(|e| (e.kind, e.size)),
            Some((DirEntryKind::File, Some(42)))
        );
        assert_eq!(
            by_name("sub").map(|e| (e.kind, e.size)),
            Some((DirEntryKind::Dir, None))
        );
        assert_eq!(by_name("link").map(|e| e.kind), Some(DirEntryKind::Symlink));
        assert_eq!(by_name("fifo").map(|e| e.kind), Some(DirEntryKind::Other));
    }

    #[test]
    fn root_components_are_normalized() {
        for root_name in [".", "/", "./"] {
            let entries = vec![
                tar_entry(root_name, TarKind::Dir),
                tar_entry("etc", TarKind::Dir),
                tar_entry("etc/hostname", TarKind::File),
            ];
            let children = children(&entries, "/").unwrap();
            assert_eq!(children.len(), 1, "root {root_name:?}");
            assert_eq!(children[0].name, "etc");
        }
    }

    #[test]
    fn a_file_answer_is_a_refusal_and_escapes_are_protocol_errors() {
        let file = vec![tar_entry("data/hello.txt", TarKind::File)];
        assert!(matches!(
            children(&file, "/data/hello.txt"),
            Err(ContainerError::CapabilityRefused { .. })
        ));
        let escape = vec![
            tar_entry("data", TarKind::Dir),
            tar_entry("other/evil", TarKind::File),
        ];
        assert!(matches!(
            children(&escape, "/data"),
            Err(ContainerError::Protocol { .. })
        ));
        let empty = vec![tar_entry("data", TarKind::Dir)];
        assert_eq!(children(&empty, "/data").unwrap(), vec![]);
    }
}
