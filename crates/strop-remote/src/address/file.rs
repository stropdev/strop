//! A canonical remote file: one validated endpoint plus an absolute native
//! path. This is the identity buffers, diagnostics and replay carry — it is
//! never an unresolved home query, so a `~`-leading first component can only
//! be the literal tilde directory, and the canonical URI keeps it escaped
//! (`%7E`) so re-parsing cannot silently turn it into a home query.

use super::endpoint::RemoteEndpoint;
use super::error::AddressError;
use super::uri::{self, path_bytes};
use std::path::{Path, PathBuf};

/// A canonical absolute remote file on one endpoint.
#[derive(Debug, Clone)]
pub struct RemoteFile {
    endpoint: RemoteEndpoint,
    path: PathBuf,
}

impl RemoteFile {
    /// Admit one canonical file location `ssh://[user@]host[:port]/abs/path`.
    /// A home query (`/~/…`) is refused: it is not canonical until a session
    /// expands it. Pure: no filesystem, no subprocess.
    pub fn parse(value: &str) -> Result<Self, AddressError> {
        let (endpoint, tail) = RemoteEndpoint::split_authority(value)?;
        match tail.as_bytes().first() {
            None => return Err(AddressError::AbsentPath),
            Some(b'/') => {}
            Some(_) => return Err(AddressError::QueryOrFragment),
        }
        if tail.contains(['?', '#']) {
            return Err(AddressError::QueryOrFragment);
        }
        if tail.as_bytes().get(1) == Some(&b'~') {
            return Err(AddressError::UnresolvedHome);
        }
        let path = uri::decode_path(tail)?;
        Self::from_path(endpoint, path)
    }

    /// The endpoint this file lives on — the pooled-session identity.
    pub fn endpoint(&self) -> &RemoteEndpoint {
        &self.endpoint
    }

    /// The decoded remote path. A remote POSIX path, not a local one.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The same endpoint addressing a different absolute native path:
    /// navigating a directory entry, a resolved home, a Git target.
    /// The path is admitted (absolute, no NUL, natively representable);
    /// it is never trimmed or normalized.
    pub fn with_path(&self, path: PathBuf) -> Result<Self, AddressError> {
        Self::from_path(self.endpoint.clone(), path)
    }

    /// Shared checked constructor: endpoint + already-native path bytes.
    pub fn from_path(endpoint: RemoteEndpoint, path: PathBuf) -> Result<Self, AddressError> {
        if !path.is_absolute() {
            return Err(AddressError::RelativePath);
        }
        if path_bytes(&path).contains(&0) {
            return Err(AddressError::NulInPath);
        }
        Ok(Self { endpoint, path })
    }

    /// The canonical URI: this is the identity replayed, traced and shown.
    fn canonical(&self) -> String {
        let bytes = path_bytes(&self.path);
        let mut out = String::with_capacity(
            self.endpoint.to_string().len() + bytes.len().saturating_mul(3) + 3,
        );
        out.push_str(&self.endpoint.to_string());
        // A canonical file is never a home query: a literal `~` naming the
        // first component stays escaped so the URI re-parses as this file.
        if bytes.get(1) == Some(&b'~') {
            out.push_str("/%7E");
            uri::push_escaped(&mut out, &bytes[2..]);
        } else {
            uri::push_escaped(&mut out, bytes);
        }
        out
    }
}

impl PartialEq for RemoteFile {
    fn eq(&self, other: &Self) -> bool {
        self.endpoint == other.endpoint && self.path.as_os_str() == other.path.as_os_str()
    }
}
impl Eq for RemoteFile {}
impl std::hash::Hash for RemoteFile {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.endpoint.hash(state);
        self.path.as_os_str().hash(state);
    }
}

impl From<RemoteFile> for RemoteEndpoint {
    fn from(file: RemoteFile) -> Self {
        file.endpoint
    }
}

impl std::fmt::Display for RemoteFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.canonical())
    }
}

/// Serde carries the canonical validated URI — one string, re-parsed on
/// the way in, so stored values can never bypass admission.
impl serde::Serialize for RemoteFile {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.canonical())
    }
}

impl<'de> serde::Deserialize<'de> for RemoteFile {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        RemoteFile::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::num::NonZeroU16;

    fn ok(uri: &str) -> RemoteFile {
        RemoteFile::parse(uri).unwrap_or_else(|error| panic!("expected parse: {uri}: {error}"))
    }

    fn refused(uri: &str) -> AddressError {
        RemoteFile::parse(uri).unwrap_err()
    }

    #[test]
    fn fields_come_from_the_endpoint_and_the_decoded_path() {
        let file = ok("ssh://dev@bbgithub:2222/var/log/app.log");
        assert_eq!(file.endpoint().host(), "bbgithub");
        assert_eq!(file.endpoint().user(), Some("dev"));
        assert_eq!(file.endpoint().port(), NonZeroU16::new(2222));
        assert_eq!(file.path(), Path::new("/var/log/app.log"));
        assert_eq!(file.to_string(), "ssh://dev@bbgithub:2222/var/log/app.log");
    }

    #[test]
    fn a_home_query_is_not_canonical() {
        for uri in ["ssh://h/~/log", "ssh://h/~", "ssh://h/~alice/x"] {
            assert!(
                matches!(refused(uri), AddressError::UnresolvedHome),
                "{uri}"
            );
        }
    }

    #[test]
    fn with_path_keeps_the_endpoint_and_checks_admission() {
        let file = ok("ssh://dev@box:2222/var/log/app.log");
        let moved = file
            .with_path(PathBuf::from("/var/log/rotated.log"))
            .expect("absolute path");
        assert_eq!(moved.endpoint(), file.endpoint());
        assert_eq!(moved.path(), Path::new("/var/log/rotated.log"));
        assert_eq!(moved.to_string(), "ssh://dev@box:2222/var/log/rotated.log");
        assert!(matches!(
            file.with_path(PathBuf::from("relative/log")),
            Err(AddressError::RelativePath)
        ));
    }

    #[test]
    fn absent_path_and_query_are_refused() {
        for uri in ["ssh://host", "ssh://host:22"] {
            assert!(matches!(refused(uri), AddressError::AbsentPath), "{uri}");
        }
        for uri in [
            "ssh://h/a?b",
            "ssh://h/a#b",
            "ssh://h?q",
            "ssh://h#f",
            "ssh://h/a%3Fb?c",
        ] {
            assert!(
                matches!(refused(uri), AddressError::QueryOrFragment),
                "{uri}"
            );
        }
    }

    #[test]
    fn percent_decoding_and_canonical_display() {
        let file = ok("ssh://h/var%20log/a%23b%25c%3Fd");
        assert_eq!(file.path(), Path::new("/var log/a#b%c?d"));
        assert_eq!(file.to_string(), "ssh://h/var%20log/a%23b%25c%3Fd");
        assert_eq!(file, ok(&file.to_string()));
    }

    #[test]
    fn malformed_percent_and_nul() {
        for uri in [
            "ssh://h/%",
            "ssh://h/%2",
            "ssh://h/%G1",
            "ssh://h/a%zz",
            "ssh://h/a%2G",
        ] {
            assert!(
                matches!(refused(uri), AddressError::MalformedPercentEscape),
                "{uri}"
            );
        }
        assert!(matches!(refused("ssh://h/%00"), AddressError::NulInPath));
        assert!(matches!(
            refused("ssh://h/a\u{0}b"),
            AddressError::NulInPath
        ));
    }

    #[test]
    fn shell_metacharacters_are_path_data() {
        let uri = "ssh://h/log/$x&(rm);'q'*+,.~!@:x";
        let file = ok(uri);
        assert_eq!(file.path(), Path::new("/log/$x&(rm);'q'*+,.~!@:x"));
        assert_eq!(file.to_string(), uri);
    }

    #[test]
    fn unencodable_raw_bytes_are_refused() {
        for uri in [
            "ssh://h/a b",
            "ssh://h/a`b",
            "ssh://h/a\"b",
            "ssh://h/a\x01b",
            "ssh://h/a\x7fb",
            "ssh://h/a\\b",
        ] {
            assert!(
                matches!(refused(uri), AddressError::UnencodedPathByte),
                "{uri}"
            );
        }
    }

    #[test]
    fn unicode_path_data_roundtrips_through_escapes() {
        let file = ok("ssh://h/日本語-légère.log");
        assert_eq!(file.path(), Path::new("/日本語-légère.log"));
        let displayed = file.to_string();
        assert_eq!(
            displayed,
            "ssh://h/%E6%97%A5%E6%9C%AC%E8%AA%9E-l%C3%A9g%C3%A8re.log"
        );
        assert_eq!(ok(&displayed), file);
    }

    #[test]
    fn paths_are_never_trimmed_or_normalized() {
        let file = ok("ssh://h/a/../b//c/./d/");
        assert_eq!(file.path(), Path::new("/a/../b//c/./d/"));
        assert_eq!(file.to_string(), "ssh://h/a/../b//c/./d/");
        assert_eq!(ok("ssh://h/").path(), Path::new("/"));
    }

    #[test]
    fn a_literal_tilde_directory_is_not_aliased_to_a_home_query() {
        // `%7E` names a directory literally called `~` under `/`.
        let literal = ok("ssh://h/%7E/log/app.log");
        assert_eq!(literal.path(), Path::new("/~/log/app.log"));
        // The canonical spelling re-escapes it, so identity round-trips
        // and can never re-enter as the unresolved home form.
        assert_eq!(literal.to_string(), "ssh://h/%7E/log/app.log");
        assert_eq!(ok(&literal.to_string()), literal);
        assert!(matches!(
            RemoteFile::parse("ssh://h/~/log/app.log"),
            Err(AddressError::UnresolvedHome)
        ));
    }

    #[test]
    fn identity_follows_the_decoded_value() {
        assert_eq!(ok("ssh://h/%61%62"), ok("ssh://h/ab"));
        assert_eq!(ok("ssh://h/a%2Fb"), ok("ssh://h/a/b"));
        assert_eq!(ok("ssh://h:022/x"), ok("ssh://h:22/x"));
        assert_eq!(ok("ssh://[::1]/x"), ok("ssh://[0:0:0:0:0:0:0:1]/x"));
        assert_ne!(ok("ssh://h/log"), ok("ssh://h/log/"));
        assert_ne!(ok("ssh://h/a/b"), ok("ssh://h/a//b"));
        let mut seen = HashSet::new();
        seen.insert(ok("ssh://h:022/x"));
        seen.insert(ok("ssh://h:22/x"));
        seen.insert(ok("ssh://[::1]/x"));
        seen.insert(ok("ssh://[0::1]/x"));
        assert_eq!(seen.len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn native_filename_bytes_roundtrip() {
        use std::os::unix::ffi::OsStrExt;
        let file = ok("ssh://h/l%FCg%FF");
        assert_eq!(file.path().as_os_str().as_bytes(), b"/l\xFCg\xFF");
        assert_eq!(file.to_string(), "ssh://h/l%FCg%FF");
        assert_eq!(ok(&file.to_string()), file);
    }

    #[cfg(not(unix))]
    #[test]
    fn unrepresentable_native_bytes_are_refused() {
        assert!(matches!(
            refused("ssh://h/l%FCg"),
            Err(AddressError::UnrepresentablePath)
        ));
    }

    #[test]
    fn serde_roundtrips_the_canonical_uri() {
        let file = ok("ssh://dev@box:2222/var%20log/a.log");
        let json = serde_json::to_string(&file).expect("serialize");
        assert_eq!(json, "\"ssh://dev@box:2222/var%20log/a.log\"");
        assert_eq!(
            serde_json::from_str::<RemoteFile>(&json).expect("deserialize"),
            file
        );
    }

    #[test]
    fn serde_rejects_invalid_values() {
        assert!(serde_json::from_str::<RemoteFile>("\"ssh://h/%zz\"").is_err());
        assert!(serde_json::from_str::<RemoteFile>("\"/local/path\"").is_err());
        // Field-shaped values cannot bypass the URI admission either.
        let shaped = serde_json::json!({"host": "h", "path": "/x"});
        assert!(serde_json::from_value::<RemoteFile>(shaped).is_err());
    }
}
