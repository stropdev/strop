//! The rg JSON adapter preserves native names and resolves them only in its scope.
use crate::{Item, Payload};
use base64::Engine;
use std::path::{Component, PathBuf};
use strop_workspace::ResourceLocation;
pub(super) const RECORD_LIMIT: usize = 1024 * 1024;
const MATCH_LIMIT: usize = 4096;

pub(super) fn scoped_path(
    root: &ResourceLocation,
    bytes: Vec<u8>,
) -> Result<ResourceLocation, String> {
    if !root.path.is_absolute() || bytes.is_empty() || bytes.contains(&0) {
        return Err("rg returned an invalid path or scope".into());
    }
    let path =
        strop_workspace::addr::uri::bytes_to_path(bytes).map_err(|error| error.to_string())?;
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
    {
        return Err("rg path escaped its captured scope".into());
    }
    let path: PathBuf = root.path.join(path).components().collect();
    if !path.starts_with(&root.path) {
        return Err("rg path escaped its captured scope".into());
    }
    Ok(ResourceLocation {
        filesystem: root.filesystem.clone(),
        path,
    })
}
fn bytes(value: &serde_json::Value) -> Result<Vec<u8>, String> {
    match (
        value.get("text").and_then(|value| value.as_str()),
        value.get("bytes").and_then(|value| value.as_str()),
    ) {
        (Some(text), None) => Ok(text.as_bytes().to_vec()),
        (None, Some(value)) => base64::prelude::BASE64_STANDARD
            .decode(value)
            .map_err(|error| format!("invalid rg byte encoding: {error}")),
        _ => Err("rg text/bytes field is missing or ambiguous".into()),
    }
}
/// A validated event produces every admitted submatch, never a silent malformed prefix.
pub fn parse_json_match(line: &[u8], root: &ResourceLocation) -> Result<Vec<Item>, String> {
    if line.len() > RECORD_LIMIT {
        return Err("rg record exceeds the 1 MiB bound".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(line).map_err(|error| format!("invalid rg JSON: {error}"))?;
    match value.get("type").and_then(|value| value.as_str()) {
        Some("begin" | "end" | "summary" | "context") => return Ok(Vec::new()),
        Some("match") => {}
        _ => return Err("rg returned an unknown event type".into()),
    }
    let data = value.get("data").ok_or("rg match has no data")?;
    let location = scoped_path(root, bytes(&data["path"])?)?;
    let line_no = data["line_number"]
        .as_u64()
        .and_then(|line| usize::try_from(line).ok())
        .filter(|line| *line > 0)
        .ok_or("invalid rg line number")?;
    let line_bytes = bytes(&data["lines"])?;
    let text = std::str::from_utf8(&line_bytes).map_err(|_| {
        format!(
            "{}: non-UTF-8 source text cannot be represented",
            location.label()
        )
    })?;
    let text: std::sync::Arc<str> = text.strip_suffix('\n').unwrap_or(text).into();
    let submatches = data["submatches"]
        .as_array()
        .ok_or("rg match has no submatch array")?;
    if submatches.len() > MATCH_LIMIT {
        return Err("rg record exceeds the 4096-match bound".into());
    }
    let short: String = text.trim().chars().take(80).collect();
    let relative = location
        .path
        .strip_prefix(&root.path)
        .map_err(|_| "rg path escaped scope")?;
    let display = strop_workspace::directory::display_path(relative);
    let mut items = Vec::with_capacity(submatches.len());
    let mut expanded = text.len();
    for hit in submatches {
        let start = hit["start"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("invalid rg match start")?;
        let end = hit["end"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("invalid rg match end")?;
        if start > end
            || end > text.len()
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            return Err("rg match is outside its UTF-8 source line".into());
        }
        let col = start.checked_add(1).ok_or("rg column overflow")?;
        let item = Item {
            badge: None,
            text: format!("{display}:{line_no} · {short}"),
            payload: Payload::Grep {
                location: location.clone(),
                line: line_no,
                col,
                match_len: end - start,
                line_text: text.clone(),
            },
        };
        expanded = expanded.saturating_add(super::flow::row_bytes(&item));
        if expanded > super::flow::BACKLOG_BYTES {
            return Err("rg match record expands beyond the 4 MiB result-batch bound".into());
        }
        items.push(item);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_matches_retain_scope_and_exact_submatch_coordinates() {
        let root = ResourceLocation::remote(
            strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap(),
            "/work".into(),
        );
        let line = br#"{"type":"match","data":{"path":{"text":"src/main.rs"},"lines":{"text":"    let x = sharpen(); sharpen();\n"},"line_number":12,"submatches":[{"start":12,"end":19},{"start":23,"end":30}]}}"#;
        let items = parse_json_match(line, &root).unwrap();
        assert_eq!(items.len(), 2);
        let Payload::Grep {
            location,
            line,
            col,
            match_len,
            ..
        } = &items[0].payload
        else {
            panic!("wrong source")
        };
        assert_eq!(location.filesystem, root.filesystem);
        assert_eq!(location.path, PathBuf::from("/work/src/main.rs"));
        assert_eq!((*line, *col, *match_len), (12, 13, 7));
        assert!(parse_json_match(b"not json", &root).is_err());
        assert!(parse_json_match(br#"{"type":"begin","data":{}}"#, &root)
            .unwrap()
            .is_empty());
        assert!(scoped_path(&root, b"../foreign".to_vec()).is_err());
        assert!(scoped_path(&root, b"/outside".to_vec()).is_err());
    }

    #[test]
    fn a_small_wire_record_cannot_expand_into_an_unbounded_result_batch() {
        let record = serde_json::to_vec(&serde_json::json!({
            "type": "match",
            "data": {
                "path": { "text": format!("{}file", "x/".repeat(1024)) },
                "lines": { "text": "x".repeat(4096) },
                "line_number": 1,
                "submatches": (0..4096).map(|start| serde_json::json!({"start":start,"end":start+1})).collect::<Vec<_>>()
            }
        })).unwrap();
        assert!(record.len() < RECORD_LIMIT);
        assert!(parse_json_match(&record, &ResourceLocation::local("/work".into())).is_err());
    }
}
