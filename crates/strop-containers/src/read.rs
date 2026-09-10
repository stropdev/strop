//! Read-only filesystem access: directory listings and bounded file
//! reads, both over the engine's own file facility — `docker cp
//! <id>:<path> -` streams a tar archive to stdout, so no `ls`/`stat`/`sh`
//! has to exist inside the container (distroless included) and nothing
//! user-controlled passes through a shell anywhere.
//!
//! Every read first re-inspects the container ([`refresh`]): the cheap
//! incarnation check that turns "restarted between inspect and read"
//! into [`ContainerError::StaleIdentity`] instead of wrong bytes.

use crate::engine::{capture, refresh, stderr_tail, stream, EngineRef, LIST_LIMIT, READ_DEADLINE};
use crate::identity::ContainerRef;
use crate::tar::{self, StreamEntry, TarEntry, TarKind};
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
/// The archive is consumed incrementally as it arrives: only
/// direct-child metadata (names, kinds, sizes) is retained, so a
/// direct-child directory's subtree — however large — streams through
/// without being held. The transfer cost is still the engine's stream:
/// the whole archive flows through the pipe, deadline-bounded; it is
/// retention, not transfer, that this listing bounds. A listing whose
/// retained metadata exceeds [`LIST_LIMIT`] is refused as
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
    match list_once(engine, id, path, token)? {
        Listing::Children(children) => Ok(children),
        Listing::Symlink(resolved) => match list_once(engine, id, &resolved, token)? {
            Listing::Children(children) => Ok(children),
            Listing::Symlink(_) => Err(ContainerError::CapabilityRefused {
                what: format!("list_dir: {path} is a symlink chain"),
            }),
        },
    }
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
    let mut archive = fetch_archive(engine, id, path, limit, token)?;
    let mut spelling = path.to_string();
    if let Some(resolved) = symlink_target(path, &archive.entries)? {
        archive = fetch_archive(engine, id, &resolved, limit, token)?;
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

/// Capture and strictly parse the archive for `path`, bounded by
/// `limit` — the read path's explicit `max` semantics: truncation past
/// the bound is expected and tolerated by the parse (`complete` false).
/// Listings do not use this; they stream ([`list_once`]).
fn fetch_archive(
    engine: &EngineRef,
    id: &ContainerRef,
    path: &str,
    limit: u64,
    token: &CancelToken,
) -> Result<Archive, ContainerError> {
    let _ = engine;
    let output = capture(&["cp", &target(id, path), "-"], limit, READ_DEADLINE, token)?;
    if output.code != Some(0) {
        return Err(classify_cp(id, path, &output.stderr));
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

/// One streamed listing pass's answer.
enum Listing {
    Children(Vec<DirEntry>),
    /// `path` itself is a symlink: the resolved absolute target. The
    /// caller re-lists once; a second symlink answer is a chain refusal.
    Symlink(String),
}

/// Stream and strictly parse the archive for `path`, retaining only
/// direct-child metadata. A consumer-side refusal (capability,
/// protocol, retention overflow) aborts the transfer rather than
/// draining past a known answer.
fn list_once(
    engine: &EngineRef,
    id: &ContainerRef,
    path: &str,
    token: &CancelToken,
) -> Result<Listing, ContainerError> {
    let _ = engine;
    let mut listing = ListingConsumer::new(path);
    let streamed = stream(
        &["cp", &target(id, path), "-"],
        READ_DEADLINE,
        token,
        |chunk| listing.feed(chunk),
    )?;
    if streamed.code != Some(0) {
        return Err(classify_cp(id, path, &streamed.stderr));
    }
    listing.finish()
}

/// The streaming listing fold: entry headers arrive in the archive's
/// pre-order, so a direct-child directory's subtree follows its header
/// and is consumed without retention. `retained` counts the direct
/// children's metadata against [`LIST_LIMIT`].
struct ListingConsumer<'a> {
    path: &'a str,
    parser: tar::StreamParser,
    /// Components of the first entry's name — the listed path itself.
    root: Option<Vec<String>>,
    /// Set when the listed path itself is a symlink: the resolved target.
    symlink: Option<String>,
    children: Vec<DirEntry>,
    retained: u64,
}

impl<'a> ListingConsumer<'a> {
    fn new(path: &'a str) -> Self {
        Self {
            path,
            parser: tar::StreamParser::default(),
            root: None,
            symlink: None,
            children: Vec::new(),
            retained: 0,
        }
    }

    /// Fold one stream chunk. The first entry-level error aborts the
    /// stream: the transfer is killed, not drained past a known refusal.
    fn feed(&mut self, chunk: &[u8]) -> Result<(), ContainerError> {
        let mut parser = std::mem::take(&mut self.parser);
        let mut failed = None;
        let parsed = parser.feed(chunk, &mut |entry| {
            if failed.is_none() {
                failed = self.on_entry(entry).err();
            }
        });
        self.parser = parser;
        if let Some(error) = failed {
            return Err(error);
        }
        parsed.map_err(|detail| ContainerError::Protocol {
            detail: format!("archive of {}: {detail}", self.path),
        })
    }

    /// One entry header. The archive's first entry is the listed path
    /// itself (a file answer means `path` was not a directory); every
    /// other entry must sit under it, and only exactly-one-level-deeper
    /// entries are children. Anything escaping that shape is a protocol
    /// violation, not a guess.
    fn on_entry(&mut self, entry: StreamEntry) -> Result<(), ContainerError> {
        if self.symlink.is_some() {
            return Ok(()); // a symlink answer stands alone; extras are drained
        }
        let Some(root) = &self.root else {
            return self.on_first(entry);
        };
        let parts = components(&entry.name);
        let escapes = parts.len() <= root.len()
            || !parts
                .iter()
                .zip(root.iter())
                .all(|(part, segment)| *part == segment.as_str());
        if escapes {
            return Err(ContainerError::Protocol {
                detail: format!(
                    "listing archive entry {:?} escapes the listed directory",
                    entry.name
                ),
            });
        }
        if parts.len() == root.len() + 1 {
            let cost = entry.name.len() as u64 + size_of::<DirEntry>() as u64;
            if self.retained.saturating_add(cost) > LIST_LIMIT {
                return Err(ContainerError::OutputTooLarge {
                    what: format!("listing of {}", self.path),
                });
            }
            self.retained += cost;
            self.children.push(DirEntry {
                name: parts[root.len()].to_string(),
                kind: kind(entry.kind),
                size: (entry.kind == TarKind::File).then_some(entry.size),
            });
        }
        Ok(())
    }

    /// The archive's first entry: a directory roots the listing, a
    /// symlink resolves one hop, anything else is a capability refusal.
    fn on_first(&mut self, entry: StreamEntry) -> Result<(), ContainerError> {
        match entry.kind {
            TarKind::Dir => {
                self.root = Some(
                    components(&entry.name)
                        .iter()
                        .map(|part| part.to_string())
                        .collect(),
                );
                Ok(())
            }
            TarKind::Symlink => {
                let target = entry
                    .link_target
                    .filter(|target| !target.is_empty())
                    .ok_or_else(|| ContainerError::Protocol {
                        detail: format!(
                            "archive of {} carries a symlink without a target",
                            self.path
                        ),
                    })?;
                self.symlink = Some(resolve_link(self.path, &target));
                Ok(())
            }
            other => Err(ContainerError::CapabilityRefused {
                what: format!("list_dir: {} is a {}", self.path, describe(other)),
            }),
        }
    }

    /// The stream ended and the engine reported success (the caller
    /// checks the exit status first): validate the archive tail and
    /// yield the listing.
    fn finish(self) -> Result<Listing, ContainerError> {
        self.parser
            .finish()
            .map_err(|detail| ContainerError::Protocol {
                detail: format!("archive of {}: {detail}", self.path),
            })?;
        if let Some(target) = self.symlink {
            return Ok(Listing::Symlink(target));
        }
        Ok(Listing::Children(self.children))
    }
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

    fn tar_entry(kind: TarKind) -> TarEntry {
        TarEntry {
            kind,
            data: 0..0,
            link_target: None,
        }
    }

    /// One ustar header + content blocks for a single archive entry.
    fn tar_part(name: &str, typeflag: u8, content: &[u8]) -> Vec<u8> {
        let mut header = [0u8; 512];
        let name_bytes = name.as_bytes();
        assert!(name_bytes.len() <= 100);
        header[..name_bytes.len()].copy_from_slice(name_bytes);
        let size = format!("{:011o}", content.len());
        header[124..124 + size.len()].copy_from_slice(size.as_bytes());
        header[257..262].copy_from_slice(b"ustar");
        header[156] = typeflag;
        let mut out = header.to_vec();
        out.extend_from_slice(content);
        out.resize(out.len() + (512 - content.len() % 512) % 512, 0);
        out
    }

    fn tar_symlink(name: &str, target: &str) -> Vec<u8> {
        let mut part = tar_part(name, b'2', b"");
        part[157..157 + target.len()].copy_from_slice(target.as_bytes());
        part
    }

    fn archive(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut out = parts.concat();
        out.extend_from_slice(&[0u8; 512]); // end marker
        out
    }

    /// Drive a listing over archive bytes in mid-sized chunks, the way
    /// the engine's pipe delivers them.
    fn list(path: &str, bytes: &[u8]) -> Result<Listing, ContainerError> {
        let mut consumer = ListingConsumer::new(path);
        for chunk in bytes.chunks(1000) {
            consumer.feed(chunk)?;
        }
        consumer.finish()
    }

    fn children_of(path: &str, bytes: &[u8]) -> Result<Vec<DirEntry>, ContainerError> {
        match list(path, bytes)? {
            Listing::Children(children) => Ok(children),
            Listing::Symlink(target) => panic!("expected children, got symlink to {target}"),
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
        let mut link = tar_entry(TarKind::Symlink);
        link.link_target = Some("hello.txt".into());
        assert_eq!(
            symlink_target("/data/link", &[link]).unwrap(),
            Some("/data/hello.txt".to_string())
        );
        let file = tar_entry(TarKind::File);
        assert_eq!(symlink_target("/data/hello.txt", &[file]).unwrap(), None);
        let mut empty = tar_entry(TarKind::Symlink);
        empty.link_target = Some(String::new());
        assert!(matches!(
            symlink_target("/data/link", &[empty]),
            Err(ContainerError::Protocol { .. })
        ));
    }

    #[test]
    fn a_streamed_symlink_answer_resolves_one_hop() {
        let bytes = archive(&[tar_symlink("link", "hello.txt")]);
        assert!(matches!(
            list("/data/link", &bytes).unwrap(),
            Listing::Symlink(target) if target == "/data/hello.txt"
        ));
        let mut bare = tar_part("link", b'2', b"");
        bare[157..257].fill(0);
        let bytes = archive(&[bare]);
        assert!(
            matches!(
                list("/data/link", &bytes),
                Err(ContainerError::Protocol { .. })
            ),
            "a symlink without a target is a protocol violation"
        );
    }

    #[test]
    fn direct_children_only_with_sizes_on_files() {
        let bytes = archive(&[
            tar_part("data", b'5', b""),
            tar_part("data/hello.txt", b'0', b"hello strop\n"),
            tar_part("data/sub", b'5', b""),
            tar_part("data/sub/inner.bin", b'0', b"inner"),
            tar_symlink("data/link", "hello.txt"),
            tar_part("data/fifo", b'6', b""),
        ]);
        let children = children_of("/data", &bytes).unwrap();
        let by_name = |name: &str| children.iter().find(|e| e.name == name);
        assert_eq!(children.len(), 4, "grandchildren are not children");
        assert_eq!(
            by_name("hello.txt").map(|e| (e.kind, e.size)),
            Some((DirEntryKind::File, Some(12)))
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
            let bytes = archive(&[
                tar_part(root_name, b'5', b""),
                tar_part("etc", b'5', b""),
                tar_part("etc/hostname", b'0', b"container\n"),
            ]);
            let children = children_of("/", &bytes).unwrap();
            assert_eq!(children.len(), 1, "root {root_name:?}");
            assert_eq!(children[0].name, "etc");
        }
    }

    #[test]
    fn a_file_answer_is_a_refusal_and_escapes_are_protocol_errors() {
        let file = archive(&[tar_part("data/hello.txt", b'0', b"x")]);
        assert!(matches!(
            list("/data/hello.txt", &file),
            Err(ContainerError::CapabilityRefused { .. })
        ));
        let escape = archive(&[
            tar_part("data", b'5', b""),
            tar_part("other/evil", b'0', b"x"),
        ]);
        assert!(matches!(
            list("/data", &escape),
            Err(ContainerError::Protocol { .. })
        ));
        let empty = archive(&[tar_part("data", b'5', b"")]);
        assert_eq!(children_of("/data", &empty).unwrap(), vec![]);
    }

    #[test]
    fn subtree_bulk_streams_through_without_retention() {
        // A direct-child directory holding megabytes of nested files:
        // the old shape refused this past the archive bound; streaming
        // retains only the direct children's metadata.
        let bulk = vec![7u8; 4 * 1024 * 1024];
        let mut parts = vec![
            tar_part("data", b'5', b""),
            tar_part("data/deep", b'5', b""),
        ];
        for index in 0..8 {
            parts.push(tar_part(
                &format!("data/deep/nest/file{index}.bin"),
                b'0',
                &bulk,
            ));
        }
        parts.push(tar_part("data/shallow.txt", b'0', b"shallow"));
        let bytes = archive(&parts);
        let mut consumer = ListingConsumer::new("/data");
        for chunk in bytes.chunks(65536) {
            consumer.feed(chunk).unwrap();
        }
        let Listing::Children(children) = consumer.finish().unwrap() else {
            panic!("a directory answer is children");
        };
        let mut names: Vec<&str> = children.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["deep", "shallow.txt"]);
        assert!(
            consumer_retained_is_tiny(&children),
            "32 MB streamed; retention is the two direct children"
        );
    }

    fn consumer_retained_is_tiny(children: &[DirEntry]) -> bool {
        children
            .iter()
            .map(|e| e.name.len() as u64 + size_of::<DirEntry>() as u64)
            .sum::<u64>()
            < 1024
    }

    #[test]
    fn retention_overflow_is_output_too_large() {
        let mut consumer = ListingConsumer::new("/data");
        let entry = |name: String, kind| StreamEntry {
            name,
            kind,
            size: 0,
            link_target: None,
        };
        consumer
            .on_entry(entry("data".to_string(), TarKind::Dir))
            .unwrap();
        let wide = "n".repeat(2048);
        let mut overflowed = false;
        for index in 0.. {
            let name = format!("data/{wide}{index}");
            if consumer.on_entry(entry(name, TarKind::File)).is_err() {
                overflowed = true;
                break;
            }
        }
        assert!(overflowed, "enough direct children trip the bound");
        assert!(
            consumer.retained <= LIST_LIMIT,
            "retention stops at the bound, not past it"
        );
    }

    #[test]
    fn a_truncated_stream_is_a_protocol_error_at_finish() {
        let full = archive(&[
            tar_part("data", b'5', b""),
            tar_part("data/big.bin", b'0', &[9u8; 1000]),
        ]);
        let cut = &full[..512 + 512 + 400]; // mid-content of big.bin
        let mut consumer = ListingConsumer::new("/data");
        consumer.feed(cut).unwrap();
        assert!(matches!(
            consumer.finish(),
            Err(ContainerError::Protocol { .. })
        ));
        let cut = &full[..512 + 100]; // mid-header of big.bin
        let mut consumer = ListingConsumer::new("/data");
        consumer.feed(cut).unwrap();
        assert!(matches!(
            consumer.finish(),
            Err(ContainerError::Protocol { .. })
        ));
    }
}
