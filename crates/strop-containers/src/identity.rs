//! Container identity: what `inspect` resolved, and the canonical
//! incarnation-pinned reference every read carries.

use crate::ContainerError;
use strop_workspace::ContainerId;

/// One container as the engine described it at inspect time. Plain data —
/// resolving or re-checking it is [`crate::inspect`] /
/// [`crate::revalidate`]'s job, never this struct's.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContainerIdentity {
    /// The canonical 64-hex id `docker inspect` reported.
    pub id: String,
    /// The container's name, without inspect's leading `/`.
    pub name: String,
    /// The image reference the container was created from (`Config.Image`).
    pub image: String,
    /// The incarnation marker (`State.StartedAt`, RFC 3339): a restart of
    /// the same id changes it, so held results can be told stale cheaply.
    pub started_at: String,
    /// `Config.User`; empty means the image default.
    pub user: String,
}

/// A canonical, incarnation-pinned reference to one running container.
///
/// Built only from an inspected [`ContainerIdentity`]: the 64-hex id is
/// revalidated at construction, so a display label or name can never
/// become an identity by accident. Every read re-checks `started_at`
/// against the engine before touching the filesystem, so a restart or
/// same-name recreation is a typed [`ContainerError::StaleIdentity`]
/// rather than silently wrong bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRef {
    id: ContainerId,
    started_at: String,
}

impl ContainerRef {
    /// The canonical reference for an inspected identity. Refuses (as a
    /// protocol violation) an identity whose id is not the 64-hex inspect
    /// id — names and prefixes resolve through [`crate::inspect`] first.
    pub fn of(identity: &ContainerIdentity) -> Result<Self, ContainerError> {
        let id =
            ContainerId::canonical(identity.id.clone()).map_err(|_| ContainerError::Protocol {
                detail: format!(
                    "inspect id {:?} is not the canonical 64-hex id",
                    identity.id
                ),
            })?;
        Ok(Self {
            id,
            started_at: identity.started_at.clone(),
        })
    }

    /// The canonical 64-hex id — the same identity
    /// [`strop_workspace::Filesystem::Container`] carries.
    pub fn id(&self) -> &ContainerId {
        &self.id
    }

    /// The incarnation marker captured at inspect time.
    pub fn started_at(&self) -> &str {
        &self.started_at
    }

    /// Display form of the incarnation: `id@started_at`.
    pub(crate) fn incarnation(&self) -> String {
        format!("{}@{}", self.id, self.started_at)
    }
}

/// Refuse names that could inject CLI options or can never resolve.
///
/// Accepted: Docker's name grammar `[A-Za-z0-9][A-Za-z0-9_.-]*` (a
/// superset that also covers hex id prefixes). Refused: empty, leading
/// `-` (option-shaped), path separators, whitespace, control bytes and
/// anything over 255 bytes — [`ContainerError::PoisonedName`] before the
/// engine is ever invoked.
pub(crate) fn validate_name(name: &str) -> Result<(), ContainerError> {
    let valid = !name.is_empty()
        && name.len() <= 255
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(ContainerError::PoisonedName {
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(id: String) -> ContainerIdentity {
        ContainerIdentity {
            id,
            name: "fixture".into(),
            image: "busybox".into(),
            started_at: "2026-09-10T08:00:00Z".into(),
            user: String::new(),
        }
    }

    #[test]
    fn names_are_validated_before_the_engine_sees_them() {
        assert!(validate_name("web").is_ok());
        assert!(validate_name("web_1.2-alpine").is_ok());
        assert!(validate_name(&"a".repeat(64)).is_ok(), "hex id");
        assert!(validate_name("f00dbabe").is_ok(), "id prefix");
        for bad in [
            "", "-rf", "--format", "a b", "a/b", "a:b", ".hidden", "_lead", "é",
        ] {
            assert!(
                matches!(validate_name(bad), Err(ContainerError::PoisonedName { .. })),
                "{bad:?} must be refused"
            );
        }
        assert!(validate_name(&"a".repeat(256)).is_err(), "length bound");
    }

    #[test]
    fn references_carry_only_canonical_ids() {
        let reference = ContainerRef::of(&identity("a".repeat(64))).unwrap();
        assert_eq!(reference.id().as_str(), &"a".repeat(64));
        assert_eq!(reference.started_at(), "2026-09-10T08:00:00Z");
        assert!(ContainerRef::of(&identity("web".into())).is_err());
        assert!(ContainerRef::of(&identity("A".repeat(64))).is_err());
    }
}
