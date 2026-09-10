//! The filesystem namespace a path names: the local disk, or exactly one
//! remote endpoint. Part of identity — two filesystems never share a path,
//! so a remote path can never alias a local path with the same bytes.
//!
//! This supersedes strop-lsp's `FsTarget` (0042): one definition for the
//! editor, LSP, Git and the picker instead of one per consumer. The serde
//! wire shape is unchanged (`Local` / `Remote`), so traces and sessions
//! written before the move still decode.
use crate::addr::RemoteEndpoint;
use crate::container::ContainerId;

/// The filesystem a path names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Filesystem {
    Local,
    Remote(RemoteEndpoint),
    /// A running container on the local engine (0037 DC1a) — paths inside
    /// it are never local paths.
    Container(ContainerId),
}

impl Default for Filesystem {
    /// The local disk: records predating remote filesystems deserialize
    /// as local — never guessed remote.
    fn default() -> Self {
        Self::Local
    }
}

impl Filesystem {
    /// Modeline/trace form: bare for local, the endpoint URI otherwise.
    /// Modeline/trace form: bare for local, the endpoint URI or the
    /// short container id otherwise.
    pub fn label(&self) -> String {
        match self {
            Self::Local => "local".to_string(),
            Self::Remote(endpoint) => endpoint.to_string(),
            Self::Container(id) => format!("container:{}", &id.as_str()[..12]),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }

    pub fn endpoint(&self) -> Option<&RemoteEndpoint> {
        match self {
            Self::Remote(endpoint) => Some(endpoint),
            _ => None,
        }
    }
}
