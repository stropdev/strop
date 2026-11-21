//! Local, data-only `.strop/workspace.toml` v1. A spelling is not authority:
//! bind against the pinned root again before using a declared directory as cwd.
use rustix::fs::{openat2, Mode, OFlags, ResolveFlags, CWD};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MANIFEST: &str = ".strop/workspace.toml";
const MAX_BYTES: usize = 64 * 1024;
const MAX_PROJECTS: usize = 256;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    #[serde(default)]
    project: Vec<Record>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    path: String,
    languages: Option<Vec<String>>,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

// Path::components normalizes internal `.` and empty components. Check the
// original bytes so two unsupported spellings can't become the same member.
fn normal_relative(path: &Path) -> bool {
    path.as_os_str()
        .as_encoded_bytes()
        .split(|&b| b == b'/')
        .all(|part| !part.is_empty() && part != b"." && part != b".." && !part.contains(&0))
}

fn literal(path: &str) -> bool {
    let scheme = path.split_once(':').is_some_and(|(prefix, _)| {
        prefix.starts_with(|c: char| c.is_ascii_alphabetic())
            && prefix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
    });
    normal_relative(Path::new(path))
        && !scheme
        && !path.starts_with('~')
        && !path.contains(['*', '?', '[', ']', '{', '}', '$', '\\'])
        && !path.chars().any(char::is_control)
}

fn parse(text: &str) -> io::Result<Vec<Record>> {
    let manifest: Manifest = toml::from_str(text).map_err(|error: toml::de::Error| {
        let line = error.span().map_or(1, |span| {
            1 + text.as_bytes()[..span.start].iter().filter(|&&b| b == b'\n').count()
        });
        // Never echo arbitrary command text or potentially sensitive values.
        io::Error::new(io::ErrorKind::InvalidData, format!("invalid v1 TOML near line {line}; use only version, [[project]], path and languages"))
    })?;
    if manifest.version != 1 {
        return Err(invalid("unsupported version; expected version = 1"));
    }
    if manifest.project.len() > MAX_PROJECTS {
        return Err(invalid("at most 256 project records are allowed"));
    }
    let mut seen = HashSet::with_capacity(manifest.project.len());
    for record in &manifest.project {
        if !literal(&record.path) {
            return Err(invalid(
                "project path must be a literal descendant directory",
            ));
        }
        if !seen.insert(record.path.as_str()) {
            return Err(invalid(
                "duplicate project path; declare each directory once",
            ));
        }
        if record
            .languages
            .iter()
            .flatten()
            .any(|id| id.is_empty() || id.chars().any(char::is_control))
        {
            return Err(invalid(
                "languages must contain nonempty language IDs without control characters",
            ));
        }
    }
    Ok(manifest.project)
}

fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC
}

fn open_root(root: &Path) -> io::Result<File> {
    Ok(openat2(
        CWD,
        root,
        dir_flags(),
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS,
    )?
    .into())
}

fn beneath(root: &File, relative: &Path, flags: OFlags) -> io::Result<File> {
    // Resolve the *whole* relative path in one operation. Checking components
    // with lstat then opening by name would allow a link swap in an ancestor.
    Ok(openat2(
        root,
        relative,
        flags,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
    )?
    .into())
}

/// Open a native literal descendant as a directory without following links.
/// Keep the returned descriptor through the operation; never reopen by name.
/// This supplies containment only, not trust or project eligibility.
pub fn open_directory(root: &Path, relative: &Path) -> io::Result<File> {
    if !normal_relative(relative) {
        return Err(invalid("expected a literal relative descendant"));
    }
    beneath(&open_root(root)?, relative, dir_flags())
}

fn open_manifest(root: &File) -> io::Result<File> {
    // O_PATH never opens a FIFO, terminal or device for I/O. Upgrade only
    // the pinned, known-regular inode, not the original attacker-writable name.
    let pin = beneath(root, Path::new(MANIFEST), OFlags::PATH | OFlags::CLOEXEC)?;
    if !pin.metadata()?.is_file() {
        return Err(invalid(
            "expected a regular file, not a link, directory or device",
        ));
    }
    File::open(format!("/proc/self/fd/{}", pin.as_raw_fd())).map_err(|error| {
        // Only ENOENT from the root-relative lookup means an absent manifest.
        io::Error::other(format!(
            "could not read pinned manifest from /proc/self/fd: {error}"
        ))
    })
}

fn same_file(a: &File, b: &File) -> io::Result<bool> {
    let (a, b) = (a.metadata()?, b.metadata()?);
    Ok((a.dev(), a.ino()) == (b.dev(), b.ino()))
}

struct Source {
    root: File,
    manifest: File,
    bytes: Box<[u8]>,
}

impl Source {
    fn check(&self, root: &File) -> io::Result<()> {
        if !same_file(root, &self.root)? {
            return Err(invalid("workspace root was replaced"));
        }
        let mut file = open_manifest(root)?;
        if !same_file(&file, &self.manifest)? || file.metadata()?.len() != self.bytes.len() as u64 {
            return Err(invalid("workspace declaration changed"));
        }
        // One bounded source witness shared by all members. Neither an old
        // inode nor an equal length/mtime is proof of the current file bytes.
        let mut buffer = [0; 4096];
        for chunk in self.bytes.chunks(buffer.len()) {
            let got = &mut buffer[..chunk.len()];
            file.read_exact(got)?;
            if got != chunk {
                return Err(invalid("workspace declaration changed"));
            }
        }
        if file.read(&mut buffer[..1])? != 0 {
            return Err(invalid("workspace declaration changed"));
        }
        Ok(())
    }
}

/// Native, deliberately non-serializable evidence captured on a worker. Kept
/// open so unlink/recreate cannot recycle an inode under an outstanding ticket.
#[derive(Clone)]
pub struct DirectoryWitness {
    source: Arc<Source>,
    directory: Arc<File>,
}

impl std::fmt::Debug for DirectoryWitness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectoryWitness").finish_non_exhaustive()
    }
}

impl PartialEq for DirectoryWitness {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.directory, &other.directory)
    }
}
impl Eq for DirectoryWitness {}

impl DirectoryWitness {
    /// Match a policy walk's freshly visited name (possibly in a narrowed
    /// folder). Advisory catalog evidence only; not a launch-time binding.
    pub fn observes(&self, path: &Path) -> bool {
        open_root(path)
            .and_then(|current| same_file(&current, &self.directory))
            .unwrap_or(false)
    }

    /// Observe the directory again at its workspace-relative name. The result
    /// is the *actual* descriptor to use, not permission to chdir(path) later.
    pub fn reopen(&self, root: &Path, relative: &Path) -> io::Result<File> {
        if !normal_relative(relative) {
            return Err(invalid("expected a literal relative descendant"));
        }
        let root = open_root(root)?;
        self.source.check(&root)?;
        let dir = beneath(&root, relative, dir_flags())?;
        if !same_file(&dir, &self.directory)? {
            return Err(invalid("declared directory was replaced"));
        }
        Ok(dir)
    }
}

/// A physically available directory, not evidence that the selection policy
/// visits it or that any of its hinted languages has an authorized producer.
#[derive(Debug)]
pub struct ProjectDeclaration {
    pub path: PathBuf,
    pub languages: Option<Vec<String>>,
    pub witness: DirectoryWitness,
}

/// Read only the pinned local root's manifest. Absence is normal; malformed
/// input rejects the entire file before opening a single declared directory.
pub fn read(root: &Path) -> io::Result<Vec<ProjectDeclaration>> {
    read_at(root).map_err(|error| io::Error::new(error.kind(), format!("{MANIFEST}: {error}")))
}

fn read_at(path: &Path) -> io::Result<Vec<ProjectDeclaration>> {
    let root = open_root(path)?;
    let mut manifest = match open_manifest(&root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        result => result?,
    };
    let len = manifest.metadata()?.len();
    if len > MAX_BYTES as u64 {
        return Err(invalid("file exceeds 64 KiB"));
    }
    let mut bytes = Vec::with_capacity(len as usize + 1);
    (&mut manifest)
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid("file exceeds 64 KiB"));
    }
    let records = parse(std::str::from_utf8(&bytes).map_err(|_| invalid("expected UTF-8 TOML"))?)?;
    let source = Arc::new(Source {
        root,
        manifest,
        bytes: bytes.into_boxed_slice(),
    });
    let mut bound = Vec::with_capacity(records.len());
    for record in records {
        let path = PathBuf::from(record.path);
        let directory = match beneath(&source.root, &path, dir_flags()) {
            Ok(dir) => dir,
            // These individual members are unavailable, not a rejection of
            // their syntax and not permission to use a less-specific cwd.
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(
                        libc::ENOENT
                            | libc::ENOTDIR
                            | libc::ELOOP
                            | libc::EACCES
                            | libc::EXDEV
                            | libc::EAGAIN
                            | libc::ENAMETOOLONG
                    )
                ) =>
            {
                continue
            }
            Err(error) => return Err(error),
        };
        bound.push(ProjectDeclaration {
            path,
            languages: record.languages,
            witness: DirectoryWitness {
                source: source.clone(),
                directory: Arc::new(directory),
            },
        });
    }
    match source.check(&open_root(path)?) {
        Ok(()) => Ok(bound),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
