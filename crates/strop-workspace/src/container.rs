//! Container namespace identity (0037 DC1a): a running container on the
//! local engine is its own filesystem namespace — paths inside it are
//! never local paths. The engine is local-only in v1; a remote-engine
//! field is earned with DC5, not reserved.

/// A canonical container identity: the 64-hex id `docker inspect`
/// reports. Names and prefixes resolve through inspect first; the checked
/// constructor keeps a display label from ever becoming an identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContainerId(String);

/// Why a container identity string was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a container identity is the 64-hex inspect id, not a name or prefix")]
pub struct ContainerIdError;

impl ContainerId {
    pub fn canonical(value: String) -> Result<Self, ContainerIdError> {
        let valid = value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if valid {
            Ok(Self(value))
        } else {
            Err(ContainerIdError)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContainerId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for ContainerId {
    type Error = ContainerIdError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::canonical(value)
    }
}
impl From<ContainerId> for String {
    fn from(value: ContainerId) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ids_roundtrip_and_names_refuse() {
        let id = ContainerId::canonical("a".repeat(64)).unwrap();
        assert_eq!(id.as_str(), &"a".repeat(64));
        assert!(ContainerId::canonical("name".into()).is_err());
        assert!(
            ContainerId::canonical("A".repeat(64)).is_err(),
            "lowercase hex"
        );
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<ContainerId>(&json).unwrap(), id);
        assert!(serde_json::from_str::<ContainerId>("\"name\"").is_err());
    }
}
