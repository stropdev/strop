//! The pure wire-protocol parsers of the remote Git backend, split
//! from `remote/mod.rs` by concern: every machine-format boundary
//! (`ls-tree`/`ls-files` records, `rev-parse` output, `config -z`
//! remotes, `rev-list --parents`) is a pure parser fed by bytes.

use std::path::PathBuf;

use super::RemoteGitError;

// ---- pure wire parsers ---------------------------------------------------
//
// Each parser is fed by bytes captured from real `git` (see tests):
// native filename bytes survive, exit-code meanings are documented at
// their call sites, and nothing is guessed from stderr text.

/// `git rev-parse --show-toplevel`: one absolute native path with a
/// trailing newline. Relative or empty output is refused — a worktree
/// root on the endpoint is absolute by definition.
pub(super) fn parse_toplevel(bytes: &[u8]) -> Result<PathBuf, RemoteGitError> {
    let trimmed = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    if trimmed.is_empty() || trimmed.first() != Some(&b'/') {
        return Err(RemoteGitError::Parse("rev-parse --show-toplevel"));
    }
    Ok(bytes_to_path(trimmed))
}

/// A full object name: 40–64 lowercase-or-uppercase hex characters.
pub(super) fn parse_sha(bytes: &[u8], what: &'static str) -> Result<String, RemoteGitError> {
    let text = String::from_utf8_lossy(bytes);
    let sha = text.trim();
    let valid = (40..=64).contains(&sha.len())
        && !sha.is_empty()
        && sha.bytes().all(|b| b.is_ascii_hexdigit());
    if !valid {
        return Err(RemoteGitError::Parse(what));
    }
    Ok(sha.to_string())
}

/// `git config -z --get-regexp '^remote\.[^.]+\.url$'` records:
/// `remote.<name>.url\n<url>\0`. Names and URLs are config strings;
/// non-UTF-8 values are refused rather than lossily renamed.
pub(super) fn parse_remote_config(bytes: &[u8]) -> Result<Vec<(String, String)>, RemoteGitError> {
    let mut remotes = Vec::new();
    for record in bytes.split(|&b| b == 0) {
        if record.is_empty() {
            continue;
        }
        let Some((key, url)) = split_record(record, b'\n') else {
            return Err(RemoteGitError::Parse("config --get-regexp remote.*.url"));
        };
        let key = std::str::from_utf8(key).map_err(|_| RemoteGitError::Utf8("remote name"))?;
        let url = std::str::from_utf8(url).map_err(|_| RemoteGitError::Utf8("remote url"))?;
        let Some(name) = key
            .strip_prefix("remote.")
            .and_then(|rest| rest.strip_suffix(".url"))
        else {
            return Err(RemoteGitError::Parse("config --get-regexp remote.*.url"));
        };
        if name.is_empty() {
            return Err(RemoteGitError::Parse("config --get-regexp remote.*.url"));
        }
        remotes.push((name.to_string(), url.to_owned()));
    }
    Ok(remotes)
}

/// One `ls-tree -z` record: `mode SP type SP oid TAB path`. `None` for
/// an empty record set (the path is absent); the oid is validated hex
/// before anything embeds it in a later argv.
pub(super) fn parse_tree_entry(bytes: &[u8]) -> Result<Option<(String, PathBuf)>, RemoteGitError> {
    let Some(record) = first_record(bytes) else {
        return Ok(None);
    };
    let Some((meta, path)) = split_record(record, b'\t') else {
        return Err(RemoteGitError::Parse("ls-tree"));
    };
    let mut fields = meta.split(|&b| b == b' ');
    let oid = fields.nth(2).ok_or(RemoteGitError::Parse("ls-tree"))?;
    let oid = parse_sha(oid, "ls-tree oid")?;
    Ok(Some((oid, bytes_to_path(path))))
}

/// One `ls-files -z --stage` record: `mode SP oid SP stage TAB path`.
pub(super) fn parse_index_entry(bytes: &[u8]) -> Result<Option<(String, PathBuf)>, RemoteGitError> {
    let Some(record) = first_record(bytes) else {
        return Ok(None);
    };
    let Some((meta, path)) = split_record(record, b'\t') else {
        return Err(RemoteGitError::Parse("ls-files --stage"));
    };
    let mut fields = meta.split(|&b| b == b' ');
    let oid = fields
        .nth(1)
        .ok_or(RemoteGitError::Parse("ls-files --stage"))?;
    let oid = parse_sha(oid, "ls-files oid")?;
    Ok(Some((oid, bytes_to_path(path))))
}

/// `git rev-list --parents -n 1 <sha>`: `child [parent…]`, all full
/// object names. The first parent is the delta base (merges diff
/// against parent(0), matching local behavior).
pub(super) fn parse_commit_parents(
    bytes: &[u8],
) -> Result<(String, Option<String>), RemoteGitError> {
    let text = String::from_utf8_lossy(bytes);
    let mut shas = text
        .split_whitespace()
        .map(|token| parse_sha(token.as_bytes(), "rev-list --parents"));
    let commit = shas
        .next()
        .ok_or(RemoteGitError::Parse("rev-list --parents"))??;
    let parent = shas.next().transpose()?;
    Ok((commit, parent))
}

fn first_record(bytes: &[u8]) -> Option<&[u8]> {
    bytes.split(|&b| b == 0).find(|record| !record.is_empty())
}

fn split_record(bytes: &[u8], separator: u8) -> Option<(&[u8], &[u8])> {
    let index = bytes.iter().position(|&byte| byte == separator)?;
    Some((&bytes[..index], &bytes[index + 1..]))
}

#[cfg(unix)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bytes below are captured from real `git` output (see
    // crates/strop-git/src/remote/mod.rs doc comment); the regressions
    // pin the exact shapes the remote backend admits.

    #[test]
    fn toplevel_is_absolute_native_path() {
        assert_eq!(
            parse_toplevel(b"/srv/proj with space\n").unwrap(),
            PathBuf::from("/srv/proj with space")
        );
        assert!(
            parse_toplevel(b"srv/proj\n").is_err(),
            "relative is refused"
        );
        assert!(parse_toplevel(b"").is_err(), "empty is refused");
    }

    #[test]
    fn shas_are_validated_object_names() {
        assert_eq!(
            parse_sha(b"c59d8ceb7aeb96a1cdccff5646ec485acce32d45\n", "x").unwrap(),
            "c59d8ceb7aeb96a1cdccff5646ec485acce32d45"
        );
        assert!(parse_sha(b"main\n", "x").is_err());
        assert!(parse_sha(b"head is at 1234\n", "x").is_err());
        assert!(parse_sha(b"\n", "x").is_err());
    }

    /// config -z records split key\nvalue on NUL; a remote with no
    /// remotes is an empty vector, and a non-UTF-8 URL is refused.
    #[test]
    fn remote_config_records_parse_native() {
        let bytes = b"remote.origin.url\nhttps://example.com/acme/demo.git\0remote.up.url\ngit@gh:acme/other.git\0";
        assert_eq!(
            parse_remote_config(bytes).unwrap(),
            vec![
                (
                    "origin".to_string(),
                    "https://example.com/acme/demo.git".to_string()
                ),
                ("up".to_string(), "git@gh:acme/other.git".to_string()),
            ]
        );
        assert_eq!(
            parse_remote_config(b"").unwrap(),
            Vec::<(String, String)>::new()
        );
        assert!(parse_remote_config(b"not-a-pair\0").is_err());
        assert!(
            parse_remote_config(b"remote..url\nx\0").is_err(),
            "empty name"
        );
        assert!(
            parse_remote_config(b"remote.o.url\nhttps://a/\xff\xfe\0").is_err(),
            "non-UTF-8 url refused"
        );
    }

    /// ls-tree/ls-files -z records keep native path bytes — spaces,
    /// quotes and non-UTF-8 names arrive as the worktree identity —
    /// and an empty record set is the honest absence.
    #[test]
    fn tree_and_index_records_keep_native_paths() {
        let tree = b"100644 blob 45b983be36b73c0788dc9cbcb76cbb80fc7bb057\tsrc/a b.rs\0";
        let (oid, path) = parse_tree_entry(tree).unwrap().unwrap();
        assert_eq!(oid, "45b983be36b73c0788dc9cbcb76cbb80fc7bb057");
        assert_eq!(path, PathBuf::from("src/a b.rs"));
        assert_eq!(parse_tree_entry(b"").unwrap(), None);

        let index = b"100644 45b983be36b73c0788dc9cbcb76cbb80fc7bb057 0\tsrc/a b.rs\0";
        let (oid, path) = parse_index_entry(index).unwrap().unwrap();
        assert_eq!(oid, "45b983be36b73c0788dc9cbcb76cbb80fc7bb057");
        assert_eq!(path, PathBuf::from("src/a b.rs"));
        assert_eq!(parse_index_entry(b"").unwrap(), None);
        assert!(parse_tree_entry(b"garbage\0").is_err());
        assert!(parse_index_entry(b"100644 noshahere 0\tx\0").is_err());
    }

    /// rev-list --parents: child first, then parents; a root commit has
    /// none, and both shapes parse to (commit, Option<parent>).
    #[test]
    fn commit_parents_root_and_merged() {
        let root = b"60209d7ce72dddfafc1caacd511325c314a083bf\n";
        assert_eq!(
            parse_commit_parents(root).unwrap(),
            ("60209d7ce72dddfafc1caacd511325c314a083bf".to_string(), None)
        );
        let merged = b"c59d8ceb7aeb96a1cdccff5646ec485acce32d45 aaaa1111111111111111111111111111111111111 bbbb2222222222222222222222222222222222222\n";
        let (child, parent) = parse_commit_parents(merged).unwrap();
        assert_eq!(child, "c59d8ceb7aeb96a1cdccff5646ec485acce32d45");
        assert_eq!(
            parent.as_deref(),
            Some("aaaa1111111111111111111111111111111111111")
        );
        assert!(parse_commit_parents(b"not-a-sha\n").is_err());
        assert!(parse_commit_parents(b"").is_err());
    }
}
