//! `git show --raw --numstat -z` machine format: raw bytes in,
//! `ChangedFile` rows out.
//!
//! Numstat's default format C-quotes "unusual" pathnames
//! (`core.quotePath`), so `src/日本語.rs` arrives as
//! `"src/\346\227\245..."` — a display spelling, not a filesystem
//! identity. Under `-z` git writes paths verbatim, NUL-terminated. The
//! raw section comes first and names what happened to each path; the
//! numstat section follows with the line counts and is what lists the
//! rows (observed on git 2.43; the shapes are long stable):
//!
//! ```text
//! :<mode> <mode> <sha> <sha> <status>\0<path>\0           raw
//! :<mode> <mode> <sha> <sha> R<score>\0<old>\0<new>\0     raw rename or copy
//! ::<mode>… <sha>… <statuses>\0<path>\0                   raw, a merge (combined)
//! <added>\t<deleted>\t<path>\0          ordinary
//! -\t-\t<path>\0                       binary (line counts don't apply)
//! <added>\t<deleted>\t\0<old>\0<new>\0  rename; the row keeps the destination
//! ```

use std::collections::HashMap;
use std::path::Path;

use crate::memory::{ChangedFile, FileStatus};

/// Parse a whole `--raw --numstat -z` stream. Malformed output is an
/// error: a dropped row here would silently hide a file from the dive
/// view, and a misaligned parse could promote garbage to a path identity.
pub(crate) fn parse_show(buf: &[u8]) -> Result<Vec<ChangedFile>, String> {
    let (statuses, numstat) = parse_raw(buf)?;
    let mut rows = parse_numstat(numstat)?;
    for row in &mut rows {
        if let Some(status) = statuses.get(row.path.as_path()) {
            row.status = *status;
        }
    }
    Ok(rows)
}

/// The raw records at the head of the stream, by destination path; and
/// the rest of the stream.
fn parse_raw(mut rest: &[u8]) -> Result<(HashMap<&Path, FileStatus>, &[u8]), String> {
    let mut statuses = HashMap::new();
    while rest.first() == Some(&b':') {
        let (header, tail) = split(rest, 0, "raw record")?;
        // a merge's combined record has one status letter per parent;
        // the file's status is the first parent's
        let letters = header
            .rsplit(|byte| *byte == b' ')
            .next()
            .ok_or("git raw: malformed record")?;
        let status = FileStatus::from_letter(*letters.first().ok_or("git raw: no status")?);
        let (path, tail) = if matches!(status, FileStatus::Renamed | FileStatus::Copied) {
            let (_old, tail) = path_field(tail)?;
            path_field(tail)?
        } else {
            path_field(tail)?
        };
        statuses.insert(path, status);
        rest = tail;
    }
    Ok((statuses, rest))
}

/// Parse a whole `--numstat -z` stream.
pub(crate) fn parse_numstat(buf: &[u8]) -> Result<Vec<ChangedFile>, String> {
    let mut rows = Vec::new();
    let mut rest = buf;
    while !rest.is_empty() {
        let (counts, tail) = counts(rest)?;
        // A NUL where a path would start is the rename marker; a path
        // can never begin with one (it is the field terminator).
        let (old_path, path, tail) = if tail.first() == Some(&0) {
            let (old, tail) = path_field(&tail[1..])?;
            let (path, tail) = path_field(tail)?;
            (Some(old.to_owned()), path, tail)
        } else {
            let (path, tail) = path_field(tail)?;
            (None, path, tail)
        };
        let (added, deleted) = counts.unwrap_or((0, 0));
        rows.push(ChangedFile {
            path: path.to_owned(),
            added,
            deleted,
            status: if old_path.is_some() {
                FileStatus::Renamed
            } else {
                FileStatus::Modified
            },
            old_path,
            binary: counts.is_none(),
        });
        rest = tail;
    }
    Ok(rows)
}

/// Added and deleted lines; None for a binary file.
type Counts = Option<(usize, usize)>;

/// `<added>\t<deleted>\t` header of one record: the two counts, or None
/// for a binary file (`-\t-`), where line counts do not apply. A binary
/// row keeps 0/0 counts so it stays visible in the file list.
fn counts(buf: &[u8]) -> Result<(Counts, &[u8]), String> {
    let (added, buf) = split(buf, b'\t', "added count")?;
    let (deleted, buf) = split(buf, b'\t', "deleted count")?;
    if added == b"-" && deleted == b"-" {
        return Ok((None, buf));
    }
    Ok((
        Some((count(added, "added")?, count(deleted, "deleted")?)),
        buf,
    ))
}

/// One numstat count: a decimal line total.
fn count(field: &[u8], what: &str) -> Result<usize, String> {
    std::str::from_utf8(field)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("git numstat: malformed {what} count"))
}

/// Split at the first `sep` byte; the tail excludes it.
fn split<'a>(buf: &'a [u8], sep: u8, what: &str) -> Result<(&'a [u8], &'a [u8]), String> {
    buf.iter()
        .position(|&b| b == sep)
        .map(|i| (&buf[..i], &buf[i + 1..]))
        .ok_or_else(|| format!("git numstat: unterminated {what}"))
}

/// One borrowed native path field. Rename sources can be validated without
/// allocating a pathname that will be discarded. Non-Unix Git emits UTF-8;
/// malformed encoding is an error, never a lossy filesystem identity.
#[cfg(unix)]
fn path_field(buf: &[u8]) -> Result<(&Path, &[u8]), String> {
    use std::os::unix::ffi::OsStrExt;
    let (field, tail) = split(buf, 0, "path")?;
    if field.is_empty() {
        return Err("git numstat: empty path".into());
    }
    Ok((Path::new(std::ffi::OsStr::from_bytes(field)), tail))
}

#[cfg(not(unix))]
fn path_field(buf: &[u8]) -> Result<(&Path, &[u8]), String> {
    let (field, tail) = split(buf, 0, "path")?;
    if field.is_empty() {
        return Err("git numstat: empty path".into());
    }
    let text = std::str::from_utf8(field)
        .map_err(|_| "git numstat: path is not valid UTF-8".to_string())?;
    Ok((Path::new(text), tail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Verbatim `git show --numstat -z --format=` bytes from a real
    /// repository (git 2.43): ordinary, binary, rename, and paths
    /// containing newline/tab — all raw, none quoted.
    #[test]
    fn parses_real_git_bytes() {
        let buf = concat_records();
        let rows = parse_numstat(&buf).unwrap();
        assert_eq!(rows.len(), 5, "{rows:?}");

        assert_eq!(rows[0].path, Path::new("a.txt"));
        assert_eq!((rows[0].added, rows[0].deleted), (2, 0));

        // binary: kept as a row, counts folded to 0/0
        assert_eq!(rows[1].path, Path::new("bin.dat"));
        assert_eq!((rows[1].added, rows[1].deleted), (0, 0));

        // rename: the destination is the identity; the old name never
        // becomes a row
        assert_eq!(rows[2].path, Path::new("new.txt"));
        assert_eq!((rows[2].added, rows[2].deleted), (0, 0));

        // control characters stay inside the path, raw
        assert_eq!(rows[3].path, Path::new("nl\nname.txt"));
        assert_eq!(rows[4].path, Path::new("tab\tname.txt"));
        assert!(rows[3].added == 1 && rows[4].added == 1);
    }

    fn concat_records() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"2\t0\ta.txt\x00");
        buf.extend_from_slice(b"-\t-\tbin.dat\x00");
        buf.extend_from_slice(b"0\t0\t\x00ren.txt\x00new.txt\x00");
        buf.extend_from_slice(b"1\t0\tnl\nname.txt\x00");
        buf.extend_from_slice(b"1\t0\ttab\tname.txt\x00");
        buf
    }
    #[test]
    fn unicode_path_stays_unquoted() {
        let rows = parse_numstat(b"3\t1\tsrc/\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e.rs\x00").unwrap();
        // equality with the native spelling is the point: the quoted
        // form `"src/\346..."` would not match
        assert_eq!(rows[0].path, Path::new("src/日本語.rs"));
        assert_eq!((rows[0].added, rows[0].deleted), (3, 1));
    }

    /// Unix filenames may be non-UTF8; they must arrive byte-for-byte.
    #[cfg(unix)]
    #[test]
    fn non_utf8_path_arrives_as_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let rows = parse_numstat(b"1\t0\t\xff\xfe.rs\x00").unwrap();
        assert_eq!(rows[0].path.as_os_str().as_bytes(), b"\xff\xfe.rs");
    }

    #[test]
    fn binary_rename_keeps_destination() {
        let rows = parse_numstat(b"-\t-\t\x00old.bin\x00new.bin\x00").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, Path::new("new.bin"));
        assert_eq!((rows[0].added, rows[0].deleted), (0, 0));
    }

    /// The raw section names what happened; numstat lists the rows.
    #[test]
    fn raw_records_give_each_row_its_status() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b":100644 100644 aaaaaaa bbbbbbb M\x00a.txt\x00");
        buf.extend_from_slice(b":000000 100644 0000000 bbbbbbb A\x00bin.dat\x00");
        buf.extend_from_slice(b":100644 100644 aaaaaaa aaaaaaa R100\x00ren.txt\x00new.txt\x00");
        buf.extend_from_slice(b":100644 100644 aaaaaaa ccccccc C075\x00a.txt\x00copy.txt\x00");
        buf.extend_from_slice(b"::100644 100644 100644 a b c MM\x00both.txt\x00");
        buf.extend_from_slice(b"2\t0\ta.txt\x00");
        buf.extend_from_slice(b"-\t-\tbin.dat\x00");
        buf.extend_from_slice(b"0\t0\t\x00ren.txt\x00new.txt\x00");
        buf.extend_from_slice(b"1\t0\t\x00a.txt\x00copy.txt\x00");
        buf.extend_from_slice(b"1\t1\tboth.txt\x00");
        let rows = parse_show(&buf).unwrap();
        let summary: Vec<(&Path, char, bool)> = rows
            .iter()
            .map(|row| (row.path.as_path(), row.status.letter(), row.binary))
            .collect();
        assert_eq!(
            summary,
            [
                (Path::new("a.txt"), 'M', false),
                (Path::new("bin.dat"), 'A', true),
                (Path::new("new.txt"), 'R', false),
                (Path::new("copy.txt"), 'C', false),
                (Path::new("both.txt"), 'M', false),
            ]
        );
        assert_eq!(rows[2].old_path.as_deref(), Some(Path::new("ren.txt")));
        assert_eq!(rows[3].old_path.as_deref(), Some(Path::new("a.txt")));
        assert_eq!(rows[0].old_path, None);
        // a raw record cut short is an error, never a dropped row
        assert!(parse_show(b":100644 100644 a b M").is_err());
    }

    #[test]
    fn empty_stream_is_no_rows() {
        assert!(parse_numstat(b"").unwrap().is_empty());
    }

    #[test]
    fn malformed_output_is_an_error() {
        // truncated: no terminating NUL on the path
        assert!(parse_numstat(b"1\t0\ta.rs").is_err());
        // counts are not numbers
        assert!(parse_numstat(b"x\t0\ta.rs\x00").is_err());
        assert!(parse_numstat(b"-\t-\x00a.rs\x00").is_err());
        // trailing garbage is a broken record, not a dropped row
        assert!(parse_numstat(b"1\t0\ta.rs\x00garbage").is_err());
        // rename missing its destination, or with empty fields
        assert!(parse_numstat(b"1\t0\t\x00old\x00").is_err());
        assert!(parse_numstat(b"1\t0\t\x00\x00new\x00").is_err());
        // missing field separators
        assert!(parse_numstat(b"1 0\ta.rs\x00").is_err());
    }
}
