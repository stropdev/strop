//! Explicit remote editing. Atomic replacement and cooperative locking are owned
//! by the shipped helper; nonparticipating writers are not excluded by flock.
mod protocol;
#[cfg(all(test, unix))]
mod tests;

use crate::{ReadLimit, RemoteFile};
use ropey::Rope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use strop_core::worker::CancelToken;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ContentDigest([u8; 32]);
impl ContentDigest {
    fn of(text: &Rope) -> Self {
        let mut hash = Sha256::new();
        for chunk in text.chunks() {
            hash.update(chunk.as_bytes());
        }
        Self(hash.finalize().into())
    }
}

/// An opaque baseline binds one file to its content and filesystem metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteVersion {
    file: RemoteFile,
    stamp: Stamp,
}
impl RemoteVersion {
    pub fn file(&self) -> &RemoteFile {
        &self.file
    }
    pub fn size(&self) -> crate::RemoteSize {
        crate::RemoteSize::new(self.stamp.size)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stamp {
    device: u64,
    inode: u64,
    size: u64,
    mtime_ns: i64,
    ctime_ns: i64,
    mode: u32,
    uid: u32,
    gid: u32,
    content: ContentDigest,
    attributes: ContentDigest,
}
impl Stamp {
    fn valid(&self) -> bool {
        self.size <= ReadLimit::MAX && self.mode <= 0o7777
    }
    fn preserves(&self, before: &Self) -> bool {
        self.mode == before.mode
            && self.uid == before.uid
            && self.gid == before.gid
            && self.mtime_ns == before.mtime_ns
            && self.attributes == before.attributes
            && self.device == before.device
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSaveReceipt {
    version: RemoteVersion,
}
impl RemoteSaveReceipt {
    pub fn into_version(self) -> RemoteVersion {
        self.version
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verification {
    Unchanged(RemoteVersion),
    Written(RemoteSaveReceipt),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalKind {
    Conflict,
    Busy,
    Unsupported,
    Permission,
    InvalidPath,
    TooLarge,
    Metadata,
    Protocol,
    Io,
    Cancelled,
}
impl std::fmt::Display for RefusalKind {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(match self {
            Self::Conflict => "remote file changed",
            Self::Busy => "remote file busy",
            Self::Unsupported => "unsupported remote save capability",
            Self::Permission => "remote permission denied",
            Self::InvalidPath => "remote path refused",
            Self::TooLarge => "remote snapshot too large",
            Self::Metadata => "remote metadata cannot be preserved",
            Self::Protocol => "invalid remote save response",
            Self::Io => "remote I/O failed",
            Self::Cancelled => "remote operation cancelled before commit",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum RemoteSaveError {
    #[error("{kind}: {detail}")]
    Refused { kind: RefusalKind, detail: String },
    #[error("remote save outcome unconfirmed: {detail}; use :remote verify")]
    Unconfirmed { detail: String },
}
impl RemoteSaveError {
    fn refused(kind: RefusalKind, detail: impl Into<String>) -> Self {
        Self::Refused {
            kind,
            detail: detail.into(),
        }
    }
    pub fn is_unconfirmed(&self) -> bool {
        matches!(self, Self::Unconfirmed { .. })
    }
}

fn checked_length(contents: &Rope) -> Result<u64, RemoteSaveError> {
    let length = contents.len_bytes() as u64;
    if length > ReadLimit::MAX {
        Err(RemoteSaveError::refused(
            RefusalKind::TooLarge,
            "edited content exceeds the 256 MiB bound",
        ))
    } else {
        Ok(length)
    }
}

/// Worker-only admission. Nothing grants editability until these exact displayed
/// bytes match the remote file and its no-follow/metadata capabilities are checked.
pub fn prepare_edit(
    file: &RemoteFile,
    contents: &Rope,
    token: &CancelToken,
) -> Result<RemoteVersion, RemoteSaveError> {
    let length = checked_length(contents)?;
    let digest = ContentDigest::of(contents);
    let reply = protocol::invoke(
        file,
        protocol::Operation::Edit {
            length,
            digest: &digest,
        },
        token,
    )?;
    let protocol::Reply::Ready { stamp } = reply else {
        return Err(RemoteSaveError::refused(
            RefusalKind::Protocol,
            "expected an edit baseline",
        ));
    };
    if !stamp.valid() || stamp.size != length || stamp.content != digest {
        return Err(RemoteSaveError::refused(
            RefusalKind::Protocol,
            "baseline does not match the displayed snapshot",
        ));
    }
    Ok(RemoteVersion {
        file: file.clone(),
        stamp,
    })
}

/// Worker-only conditional atomic replacement. The file is carried by its baseline,
/// so a caller cannot accidentally pair a different path with the expected version.
pub fn save(
    before: &RemoteVersion,
    contents: &Rope,
    token: &CancelToken,
) -> Result<RemoteSaveReceipt, RemoteSaveError> {
    let length = checked_length(contents)?;
    let digest = ContentDigest::of(contents);
    let reply = protocol::invoke(
        &before.file,
        protocol::Operation::Save {
            before: &before.stamp,
            length,
            digest: &digest,
            contents,
        },
        token,
    )?;
    let protocol::Reply::Written { stamp } = reply else {
        return Err(RemoteSaveError::Unconfirmed {
            detail: "expected a durable save receipt".into(),
        });
    };
    receipt(before, stamp, &digest, length)
}

/// Explicit reconciliation after an ambiguous result. A matching intended state
/// is synced again before acknowledgment; this does not prove historical authorship.
pub fn verify(
    before: &RemoteVersion,
    intended: &Rope,
    token: &CancelToken,
) -> Result<Verification, RemoteSaveError> {
    let length = checked_length(intended)?;
    let digest = ContentDigest::of(intended);
    match protocol::invoke(
        &before.file,
        protocol::Operation::Verify {
            before: &before.stamp,
            length,
            digest: &digest,
        },
        token,
    )? {
        protocol::Reply::Unchanged { stamp } if stamp == before.stamp => {
            Ok(Verification::Unchanged(before.clone()))
        }
        protocol::Reply::Written { stamp } => {
            receipt(before, stamp, &digest, length).map(Verification::Written)
        }
        _ => Err(RemoteSaveError::Unconfirmed {
            detail: "verification returned an inconsistent state".into(),
        }),
    }
}

fn receipt(
    before: &RemoteVersion,
    stamp: Stamp,
    digest: &ContentDigest,
    length: u64,
) -> Result<RemoteSaveReceipt, RemoteSaveError> {
    if !stamp.valid()
        || stamp.size != length
        || stamp.content != *digest
        || !stamp.preserves(&before.stamp)
    {
        return Err(RemoteSaveError::Unconfirmed {
            detail: "receipt does not match the intended bytes and metadata".into(),
        });
    }
    Ok(RemoteSaveReceipt {
        version: RemoteVersion {
            file: before.file.clone(),
            stamp,
        },
    })
}
