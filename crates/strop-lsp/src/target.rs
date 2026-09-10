//! Local-vs-remote workspace identity (0036 RW8). A language server
//! runs somewhere and its URIs name that host's filesystem: the
//! filesystem is part of every path identity, so a remote path can
//! never alias a local path with the same bytes. The identity types
//! themselves — `Filesystem`, `ResourceLocation`, `RemoteEndpoint` —
//! live in strop-workspace (0042); this module keeps the `Workspace`
//! spawn root and the `file://` URI math, which needs async-lsp's
//! `Url` (strop-workspace never depends on async-lsp). All URI math
//! here is pure — no local filesystem probe ever resolves a remote
//! location.

use std::path::{Path, PathBuf};

use strop_workspace::addr::uri;
use strop_workspace::{ContainerId, Filesystem, RemoteEndpoint};

/// Where a spawned server runs: its filesystem target plus the
/// workspace root on that filesystem. The root anchors relative
/// document paths and is the server's own working directory. Never
/// serialized — wire identity is [`strop_workspace::ResourceLocation`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Workspace {
    Local {
        root: PathBuf,
    },
    Remote {
        endpoint: RemoteEndpoint,
        root: PathBuf,
    },
    /// A language server inside a running container (0037 DC1b): the
    /// workspace root names container paths, never local ones.
    Container {
        container: ContainerId,
        root: PathBuf,
    },
}

impl Workspace {
    pub fn root(&self) -> &Path {
        match self {
            Self::Local { root } | Self::Remote { root, .. } | Self::Container { root, .. } => root,
        }
    }

    pub fn target(&self) -> Filesystem {
        match self {
            Self::Local { .. } => Filesystem::Local,
            Self::Remote { endpoint, .. } => Filesystem::Remote(endpoint.clone()),
            Self::Container { container, .. } => Filesystem::Container(container.clone()),
        }
    }

    pub fn endpoint(&self) -> Option<&RemoteEndpoint> {
        match self {
            Self::Remote { endpoint, .. } => Some(endpoint),
            _ => None,
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
            // Container paths are POSIX inside the container: the remote
            // byte codec, never the local OS's path rules.
            Self::Remote { .. } | Self::Container { .. } => remote_file_uri(&self.absolute(path)),
        }
    }

    /// Decode a server URI onto this workspace's filesystem: `None`
    /// for other schemes, host-bearing file URIs or broken escapes.
    /// Remote decoding is pure percent-decoding — the analogous local
    /// path is never consulted.
    pub fn decode(&self, uri_: &async_lsp::lsp_types::Url) -> Option<PathBuf> {
        if uri_.scheme() != "file" || !uri_.host_str().unwrap_or_default().is_empty() {
            return None;
        }
        match self {
            Self::Local { .. } => uri_.to_file_path().ok(),
            Self::Remote { .. } | Self::Container { .. } => decode_uri_path(uri_.path()),
        }
    }

    /// Trace/modeline label: `/root` locally, `ssh://user@host:port/root`
    /// for a remote server's workspace.
    pub fn label(&self) -> String {
        match self {
            Self::Local { root } => root.display().to_string(),
            Self::Remote { endpoint, root } => format!("{endpoint}{}", root.display()),
            Self::Container { container, root } => {
                format!("container:{}{}", &container.as_str()[..12], root.display())
            }
        }
    }
}

/// `file:///` + the path's native bytes, percent-encoded. Building the
/// URI from raw bytes (not `from_file_path`) keeps it independent of
/// the local OS's path rules — the remote filesystem is POSIX no
/// matter where the editor runs. Native byte access is strop-workspace's
/// codec; only the escape loop and the `Url` conversion live here.
fn remote_file_uri(path: &Path) -> Option<async_lsp::lsp_types::Url> {
    let bytes = uri::path_bytes(path);
    // A remote POSIX path is absolute and slash-led; anything else
    // (or a NUL) can never name a remote file.
    if bytes.first() != Some(&b'/') || bytes.contains(&0) {
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
    // A NUL cannot name a real file on the target filesystem; a path
    // carrying one is refused, never truncated.
    if bytes.contains(&0) {
        return None;
    }
    let mut absolute = vec![b'/'];
    absolute.extend_from_slice(&bytes);
    let path = uri::bytes_to_path(absolute).ok()?;
    path.is_absolute().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_workspace::ResourceLocation;

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
            let uri_ = workspace.uri(&path).expect("uri");
            assert_eq!(uri_.scheme(), "file");
            assert_eq!(workspace.decode(&uri_).as_deref(), Some(path.as_path()));
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
    fn filesystems_and_locations_never_alias_across_hosts() {
        let endpoint = endpoint();
        let local = ResourceLocation::local(PathBuf::from("/srv/x.rs"));
        let remote = ResourceLocation::remote(endpoint, PathBuf::from("/srv/x.rs"));
        assert_ne!(local, remote);
        assert_ne!(local.filesystem, remote.filesystem);
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
        assert_eq!(remote.target(), Filesystem::Remote(endpoint));
        assert!(local.endpoint().is_none());
    }
}
