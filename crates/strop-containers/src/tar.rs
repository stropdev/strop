//! A strict, minimal tar header reader for `docker cp … -` archives.
//!
//! Supports exactly what the engine emits: ustar headers with the prefix
//! field, GNU longname (`L`) records, and pax per-file (`x`) `path`
//! overrides. Entry content is never copied — entries borrow offset ranges
//! into the captured archive. Structural violations are errors, never
//! skipped-and-hoped: a listing parsed from a corrupt stream would be a
//! lie. Base-256 numeric fields (sizes past the octal range) are refused;
//! `docker cp` of a browsable tree does not produce them.

use std::ops::Range;

const BLOCK: usize = 512;

/// The entry's kind, from the tar typeflag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TarKind {
    File,
    Dir,
    Symlink,
    /// Hard links, devices, fifos — anything this backend does not
    /// distinguish further.
    Other,
}

/// One archive entry: name, kind, declared size and where its content
/// sits in the captured archive (clamped to what was captured).
#[derive(Debug)]
pub(crate) struct TarEntry {
    pub name: String,
    pub kind: TarKind,
    pub size: u64,
    pub data: Range<usize>,
    /// The linkname field, kept for symlinks (`docker cp` does not
    /// resolve a symlinked source path — callers resolve one hop).
    pub link_target: Option<String>,
}

/// Parse every header in `bytes`. When `complete` is false the capture
/// was truncated by the output bound: a final entry whose *content* is
/// cut short is tolerated (its `data` range is clamped — callers reading
/// with an explicit `max` rely on this), but a cut-off *header* or any
/// structural flaw is still an error.
pub(crate) fn parse(bytes: &[u8], complete: bool) -> Result<Vec<TarEntry>, String> {
    let mut entries = Vec::new();
    let mut pending_name: Option<String> = None;
    let mut offset = 0;
    loop {
        let Some(header) = bytes.get(offset..offset + BLOCK) else {
            let rest = &bytes[offset.min(bytes.len())..];
            if complete && rest.iter().any(|&b| b != 0) {
                return Err("truncated tar header".into());
            }
            break;
        };
        if header.iter().all(|&b| b == 0) {
            break; // end-of-archive marker
        }
        if &header[257..262] != b"ustar" {
            return Err("not a ustar archive".into());
        }
        let size = octal(&header[124..136])?;
        let typeflag = header[156];
        let data_start = offset + BLOCK;
        let padded = (size as usize).saturating_add(BLOCK - 1) / BLOCK * BLOCK;
        // `data` covers the exact content (never the block padding); the
        // stream position still advances by the padded size.
        let data_end = data_start.saturating_add(size as usize).min(bytes.len());
        let block_end = data_start.saturating_add(padded).min(bytes.len());
        let short_content = block_end - data_start < padded;
        if short_content && complete {
            return Err("truncated tar content".into());
        }
        let content = &bytes[data_start..data_end];
        match typeflag {
            b'L' => pending_name = Some(nul_terminated(content)?),
            b'x' => {
                if let Some(path) = pax_path(content)? {
                    pending_name = Some(path);
                }
            }
            b'K' | b'g' => {} // longlink / global pax: nothing we consume
            flag => {
                let name = match pending_name.take() {
                    Some(name) => name,
                    None => header_name(header)?,
                };
                let kind = match flag {
                    b'0' | 0 => TarKind::File,
                    b'5' => TarKind::Dir,
                    b'2' => TarKind::Symlink,
                    _ => TarKind::Other,
                };
                let link_target = (kind == TarKind::Symlink)
                    .then(|| nul_terminated(&header[157..257]))
                    .transpose()?;
                entries.push(TarEntry {
                    name,
                    kind,
                    size,
                    data: data_start..data_end,
                    link_target,
                });
            }
        }
        if short_content {
            break; // truncated capture: the final entry's content is cut
        }
        offset = block_end;
    }
    Ok(entries)
}

/// The name from the header's own fields: `prefix/name` when the ustar
/// prefix is set, else the name field alone.
fn header_name(header: &[u8]) -> Result<String, String> {
    let name = nul_terminated(&header[0..100])?;
    let prefix = nul_terminated(&header[345..500])?;
    if prefix.is_empty() {
        Ok(name)
    } else {
        Ok(format!("{prefix}/{name}"))
    }
}

/// An octal numeric field, NUL/space terminated. Base-256 binary fields
/// (high bit of the first byte) are refused, not misread.
fn octal(field: &[u8]) -> Result<u64, String> {
    if field.first().is_some_and(|b| b & 0x80 != 0) {
        return Err("base-256 tar numeric field refused".into());
    }
    let text = nul_terminated(field)?;
    let digits = text.trim_end_matches(' ');
    if digits.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(digits, 8).map_err(|_| format!("malformed octal field {digits:?}"))
}

/// Bytes up to the first NUL as UTF-8; names this backend cannot spell
/// are an error, never a lossy stand-in.
fn nul_terminated(bytes: &[u8]) -> Result<String, String> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(str::to_string)
        .map_err(|_| "non-UTF-8 tar name".into())
}

/// The `path` override of a pax per-file extended header, if present.
/// Records are `LEN KEY=VALUE\n` where LEN counts the whole record.
fn pax_path(content: &[u8]) -> Result<Option<String>, String> {
    let mut path = None;
    let mut cursor = 0;
    while cursor < content.len() {
        let gap = content[cursor..]
            .iter()
            .position(|&b| b == b' ')
            .ok_or("malformed pax record length")?;
        let digits = std::str::from_utf8(&content[cursor..cursor + gap])
            .map_err(|_| "malformed pax record length")?;
        let length: usize = digits.parse().map_err(|_| "malformed pax record length")?;
        let record = content
            .get(cursor + gap + 1..cursor + length)
            .filter(|_| length > gap + 1)
            .ok_or("pax record overruns its header")?;
        if let Some(value) = record.strip_prefix(b"path=") {
            let value = value.strip_suffix(b"\n").unwrap_or(value);
            path = Some(
                std::str::from_utf8(value)
                    .map_err(|_| "non-UTF-8 pax path")?
                    .to_string(),
            );
        }
        cursor += length;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One ustar header + content blocks for a single entry.
    fn entry(name: &str, typeflag: u8, content: &[u8]) -> Vec<u8> {
        let mut header = [0u8; BLOCK];
        let name_bytes = name.as_bytes();
        assert!(name_bytes.len() <= 100);
        header[..name_bytes.len()].copy_from_slice(name_bytes);
        let size = format!("{:011o}", content.len());
        header[124..124 + size.len()].copy_from_slice(size.as_bytes());
        header[257..262].copy_from_slice(b"ustar");
        header[156] = typeflag;
        let mut out = header.to_vec();
        out.extend_from_slice(content);
        out.resize(out.len() + (BLOCK - content.len() % BLOCK) % BLOCK, 0);
        out
    }

    fn archive(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut out = parts.concat();
        out.extend_from_slice(&[0u8; BLOCK]); // end marker
        out
    }

    #[test]
    fn parses_kinds_names_and_content_offsets() {
        let mut link = entry("data/link", b'2', b"");
        link[157..157 + 9].copy_from_slice(b"hello.txt");
        let bytes = archive(&[
            entry("data", b'5', b""),
            entry("data/hello.txt", b'0', b"hello strop\n"),
            link,
            entry("data/fifo", b'6', b""),
        ]);
        let entries = parse(&bytes, true).unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].name, "data");
        assert_eq!(entries[0].kind, TarKind::Dir);
        assert_eq!(entries[1].kind, TarKind::File);
        assert_eq!(entries[1].size, 12);
        assert_eq!(&bytes[entries[1].data.clone()], b"hello strop\n");
        assert_eq!(entries[2].kind, TarKind::Symlink);
        assert_eq!(entries[2].link_target.as_deref(), Some("hello.txt"));
        assert_eq!(entries[3].kind, TarKind::Other);
        assert_eq!(entries[3].link_target, None);
    }

    #[test]
    fn gnu_longname_overrides_the_header_field() {
        let long = format!("dir/{}", "x".repeat(120));
        let bytes = archive(&[
            entry("longname", b'L', format!("{long}\0").as_bytes()),
            entry("truncated", b'0', b"abc"),
        ]);
        let entries = parse(&bytes, true).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, long);
    }

    #[test]
    fn pax_path_overrides_the_header_field() {
        let record_body = "path=pax/spelled name.txt\n";
        // LEN counts the whole record: two digits, the space, the body.
        let record = format!("{} {}", record_body.len() + 3, record_body);
        let bytes = archive(&[
            entry("pax", b'x', record.as_bytes()),
            entry("field", b'0', b"z"),
        ]);
        let entries = parse(&bytes, true).unwrap();
        assert_eq!(entries[0].name, "pax/spelled name.txt");
    }

    #[test]
    fn a_truncated_capture_keeps_the_final_entries_prefix_only() {
        let full = entry("big.bin", b'0', &[7u8; 1000]);
        let cut = &full[..BLOCK + 400]; // header intact, content cut
        let entries = parse(cut, false).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].size, 1000);
        assert_eq!(entries[0].data.len(), 400);
        assert!(parse(cut, true).is_err(), "complete stream must not lie");
        assert!(parse(&cut[..BLOCK + 100], true).is_err());
        assert!(parse(&cut[..100], false).is_ok(), "zero-padded tail ends");
    }

    #[test]
    fn structural_garbage_is_an_error() {
        assert!(parse(b"not a tar at all", true).is_err());
        let mut bad = entry("a", b'0', b"");
        bad[257..262].copy_from_slice(b"nope!");
        assert!(parse(&bad, true).is_err(), "bad magic");
        let mut bad = entry("a", b'0', b"");
        bad[124] = 0x80; // base-256 size field
        assert!(parse(&bad, true).is_err(), "base-256 refused");
    }
}
