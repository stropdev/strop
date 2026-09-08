//! Local-vs-remote workspace identity (0036 RW8). A language server
//! runs somewhere and its URIs name that host's filesystem: the target
//! is part of every path identity, so a remote path can never alias a
//! local path with the same bytes. All URI math here is pure — no
//! local filesystem probe ever resolves a remote location.

use std::path::{Path, PathBuf};

use strop_remote::RemoteEndpoint;

/// The filesystem a path names: the local disk, or exactly one remote
/// endpoint. Part of identity — two targets never share a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FsTarget {
    Local,
    Remote(RemoteEndpoint),
}

impl Default for FsTarget {
    /// The local disk: records predating remote targets deserialize
    /// as local — never guessed remote.
    fn default() -> Self {
        Self::Local
    }
}

impl FsTarget {
    /// Modeline/trace form: bare for local, the endpoint URI otherwise.
    pub fn label(&self) -> String {
        match self {
            Self::Local => "local".to_string(),
            Self::Remote(endpoint) => endpoint.to_string(),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }
}

/// One document path on one filesystem: the identity diagnostics,
/// bindings and navigation route by.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DocPath {
    pub target: FsTarget,
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
}

impl DocPath {
    pub fn local(path: PathBuf) -> Self {
        Self {
            target: FsTarget::Local,
            path,
        }
    }

    pub fn remote(endpoint: RemoteEndpoint, path: PathBuf) -> Self {
        Self {
            target: FsTarget::Remote(endpoint),
            path,
        }
    }

    /// Modeline/trace form: the bare path locally, `endpoint + path`
    /// for a remote document — never an ambiguous local-looking path.
    pub fn label(&self) -> String {
        match &self.target {
            FsTarget::Local => self.path.display().to_string(),
            FsTarget::Remote(endpoint) => format!("{endpoint}{}", self.path.display()),
        }
    }
}

/// Where a spawned server runs: its filesystem target plus the
/// workspace root on that filesystem. The root anchors relative
/// document paths and is the server's own working directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Workspace {
    Local {
        root: PathBuf,
    },
    Remote {
        endpoint: RemoteEndpoint,
        root: PathBuf,
    },
}

impl Workspace {
    pub fn root(&self) -> &Path {
        match self {
            Self::Local { root } | Self::Remote { root, .. } => root,
        }
    }

    pub fn target(&self) -> FsTarget {
        match self {
            Self::Local { .. } => FsTarget::Local,
            Self::Remote { endpoint, .. } => FsTarget::Remote(endpoint.clone()),
        }
    }

    pub fn endpoint(&self) -> Option<&RemoteEndpoint> {
        match self {
            Self::Local { .. } => None,
            Self::Remote { endpoint, .. } => Some(endpoint),
        }
    }
    /// Absolute path on the workspace's own filesystem: pure joining,
    /// never a local canonicalization.
    pub fn absolute(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root().join(path)
        }
    }

    /// The `file://` URI a document path takes on this workspace's
    /// host. Local paths use the OS-native mapping; a remote path is
    /// percent-encoded from its native bytes so the URI names exactly
    /// those bytes on the server's own filesystem — no local probe.
    pub fn uri(&self, path: &Path) -> Option<async_lsp::lsp_types::Url> {
        match self {
            Self::Local { .. } => {
                async_lsp::lsp_types::Url::from_file_path(self.absolute(path)).ok()
            }
            Self::Remote { .. } => remote_file_uri(&self.absolute(path)),
        }
    }

    /// Decode a server URI onto this workspace's filesystem: `None`
    /// for other schemes, host-bearing file URIs or broken escapes.
    /// Remote decoding is pure percent-decoding — the analogous local
    /// path is never consulted.
    pub fn decode(&self, uri: &async_lsp::lsp_types::Url) -> Option<PathBuf> {
        if uri.scheme() != "file" || !uri.host_str().unwrap_or_default().is_empty() {
            return None;
        }
        match self {
            Self::Local { .. } => uri.to_file_path().ok(),
            Self::Remote { .. } => decode_uri_path(uri.path()),
        }
    }

    /// Trace/modeline label: `/root` locally, `ssh://user@host:port/root`
    /// for a remote server's workspace.
    pub fn label(&self) -> String {
        match self {
            Self::Local { root } => root.display().to_string(),
            Self::Remote { endpoint, root } => format!("{endpoint}{}", root.display()),
        }
    }
}

/// Native unix bytes of a path, on platforms that can see them.
#[cfg(unix)]
fn native_bytes(path: &Path) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    Some(path.as_os_str().as_bytes().to_vec())
}

/// Non-unix hosts cannot carry native remote bytes losslessly; refuse
/// instead of mangling (remote services need process supervision that
/// is unix-only there anyway).
#[cfg(not(unix))]
fn native_bytes(_path: &Path) -> Option<Vec<u8>> {
    None
}

#[cfg(unix)]
fn path_from_native(bytes: Vec<u8>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    // A NUL cannot name a real file on the target filesystem; a path
    // carrying one is refused, never truncated.
    if bytes.contains(&0) {
        return None;
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(bytes));
    path.is_absolute().then_some(path)
}

#[cfg(not(unix))]
fn path_from_native(_bytes: Vec<u8>) -> Option<PathBuf> {
    None
}

/// `file:///` + the path's native bytes, percent-encoded. Building the
/// URI from raw bytes (not `from_file_path`) keeps it independent of
/// the local OS's path rules — the remote filesystem is POSIX no
/// matter where the editor runs.
fn remote_file_uri(path: &Path) -> Option<async_lsp::lsp_types::Url> {
    let bytes = native_bytes(path)?;
    if !path.is_absolute() || bytes.contains(&0) {
        return None;
    }
    let mut text = String::with_capacity(bytes.len() + 8);
    text.push_str("file:///");
    for &byte in &bytes[1..] {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                text.push(byte as char)
            }
            _ => text.push_str(&format!("%{byte:02X}")),
        }
    }
    async_lsp::lsp_types::Url::parse(&text).ok()
}

/// Strict percent-decode of a URI path back to native bytes. A lone
/// `%` or non-hex escape is a refusal, never a lossy guess.
fn decode_uri_path(path: &str) -> Option<PathBuf> {
    let text = path.strip_prefix('/')?;
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some(&first) = rest.first() {
        rest = &rest[1..];
        if first != b'%' {
            bytes.push(first);
            continue;
        }
        if rest.len() < 2 {
            return None;
        }
        let high = (rest[0] as char).to_digit(16)?;
        let low = (rest[1] as char).to_digit(16)?;
        rest = &rest[2..];
        bytes.push((high * 16 + low) as u8);
    }
    let mut absolute = vec![b'/'];
    absolute.extend_from_slice(&bytes);
    path_from_native(absolute)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint() -> RemoteEndpoint {
        RemoteEndpoint::parse("ssh://dev@builder.example:2222").unwrap()
    }

    #[test]
    fn remote_uri_roundtrips_native_bytes_without_local_probe() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let path = PathBuf::from(std::ffi::OsString::from_vec(
                b"/srv/native \xff.rs".to_vec(),
            ));
            let workspace = Workspace::Remote {
                endpoint: endpoint(),
                root: PathBuf::from("/srv"),
            };
            let uri = workspace.uri(&path).expect("uri");
            assert_eq!(uri.scheme(), "file");
            assert_eq!(workspace.decode(&uri).as_deref(), Some(path.as_path()));
            // Decoding is pure math: it never stats the analogous local
            // path, which does not exist here.
            assert!(!path.exists());
        }
    }

    #[test]
    fn remote_decode_refuses_other_schemes_hosts_and_broken_escapes() {
        let workspace = Workspace::Remote {
            endpoint: endpoint(),
            root: PathBuf::from("/srv"),
        };
        let https = async_lsp::lsp_types::Url::parse("https://x/%41").unwrap();
        let hostful = async_lsp::lsp_types::Url::parse("file://builder.example/etc").unwrap();
        assert_eq!(workspace.decode(&https), None);
        assert_eq!(workspace.decode(&hostful), None);
        let broken = async_lsp::lsp_types::Url::parse("file:///a%2").unwrap();
        assert_eq!(workspace.decode(&broken), None);
        let nonhex = async_lsp::lsp_types::Url::parse("file:///a%zz").unwrap();
        assert_eq!(workspace.decode(&nonhex), None);
        // A percent-encoded NUL can never become a native path.
        let nul = async_lsp::lsp_types::Url::parse("file:///a%00b").unwrap();
        assert_eq!(workspace.decode(&nul), None);
    }

    #[test]
    fn targets_and_doc_paths_never_alias_across_hosts() {
        let endpoint = endpoint();
        let local = DocPath::local(PathBuf::from("/srv/x.rs"));
        let remote = DocPath::remote(endpoint, PathBuf::from("/srv/x.rs"));
        assert_ne!(local, remote);
        assert_ne!(local.target, remote.target);
        assert!(local.label().starts_with("/srv"));
        assert!(remote.label().starts_with("ssh://dev@builder.example"));
    }

    #[test]
    fn workspace_labels_and_roots() {
        let endpoint = endpoint();
        let local = Workspace::Local {
            root: PathBuf::from("/w"),
        };
        let remote = Workspace::Remote {
            endpoint: endpoint.clone(),
            root: PathBuf::from("/srv"),
        };
        assert_eq!(local.label(), "/w");
        assert_eq!(remote.label(), "ssh://dev@builder.example:2222/srv");
        assert_eq!(local.absolute(Path::new("a.rs")), PathBuf::from("/w/a.rs"));
        assert_eq!(
            remote.absolute(Path::new("/srv/a.rs")),
            PathBuf::from("/srv/a.rs")
        );
        assert_eq!(remote.target(), FsTarget::Remote(endpoint));
        assert!(local.endpoint().is_none());
    }
}
