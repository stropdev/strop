//! Permalink remote normalization and URL construction (0001 pillar
//! 3.3, 0033 finding 1). Everything here is pure — no filesystem, no
//! environment, no process. SSH aliases come back to the caller as
//! unresolved data; OpenSSH effective-configuration evaluation lives
//! in [`crate::ssh`] and is owned IO-worker work, never this module.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::{Component, Path};

/// A remote whose web identity is fully known: permalink construction
/// from here is pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebRemote {
    /// Scheme plus authority, `"https://github.com"`. HTTP(S) remotes
    /// keep their full authority — host and port survive, nested
    /// groups included; SSH-derived bases drop user and SSH port (the
    /// web port is not the SSH port).
    pub base: String,
    /// Repository path under the base with one trailing `.git`
    /// stripped: `"acme/demo"` — nested paths survive whole.
    pub repo: String,
}

/// An SSH endpoint whose effective configuration has not been evaluated.
/// Any host spelling can be remapped by OpenSSH, including dotted aliases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasRemote {
    /// The alias exactly as the remote spells it (validated host
    /// characters only, so it cannot become an option later).
    pub alias: String,
    pub repo: String,
    pub(crate) user: Option<String>,
    pub(crate) port: Option<std::num::NonZeroU16>,
}

impl AliasRemote {
    pub fn host(&self) -> &str {
        &self.alias
    }
    /// Pure: fold OpenSSH's effective hostname into a web remote.
    pub fn resolved(&self, hostname: &str) -> Option<WebRemote> {
        if !is_safe_host(hostname) || (hostname == self.alias && !hostname.contains('.')) {
            return None;
        }
        Some(WebRemote {
            base: format!("https://{hostname}"),
            repo: self.repo.clone(),
        })
    }
}

/// The remote a permalink will point at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectedRemote {
    Web(WebRemote),
    Alias(AliasRemote),
}

/// Parse one remote URL. Pure — no IO of any kind.
///
/// - HTTP(S) remotes keep their full authority (host and port) and
///   their whole repository path, nested or not; userinfo is dropped
///   so credentials never reach a link.
/// - Every SSH host is evaluated with OpenSSH; punctuation cannot tell
///   a literal host from an alias, and Match rules can depend on user/port.
/// - Everything else — local paths, `git://`, bracketed IPv6,
///   option-shaped text — is an honest `None`, never a guessed base.
pub fn parse_remote(url: &str) -> Option<SelectedRemote> {
    let url = url.trim();
    if let Some(rest) = url.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/')?;
        return selected(authority, uri_repo(path)?);
    }
    if let Some((scheme, rest)) = url
        .split_once("://")
        .filter(|(scheme, _)| matches!(*scheme, "http" | "https"))
    {
        let (authority, path) = rest.split_once('/')?;
        let authority = strip_userinfo(authority);
        if !safe_authority(authority) {
            return None;
        }
        return Some(SelectedRemote::Web(WebRemote {
            base: format!("{scheme}://{authority}"),
            repo: uri_repo(path)?,
        }));
    }
    if url.contains("://") {
        // git://, file://, svn+ssh:// … — no web home to link to.
        return None;
    }
    // scp-like syntax: [user@]host:repo/path — the colon separates.
    let (owner, path) = url.split_once(':')?;
    selected(owner, repo_path(path)?)
}

/// Pick the permalink remote: upstream > origin > first remaining
/// (0001 pillar 3.3). The first remote that parses wins; a failure of
/// the winner surfaces at the caller — never a silent substitute URL.
pub fn pick_remote(remotes: &[(String, String)]) -> Option<SelectedRemote> {
    for name in ["upstream", "origin"] {
        if let Some(url) = remotes.iter().find(|(n, _)| n == name).map(|(_, u)| u) {
            if let Some(remote) = parse_remote(url) {
                return Some(remote);
            }
        }
    }
    remotes.iter().find_map(|(_, url)| parse_remote(url))
}

/// The immutable permalink (0001 pillar 3.3): revision pinned to a
/// commit SHA the caller already resolved, repository and file path
/// segments percent-encoded from native bytes, GitHub/GitLab `#L` line
/// anchors.
pub fn permalink(remote: &WebRemote, sha: &str, path: &Path, lines: (usize, usize)) -> String {
    let frag = if lines.0 == lines.1 {
        format!("#L{}", lines.0)
    } else {
        format!("#L{}-L{}", lines.0, lines.1)
    };
    format!(
        "{}/{}/blob/{}/{}{frag}",
        remote.base,
        encode_repo_path(&remote.repo),
        sha,
        encode_path(path),
    )
}

/// Hostname/alias shape: ASCII letters, digits, `.`, `_`, `-` only,
/// never option-shaped (no leading `-`). The same text later rides an
/// argv and a URL — both stay inert by construction.
pub(crate) fn is_safe_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Userinfo off an authority: `git@host` → `host`. Credentials have no
/// business in a permalink.
fn strip_userinfo(authority: &str) -> &str {
    authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
}

/// An HTTP(S) authority quoted verbatim into a link: host, or
fn safe_authority(authority: &str) -> bool {
    let mut parts = authority.split(':');
    let host = parts.next().unwrap_or_default();
    match (parts.next(), parts.next()) {
        (None, _) => is_safe_host(host),
        (Some(port), None) => is_safe_host(host) && port.parse::<std::num::NonZeroU16>().is_ok(),
        // more than one colon is not a plain authority
        (Some(_), Some(_)) => false,
    }
}

/// Host part of an SSH authority: one optional trailing numeric port,
/// which is dropped — the SSH port is not the web port. Bracketed IPv6
/// and other exotic authorities are refused: an honest `None` beats a
/// mangled link.
fn ssh_host(authority: &str) -> Option<&str> {
    let host = match authority.split_once(':') {
        None => authority,
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        Some(_) => return None,
    };
    is_safe_host(host).then_some(host)
}

/// Retain transport identity for OpenSSH's effective configuration.
fn selected(authority: &str, repo: String) -> Option<SelectedRemote> {
    let (user, authority) = match authority.rsplit_once('@') {
        Some((user, authority)) if is_safe_host(user) => (Some(user.to_owned()), authority),
        Some(_) => return None,
        None => (None, authority),
    };
    let host = ssh_host(authority)?;
    let port = match authority.split_once(':') {
        Some((_, port)) => Some(port.parse::<std::num::NonZeroU16>().ok()?),
        None => None,
    };
    Some(SelectedRemote::Alias(AliasRemote {
        alias: host.to_owned(),
        repo,
        user,
        port,
    }))
}

/// Repository path under a host: leading separators dropped, exactly
/// one trailing `.git` stripped (git's own canonical form — a repo
/// really named `demo.git.git` stays `demo.git`). Empty is no
/// repository, not a link to the site root.
fn repo_path(path: &str) -> Option<String> {
    let path = path.trim_start_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    (!path.is_empty()).then(|| path.to_string())
}

/// URL paths are already escaped; decode once before the shared segment encoder.
/// scp syntax stays byte-literal and does not pass through this boundary.
fn uri_repo(path: &str) -> Option<String> {
    if path.contains(['?', '#']) {
        return None;
    }
    let source = path.trim_start_matches('/').as_bytes();
    let mut decoded = Vec::with_capacity(source.len());
    let mut offset = 0;
    while offset < source.len() {
        if source[offset] == b'%' {
            let digits = std::str::from_utf8(source.get(offset + 1..offset + 3)?).ok()?;
            decoded.push(u8::from_str_radix(digits, 16).ok()?);
            offset += 3;
        } else {
            decoded.push(source[offset]);
            offset += 1;
        }
    }
    let mut path = String::from_utf8(decoded).ok()?;
    if path.ends_with(".git") {
        path.truncate(path.len() - 4);
    }
    (!path.is_empty() && !path.contains('\0')).then_some(path)
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Percent-encode one path segment per RFC 3986: unreserved bytes pass
/// through, everything else escapes — spaces, Unicode and the
/// non-UTF8 bytes Unix paths carry alike. Separators are re-added by
/// the caller, so no segment can smuggle structure.
fn encode_segment(segment: &[u8]) -> String {
    let mut out = String::with_capacity(segment.len());
    for &byte in segment {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
    out
}

fn segment_bytes(segment: &OsStr) -> Cow<'_, [u8]> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Cow::Borrowed(segment.as_bytes())
    }
    #[cfg(not(unix))]
    {
        Cow::Owned(segment.to_string_lossy().into_owned().into_bytes())
    }
}

/// The location's repo-relative path as URL path segments: native
/// bytes in — never a lossy spelling — encoded segments out.
fn encode_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(segment) => Some(encode_segment(&segment_bytes(segment))),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// A remote's repository path (`"acme/nested/demo"`), encoded segment
/// by segment so spaces and Unicode survive as themselves.
fn encode_repo_path(repo: &str) -> String {
    repo.split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| encode_segment(segment.as_bytes()))
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web(url: &str) -> (String, String) {
        match parse_remote(url) {
            Some(SelectedRemote::Web(remote)) => (remote.base, remote.repo),
            Some(SelectedRemote::Alias(remote)) => {
                let web = remote
                    .resolved(remote.host())
                    .expect("literal FQDN in fixture");
                (web.base, web.repo)
            }
            other => panic!("{url} did not parse to a web remote: {other:?}"),
        }
    }

    fn alias(url: &str) -> (String, String) {
        match parse_remote(url) {
            Some(SelectedRemote::Alias(remote)) => (remote.alias, remote.repo),
            other => panic!("{url} did not parse to an alias remote: {other:?}"),
        }
    }

    /// The reviewer's defect (0033 finding 1): the full FQDN authority
    /// and the whole repository path — nested or not — must survive.
    #[test]
    fn https_authority_and_nested_repo_path_survive() {
        assert_eq!(
            web("https://bbgithub.dev.bloomberg.com/acme/demo.git"),
            (
                "https://bbgithub.dev.bloomberg.com".to_string(),
                "acme/demo".to_string()
            )
        );
        assert_eq!(
            web("https://bbgithub.dev.bloomberg.com/acme/nested/demo.git"),
            (
                "https://bbgithub.dev.bloomberg.com".to_string(),
                "acme/nested/demo".to_string()
            )
        );
        // Explicit schemes and ports remain part of the destination.
        assert_eq!(
            web("https://gitea.internal:3000/team/sub/project.git"),
            (
                "https://gitea.internal:3000".to_string(),
                "team/sub/project".to_string()
            )
        );
        assert_eq!(
            web("http://gitea.internal/team/proj"),
            ("http://gitea.internal".to_string(), "team/proj".to_string())
        );
        // credentials never reach a link
        assert_eq!(
            web("https://oauth2:tok@gitlab.example.com/group/proj.git"),
            (
                "https://gitlab.example.com".to_string(),
                "group/proj".to_string()
            )
        );
    }

    /// The reviewer table, verbatim, with the exact identity each form
    /// must produce.
    #[test]
    fn reviewer_table_identities() {
        assert_eq!(
            web("https://github.com/acme/demo.git"),
            ("https://github.com".to_string(), "acme/demo".to_string())
        );
        assert_eq!(
            web("ssh://git@github.com/acme/demo.git"),
            ("https://github.com".to_string(), "acme/demo".to_string())
        );
        assert_eq!(
            web("git@github.com:acme/demo"),
            ("https://github.com".to_string(), "acme/demo".to_string())
        );
        assert_eq!(
            web("git@bbgithub.dev.bloomberg.com:acme/demo.git"),
            (
                "https://bbgithub.dev.bloomberg.com".to_string(),
                "acme/demo".to_string()
            )
        );
        assert_eq!(
            web("https://bbgithub.dev.bloomberg.com/acme/demo.git"),
            (
                "https://bbgithub.dev.bloomberg.com".to_string(),
                "acme/demo".to_string()
            )
        );
        // the SSH port is not the web port
        assert_eq!(
            web("ssh://git@gitlab.example.com:2222/team/repo.git"),
            (
                "https://gitlab.example.com".to_string(),
                "team/repo".to_string()
            )
        );
    }

    /// A dotless SSH host is an alias: unresolved data, never a bare
    /// guess from a config parser that never ran.
    #[test]
    fn dotless_ssh_hosts_are_unresolved_aliases() {
        assert_eq!(
            alias("bbgithub:acme/demo.git"),
            ("bbgithub".to_string(), "acme/demo".to_string())
        );
        assert_eq!(
            alias("git@bbgithub:acme/demo.git"),
            ("bbgithub".to_string(), "acme/demo".to_string())
        );
        assert_eq!(
            alias("ssh://git@bbgithub/acme/demo.git"),
            ("bbgithub".to_string(), "acme/demo".to_string())
        );
        assert_eq!(
            alias("ssh://bb:2222/team/repo.git"),
            ("bb".to_string(), "team/repo".to_string())
        );
        // the resolved fold is pure: effective hostname in, web remote out
        let resolved = AliasRemote {
            alias: "bbgithub".to_string(),
            repo: "acme/demo".to_string(),
            user: None,
            port: None,
        }
        .resolved("bbgithub.dev.bloomberg.com")
        .unwrap();
        assert_eq!(resolved.base, "https://bbgithub.dev.bloomberg.com");
        assert_eq!(resolved.repo, "acme/demo");
    }

    /// Refusals: local paths, other schemes, option-shaped text and
    /// empty repositories produce `None`, not a dead URL.
    #[test]
    fn unsupported_urls_refuse_instead_of_guessing() {
        for url in [
            "not a url",
            "/srv/git/repo.git",
            "../repo",
            "file:///srv/repo.git",
            "git://github.com/acme/demo.git",
            "svn+ssh://host/team/repo",
            "https://host/",
            "https://host",
            "git@host:",
            "-oProxyCommand=evil:org/repo",
            "git@-flag:org/repo",
            "ssh://git@[::1]/repo",
            "ssh://git@host:notaport/repo",
            "ssh://git@host",
        ] {
            assert!(parse_remote(url).is_none(), "should refuse: {url}");
        }
    }

    /// `.git` is stripped once — a repository genuinely named
    /// `demo.git.git` keeps its identity as `demo.git`.
    #[test]
    fn git_suffix_strips_once() {
        assert_eq!(
            web("https://host/acme/demo.git.git"),
            ("https://host".to_string(), "acme/demo.git".to_string())
        );
    }

    /// Priority is upstream > origin > first remaining, and the winner
    /// is chosen once: an alias stays an alias, never a substitute.
    #[test]
    fn pick_remote_prefers_upstream_then_origin() {
        let remote = |name: &str, url: &str| (name.to_string(), url.to_string());
        let picked = |remotes: &[(String, String)]| pick_remote(remotes);
        assert_eq!(
            picked(&[
                remote("origin", "https://gitlab.com/o/r.git"),
                remote("upstream", "https://github.com/a/b.git"),
            ]),
            Some(SelectedRemote::Web(WebRemote {
                base: "https://github.com".to_string(),
                repo: "a/b".to_string(),
            }))
        );
        // an unparseable upstream falls through to origin
        assert!(matches!(
            picked(&[
                remote("upstream", "/local/x"),
                remote("origin", "git@github.com:o/r.git"),
            ]),
            Some(SelectedRemote::Alias(_))
        ));
        // the winner's shape is kept: an alias upstream is not replaced
        // by origin's ready URL
        assert!(matches!(
            picked(&[
                remote("origin", "https://gitlab.com/o/r.git"),
                remote("upstream", "git@bb:acme/demo.git"),
            ]),
            Some(SelectedRemote::Alias(_))
        ));
        assert!(picked(&[remote("origin", "/local/x")]).is_none());
        assert!(matches!(
            picked(&[remote("other", "https://gitlab.com/o/r.git")]),
            Some(SelectedRemote::Web(_))
        ));
    }

    /// The built link: SHA-pinned, percent-encoded segments, `#L`
    /// anchors.
    #[test]
    fn permalink_pins_sha_and_encodes_segments() {
        let github = WebRemote {
            base: "https://github.com".to_string(),
            repo: "stropdev/strop".to_string(),
        };
        assert_eq!(
            permalink(&github, "abc123", Path::new("f.rs"), (2, 2)),
            "https://github.com/stropdev/strop/blob/abc123/f.rs#L2"
        );
        assert_eq!(
            permalink(&github, "abc123", Path::new("src/lib.rs"), (1, 3)),
            "https://github.com/stropdev/strop/blob/abc123/src/lib.rs#L1-L3"
        );
        // spaces, Unicode and repository segments encode; separators stay
        assert_eq!(
            permalink(&github, "abc123", Path::new("src/sp ace/日本語.rs"), (1, 1)),
            "https://github.com/stropdev/strop/blob/abc123/src/sp%20ace/%E6%97%A5%E6%9C%AC%E8%AA%9E.rs#L1"
        );
        let spaced = WebRemote {
            base: "https://host".to_string(),
            repo: "my repo/x".to_string(),
        };
        assert_eq!(
            permalink(&spaced, "abc", Path::new("f.rs"), (1, 1)),
            "https://host/my%20repo/x/blob/abc/f.rs#L1"
        );
    }

    /// Unix filenames may be non-UTF8: their bytes percent-encode, no
    /// lossy spelling ever reaches the link.
    #[cfg(unix)]
    #[test]
    fn permalink_percent_encodes_non_utf8_path_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let github = WebRemote {
            base: "https://github.com".to_string(),
            repo: "stropdev/strop".to_string(),
        };
        let path = Path::new(std::ffi::OsStr::from_bytes(b"src/\xff\xfe.rs"));
        assert_eq!(
            permalink(&github, "abc", path, (1, 1)),
            "https://github.com/stropdev/strop/blob/abc/src/%FF%FE.rs#L1"
        );
    }
}
