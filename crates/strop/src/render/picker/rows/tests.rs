use super::*;
use std::path::PathBuf;
use strop_picker::{Item, Payload};

fn sym(name: &str, container: &str, line: usize, kind: &str) -> Item {
    Item {
        badge: Some(kind.into()),
        text: format!("{name}  {container} · :{line}"),
        payload: Payload::Grep {
            path: PathBuf::from("/p/net.cpp"),
            line,
            col: 1,
            match_len: 1,
            line_text: "".into(),
        },
    }
}

fn line_text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn symbol_row_aligns_names_and_keeps_container_when_it_fits() {
    let item = sym("retry_request", "net :: RequestDispatcher", 5, "meth");
    let line = symbol_row(&item, &[], 80, false);
    let text = line_text(&line);
    assert!(text.contains(" meth   "), "fixed chip slot: {text:?}");
    assert!(text.contains("retry_request"), "{text:?}");
    assert!(
        text.contains("net :: RequestDispatcher"),
        "container kept at width: {text:?}"
    );
    assert!(
        text.trim_end().ends_with(":5"),
        "location at the right: {text:?}"
    );
    // name starts at one column across kinds
    let a = symbol_row(&sym("a", "", 1, "fn"), &[], 80, false);
    let b = symbol_row(&sym("b", "", 1, "variant"), &[], 80, false);
    assert_eq!(line_text(&a).find('a'), line_text(&b).find('b'));
}

fn grep_item(path: &str, line: usize, col: usize, len: usize, text: &str) -> Item {
    Item {
        badge: None,
        text: text.into(),
        payload: Payload::Grep {
            path: PathBuf::from(path),
            line,
            col,
            match_len: len,
            line_text: text.into(),
        },
    }
}

#[test]
fn replace_rows_show_identity_window_and_delta() {
    let item = grep_item(
        "src/search/query-parser.rs",
        5,
        8,
        13,
        "pub fn retry_request(attempt: usize) -> bool {",
    );
    let lines = search_rows(&item, Some("dispatch_request"), false, 100, false, 4, None);
    assert_eq!(lines.len(), 3);
    let header = line_text(&lines[0]);
    assert!(header.contains("[x]"), "inclusion state: {header:?}");
    assert!(header.contains("query-parser.rs"), "filename: {header:?}");
    assert!(header.contains("src/search"), "directory: {header:?}");
    let old = line_text(&lines[1]);
    assert!(old.contains("5:8"), "source byte column: {old:?}");
    assert!(old.contains("retry_request"), "old window: {old:?}");
    let new = line_text(&lines[2]);
    assert!(new.contains("dispatch_request"), "new window: {new:?}");
    assert!(!new.contains("retry_request"), "delta, not a copy: {new:?}");
    // common diff roles: `-` old in the deletion role, `+` new in addition
    let old_sign = lines[1].spans.iter().find(|s| s.content == "- ").unwrap();
    assert_eq!(old_sign.style.fg, Some(DEL_FG));
    let new_sign = lines[2].spans.iter().find(|s| s.content == "+ ").unwrap();
    assert_eq!(new_sign.style.fg, Some(ADD_FG));
    // match/replacement evidence stays amber+bold over the roles
    let amber = |line: &Line| {
        line.spans
            .iter()
            .filter(|span| span.style.fg == Some(ACCENT))
            .map(|span| span.content.as_ref())
            .collect::<String>()
    };
    assert_eq!(amber(&lines[1]), "retry_request");
    assert_eq!(amber(&lines[2]), "dispatch_request");
}

#[test]
fn replace_rows_exclusion_is_neutral_not_failure_red() {
    let item = grep_item(
        "tests/request-tests.rs",
        5,
        9,
        13,
        "assert!(retry_request(1));",
    );
    let lines = search_rows(&item, Some("dispatch_request"), true, 100, false, 4, None);
    let header = line_text(&lines[0]);
    assert!(
        header.contains("[ ]"),
        "excluded inclusion state: {header:?}"
    );
    assert!(header.contains("excluded"), "named, neutral: {header:?}");
    for line in &lines {
        for span in &line.spans {
            assert_ne!(span.style.fg, Some(DEL_FG), "no deletion red: {span:?}");
            assert_ne!(span.style.fg, Some(ADD_FG), "no addition green: {span:?}");
            assert_ne!(span.style.fg, Some(ACCENT), "no match evidence: {span:?}");
            assert_ne!(
                span.style.fg,
                Some(Color::Rgb(0xf3, 0x8b, 0xa8)),
                "not the failure red: {span:?}"
            );
        }
    }
}

#[test]
fn replace_rows_selected_band_covers_the_logical_block() {
    let item = grep_item("a.rs", 1, 1, 3, "foo bar");
    for line in search_rows(&item, Some("baz"), false, 40, true, 4, None) {
        let width: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
        assert_eq!(width, 40, "the band pads to full width");
        assert!(
            line.spans.iter().all(|s| s.style.bg == Some(SELECT_BG)),
            "every cell carries the band"
        );
    }
}

#[test]
fn replace_rows_empty_replacement_renders_the_deletion() {
    let item = grep_item("a.rs", 1, 1, 3, "foo bar");
    let lines = search_rows(&item, Some(""), false, 60, false, 4, None);
    let new = line_text(&lines[2]);
    assert!(new.contains("+ "), "{new:?}");
    assert!(
        new.contains("bar"),
        "context survives the deletion: {new:?}"
    );
    assert!(
        !new.contains("foo"),
        "the match is gone from the delta: {new:?}"
    );
}

#[test]
fn source_rows_are_total_and_cell_bounded_at_degenerate_widths() {
    let item = grep_item("src/界界/é.rs", 123, 5, 0, "界 é");
    for width in 0..30 {
        for line in search_rows(&item, None, false, width, true, 3, None)
            .into_iter()
            .chain(search_rows(&item, Some("界"), false, width, true, 3, None))
        {
            assert!(line.width() <= width as usize, "width {width}: {line:?}");
        }
        let line = symbol_row(&sym("界é", "根", 123, "fn"), &[], width, true);
        assert!(line.width() <= width as usize);
    }
}
