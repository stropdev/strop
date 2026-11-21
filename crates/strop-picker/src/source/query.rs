//! Grep hit rows and the rg JSON adapter for remote search.
use crate::query::{ContentPlan, Evidence, RowFacts};
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

/// An rg match record is a candidate line. Its provider submatches must
/// not become rows: the alternation need not expose every exact atom.
pub(super) fn parse_json_match_with(
    line: &[u8],
    root: &ResourceLocation,
    plan: &ContentPlan,
    evidence: &Evidence<'_>,
) -> Result<Vec<Item>, String> {
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
    let relative = location
        .path
        .strip_prefix(&root.path)
        .map_err(|_| "rg path escaped scope")?;
    let relative_path = relative.to_string_lossy();
    let row = RowFacts {
        path: Some(&relative_path),
        native_path: Some(relative),
        line: Some(line_no),
        text: &text,
        ..Default::default()
    };
    if !plan.admits(&row, evidence) {
        return Ok(Vec::new());
    }
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
    }
    let display = strop_workspace::directory::display_path(relative);
    let mut items = Vec::new();
    append_matches(&mut items, plan, &location, &display, line_no, &text, None)?;
    Ok(items)
}

/// Publish the same bounded, exact spans for disk, dirty buffers and SSH.
pub(super) fn append_matches(
    items: &mut Vec<Item>,
    plan: &ContentPlan,
    location: &ResourceLocation,
    display: &str,
    line_no: usize,
    text: &std::sync::Arc<str>,
    badge: Option<&'static str>,
) -> Result<(), String> {
    let short: String = text.trim().chars().take(80).collect();
    let mut expanded = text.len();
    for (index, hit) in plan.spans(text).enumerate() {
        if index == MATCH_LIMIT {
            return Err(format!(
                "{display}:{line_no}: source line exceeds 4096 matches"
            ));
        }
        let item = Item {
            badge: badge.map(Into::into),
            text: format!("{display}:{line_no} · {short}"),
            payload: Payload::Grep {
                location: location.clone(),
                line: line_no,
                col: hit.start() + 1,
                match_len: hit.len(),
                line_text: text.clone(),
            },
            facts: Default::default(),
        };
        expanded = expanded.saturating_add(super::flow::row_bytes(&item));
        if expanded > super::flow::BACKLOG_BYTES {
            return Err(format!(
                "{display}:{line_no}: source line exceeds 4 MiB of result rows"
            ));
        }
        items.push(item);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan(query: &str) -> ContentPlan {
        ContentPlan::compile(&crate::query::parse_search(query))
            .unwrap()
            .unwrap()
    }
    #[test]
    fn exact_admission_sees_the_remote_path_line_and_text() {
        let root = ResourceLocation::local(std::path::PathBuf::from("/work"));
        let record = |path: &str, line: usize, text: &str| {
            serde_json::to_vec(&serde_json::json!({
                "type": "match",
                "data": {
                    "path": { "text": path },
                    "lines": { "text": format!("{text}\n") },
                    "line_number": line,
                    "submatches": [{ "start": 0, "end": 1 }]
                }
            }))
            .unwrap()
        };
        // The plan must receive the record's line number and drop the
        // line the index excludes (outside every function).
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("src")).unwrap();
        std::fs::write(
            directory.path().join("src/lib.rs"),
            "fn wrap() {\n    let parser = 1;\n}\nlet bare = 2;\n",
        )
        .unwrap();
        let index = crate::source::symbols::SymbolIndex::build(
            directory.path(),
            &[std::path::PathBuf::from("src/lib.rs")],
            &|| false,
        );
        let plan = plan("kind:function AND parser");
        let evidence = Evidence {
            symbols: Some(&index),
            ..Default::default()
        };
        let kept = parse_json_match_with(
            &record("src/lib.rs", 2, "    let parser = 1"),
            &root,
            &plan,
            &evidence,
        )
        .unwrap();
        assert_eq!(kept.len(), 1);
        let dropped = parse_json_match_with(
            &record("src/lib.rs", 4, "let bare = parser"),
            &root,
            &plan,
            &evidence,
        )
        .unwrap();
        assert!(dropped.is_empty(), "the index excludes the bare line");
    }

    #[test]
    fn remote_boolean_hits_use_the_atoms_not_the_folded_provider_coordinates() {
        let root = ResourceLocation::local("/work".into());
        let line = br#"{"type":"match","data":{"path":{"text":"case.txt"},"lines":{"text":"Needle needle\n"},"line_number":1,"submatches":[{"start":0,"end":6},{"start":7,"end":13}]}}"#;
        let hits = parse_json_match_with(line, &root, &plan("Needle OR eed"), &Evidence::default())
            .unwrap();
        let mut spans: Vec<_> = hits
            .into_iter()
            .map(|item| {
                let Payload::Grep { col, match_len, .. } = item.payload else {
                    panic!("expected grep hit")
                };
                (col, match_len)
            })
            .collect();
        spans.sort_unstable();
        assert_eq!(spans, [(1, 6), (2, 3), (9, 3)]);
    }

    #[test]
    fn remote_match_limit_counts_exact_hits_not_prefilter_spans() {
        let root = ResourceLocation::local("/work".into());
        let text = format!("N{}", "n".repeat(MATCH_LIMIT));
        let record = serde_json::to_vec(&serde_json::json!({
            "type": "match",
            "data": {
                "path": {"text": "case.txt"},
                "line_number": 1,
                "submatches": (0..text.len()).map(|i| serde_json::json!({"start":i,"end":i+1})).collect::<Vec<_>>(),
                "lines": {"text": text}
            }
        })).unwrap();
        assert_eq!(
            parse_json_match_with(&record, &root, &plan("N NOT x"), &Evidence::default())
                .unwrap()
                .len(),
            1
        );
        let error = parse_json_match_with(&record, &root, &plan("n NOT x"), &Evidence::default())
            .unwrap_err();
        assert!(error.contains("4096 matches"), "{error}");
    }

    #[test]
    fn json_matches_retain_scope_and_exact_submatch_coordinates() {
        let root = ResourceLocation::remote(
            strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap(),
            "/work".into(),
        );
        let line = br#"{"type":"match","data":{"path":{"text":"src/main.rs"},"lines":{"text":"    let x = sharpen(); sharpen();\n"},"line_number":12,"submatches":[{"start":12,"end":19},{"start":23,"end":30}]}}"#;
        let parse_json_match = |record: &[u8], root: &ResourceLocation| {
            parse_json_match_with(record, root, &plan("sharpen"), &Evidence::default())
        };
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
        assert!(parse_json_match_with(
            &record,
            &ResourceLocation::local("/work".into()),
            &plan("x"),
            &Evidence::default()
        )
        .is_err());
    }
}
