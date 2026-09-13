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
    /// Explicit, lossless clipboard/completion spelling. Display labels are not
    /// used as an input codec, especially for non-UTF-8 names and container IDs.
    pub fn uri(&self) -> Result<String, crate::AddressError> {
        if !self.path.is_absolute() {
            return Err(crate::AddressError::RelativePath);
        }
        if crate::addr::uri::path_bytes(&self.path).contains(&0) {
            return Err(crate::AddressError::NulInPath);
        }
        let mut value = match &self.filesystem {
            Filesystem::Local => "file://".to_owned(),
            Filesystem::Remote(endpoint) => {
                return crate::RemoteFile::from_path(endpoint.clone(), self.path.clone())
                    .map(|file| file.to_string())
            }
            Filesystem::Container(id) => format!("container:{id}"),
        };
        crate::addr::uri::push_escaped(&mut value, crate::addr::uri::path_bytes(&self.path));
        Ok(value)
    }

    pub fn parse_uri(value: &str) -> Result<Self, crate::AddressError> {
        if value.starts_with("ssh://") {
            let file = crate::RemoteFile::parse(value)?;
            return Ok(Self::remote(
                file.endpoint().clone(),
                file.path().to_path_buf(),
            ));
        }
        let (filesystem, raw_path) = if let Some(rest) = value.strip_prefix("file://") {
            let path = if rest.starts_with('/') {
                rest
            } else {
                rest.strip_prefix("localhost")
                    .filter(|path| path.starts_with('/'))
                    .ok_or(crate::AddressError::NonLocalFileAuthority)?
            };
            (Filesystem::Local, path)
        } else if let Some(rest) = value.strip_prefix("container:") {
            let slash = rest
                .find('/')
                .ok_or(crate::AddressError::InvalidContainerLocation)?;
            let id = crate::ContainerId::canonical(rest[..slash].to_owned())
                .map_err(|_| crate::AddressError::InvalidContainerLocation)?;
            (Filesystem::Container(id), &rest[slash..])
        } else {
            return Err(crate::AddressError::UnsupportedResourceUri);
        };
        let path = crate::addr::uri::decode_path(raw_path)?;
        if !path.is_absolute() {
            return Err(crate::AddressError::RelativePath);
        }
        Ok(Self { filesystem, path })
    }
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

    /// Map an exact resource or descendant through a same-namespace relocation.
    /// Empty relative paths must not append a slash to a regular-file binding.
    pub fn relocated(&self, source: &Self, destination: &Self) -> Option<Self> {
        if self.filesystem != source.filesystem || source.filesystem != destination.filesystem {
            return None;
        }
        let relative = self.path.strip_prefix(&source.path).ok()?;
        Some(Self {
            filesystem: self.filesystem.clone(),
            path: if relative.as_os_str().is_empty() {
                destination.path.clone()
            } else {
                destination.path.join(relative)
            },
        })
    }

    /// Modeline/trace form: the bare path locally, `endpoint + path`
    /// for a remote resource — never an ambiguous local-looking path.
    pub fn label(&self) -> String {
        let path = crate::directory::display_path(&self.path);
        match &self.filesystem {
            Filesystem::Local => path,
            other => format!("{}{path}", other.label()),
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

    #[test]
    fn resource_uri_roundtrips_without_namespace_fallback() {
        let local = ResourceLocation::local("/tmp/a b#c%/ssh:literal".into());
        assert_eq!(
            ResourceLocation::parse_uri(&local.uri().unwrap()).unwrap(),
            local
        );
        assert!(ResourceLocation::parse_uri("file://another-host/tmp/a").is_err());
        assert!(ResourceLocation::parse_uri("file:///tmp/%00").is_err());
        assert!(ResourceLocation::parse_uri("container:short/tmp/a").is_err());
        let container = ResourceLocation {
            filesystem: Filesystem::Container(
                crate::ContainerId::canonical("a".repeat(64)).unwrap(),
            ),
            path: "/tmp/a b".into(),
        };
        assert_eq!(
            ResourceLocation::parse_uri(&container.uri().unwrap()).unwrap(),
            container
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_names_and_literal_escape_text_have_distinct_display_and_uri() {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/a\xff".to_vec()));
        let resource = ResourceLocation::local(path);
        let literal = ResourceLocation::local("/tmp/a\\xFF".into());
        assert_ne!(resource.label(), literal.label());
        assert_ne!(resource.uri().unwrap(), literal.uri().unwrap());
        assert_eq!(
            ResourceLocation::parse_uri(&resource.uri().unwrap()).unwrap(),
            resource
        );
    }
}
