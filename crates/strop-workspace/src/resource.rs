//! One resolved resource path on one filesystem: the identity routing,
//! diagnostics, bindings and navigation share (0042). Supersedes
//! strop-lsp's `DocPath`; the serialized field accepts the legacy
//! `target` name so records written before the move still decode.

use std::path::PathBuf;

use crate::addr::RemoteEndpoint;
use crate::filesystem::Filesystem;

/// One resolved path on one filesystem namespace. Always resolved: an
/// unresolved user entry (a `~` home query) is a
/// [`crate::addr::RemoteLocation`], never a `ResourceLocation`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ResourceLocation {
    #[serde(alias = "target")]
    pub filesystem: Filesystem,
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
}

impl ResourceLocation {
    pub fn local(path: PathBuf) -> Self {
        Self {
            filesystem: Filesystem::Local,
            path,
        }
    }

    pub fn remote(endpoint: RemoteEndpoint, path: PathBuf) -> Self {
        Self {
            filesystem: Filesystem::Remote(endpoint),
            path,
        }
    }

    /// Modeline/trace form: the bare path locally, `endpoint + path`
    /// for a remote resource — never an ambiguous local-looking path.
    pub fn label(&self) -> String {
        match &self.filesystem {
            Filesystem::Local => self.path.display().to_string(),
            other => format!("{}{}", other.label(), self.path.display()),
        }
    }

    /// The local path, only when this resource is on the local disk.
    /// There is no other way to read the path as a local one.
    pub fn local_path(&self) -> Option<&std::path::Path> {
        match &self.filesystem {
            Filesystem::Local => Some(&self.path),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_legacy_docpath_shape() {
        // Written by strop-lsp's DocPath (0.19.1): field name `target`.
        let legacy = serde_json::json!({
            "target": "Local",
            "path": "/workspace/a.rs",
        });
        let location: ResourceLocation = serde_json::from_value(legacy).unwrap();
        assert_eq!(location.filesystem, Filesystem::Local);
        assert_eq!(location.path, PathBuf::from("/workspace/a.rs"));
        let endpoint = RemoteEndpoint::parse("ssh://user@example.com:2222").unwrap();
        let legacy = serde_json::json!({
            "target": { "Remote": endpoint },
            "path": "/var/log/app.log",
        });
        let location: ResourceLocation = serde_json::from_value(legacy).unwrap();
        assert_eq!(location.filesystem, Filesystem::Remote(endpoint));
    }

    #[test]
    fn remote_resources_never_expose_a_local_path() {
        let endpoint = RemoteEndpoint::parse("ssh://example.com").unwrap();
        let location = ResourceLocation::remote(endpoint, PathBuf::from("/etc/hostname"));
        assert_eq!(location.local_path(), None);
        assert!(location.label().starts_with("ssh://example.com"));
    }
}
