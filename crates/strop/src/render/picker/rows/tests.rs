use super::*;
use std::path::PathBuf;
use strop_picker::{Item, Payload};

fn t() -> Tokens {
    Tokens::new(strop_core::theme::ThemeId::Hone)
}

fn sym(name: &str, container: &str, line: usize, kind: &str) -> Item {
    Item {
        badge: Some(kind.into()),
        text: format!("{name}  {container} · :{line}"),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(PathBuf::from("/p/net.cpp")),
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
    let line = symbol_row(&t(), &item, &[], 80, false);
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
    let a = symbol_row(&t(), &sym("a", "", 1, "fn"), &[], 80, false);
    let b = symbol_row(&t(), &sym("b", "", 1, "variant"), &[], 80, false);
    assert_eq!(line_text(&a).find('a'), line_text(&b).find('b'));
}

fn grep_item(path: &str, line: usize, col: usize, len: usize, text: &str) -> Item {
    Item {
        badge: None,
        text: text.into(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(PathBuf::from(path)),
            line,
            col,
            match_len: len,
            line_text: text.into(),
        },
    }
}

#[test]
fn replace_rows_show_the_hit_and_its_delta() {
    let item = grep_item(
        "src/search/query-parser.rs",
        5,
        8,
        13,
        "pub fn retry_request(attempt: usize) -> bool {",
    );
    let t = t();
    let lines = search_rows(&t, &item, Some("dispatch_request"), false, 100, false, 4);
    assert_eq!(lines.len(), 2, "one hit row and one delta row");
    let old = line_text(&lines[0]);
    assert!(old.contains("    5  "), "line number: {old:?}");
    assert!(old.contains("retry_request"), "old window: {old:?}");
    assert!(!old.contains("[x]"), "no checkbox vocabulary: {old:?}");
    let new = line_text(&lines[1]);
    assert!(new.contains("dispatch_request"), "new window: {new:?}");
    assert!(!new.contains("retry_request"), "delta, not a copy: {new:?}");
    // common diff roles: `-` old in the deletion role, `+` new in addition
    let old_sign = lines[0].spans.iter().find(|s| s.content == "- ").unwrap();
    assert_eq!(old_sign.style.fg, Some(t.diff_del));
    let new_sign = lines[1].spans.iter().find(|s| s.content == "+ ").unwrap();
    assert_eq!(new_sign.style.fg, Some(t.diff_add));
    // match/replacement evidence stays accent+bold over the roles
    let amber = |line: &Line| {
        line.spans
            .iter()
            .filter(|span| span.style.fg == Some(t.accent))
            .map(|span| span.content.as_ref())
            .collect::<String>()
    };
    assert_eq!(amber(&lines[0]), "retry_request");
    assert_eq!(amber(&lines[1]), "dispatch_request");
}

/// U7: a file header once per group — relative path left, the hit
/// count right — in text and secondary, never a checkbox.
#[test]
fn file_headers_name_the_relative_path_and_hit_count() {
    let t = t();
    let location = strop_workspace::ResourceLocation::local(PathBuf::from("/w/docs/remote.md"));
    let line = file_header(&t, &location, std::path::Path::new("/w"), Some(3), 60);
    let text = line_text(&line);
    assert!(text.starts_with(" docs/remote.md ───"), "{text:?}");
    assert!(text.trim_end().ends_with("─ 3"), "{text:?}");
    assert!(!text.contains("/w/"), "workspace-relative: {text:?}");
    assert_eq!(line.width(), 60);
    // a ruled heading: directory secondary, name bold text, leader faint
    let style = |needle: &str| {
        line.spans
            .iter()
            .find(|s| s.content.as_ref() == needle)
            .unwrap_or_else(|| panic!("{needle:?} in {text:?}"))
            .style
    };
    assert_eq!(style("docs/").fg, Some(t.secondary));
    assert_eq!(style("remote.md").fg, Some(t.text));
    assert!(style("remote.md").add_modifier.contains(Modifier::BOLD));
    let leader = line
        .spans
        .iter()
        .find(|s| s.content.contains('─'))
        .expect("leader");
    assert_eq!(leader.style.fg, Some(t.faint));
    // too tight for a leader: the gap is plain and the name is whole
    let tight = file_header(&t, &location, std::path::Path::new("/w"), Some(3), 20);
    let tight_text = line_text(&tight);
    assert_eq!(tight_text, " docs/remote.md   3 ");
    assert!(!tight_text.contains('─'));
}

#[test]
fn buffer_rows_split_the_directory_from_the_name_and_keep_terminals_whole() {
    let t = t();
    let mut arena: strop_core::id::Arena<strop_core::id::DocumentKind, ()> =
        strop_core::id::Arena::default();
    let file = Item {
        badge: None,
        text: "crates/strop/src/render/card.rs".into(),
        payload: Payload::Buffer(arena.try_insert(()).unwrap()),
    };
    let line = buffer_row(&t, &file, &[], 60, false);
    assert_eq!(
        line_text(&line).trim_end(),
        "  crates/strop/src/render/card.rs"
    );
    let dir = &line.spans[2];
    let name = &line.spans[3];
    assert_eq!(dir.content, "crates/strop/src/render/");
    assert_eq!(dir.style.fg, Some(t.secondary));
    assert_eq!(name.content, "card.rs");
    assert_eq!(name.style.fg, Some(t.text));
    assert!(!name.style.add_modifier.contains(Modifier::BOLD));
    let terminal = Item {
        badge: Some("!".into()),
        text: "term:///w//1:sh · running".into(),
        payload: Payload::Buffer(arena.try_insert(()).unwrap()),
    };
    let line = buffer_row(&t, &terminal, &[], 60, false);
    assert_eq!(line.spans[2].content, "term:///w//1:sh");
    assert_eq!(line.spans[2].style.fg, Some(t.text));
    assert_eq!(line.spans[3].content, " · running");
    assert_eq!(line.spans[3].style.fg, Some(t.muted));
}

#[test]
fn excluded_hits_strike_through_in_muted_not_failure_red() {
    let item = grep_item(
        "tests/request-tests.rs",
        5,
        9,
        13,
        "assert!(retry_request(1));",
    );
    let t = t();
    let lines = search_rows(&t, &item, Some("dispatch_request"), true, 100, false, 4);
    assert_eq!(
        lines.len(),
        2,
        "the delta slot stays blank for an excluded hit"
    );
    let hit = &lines[0];
    assert!(
        hit.spans
            .iter()
            .any(|span| span.content.contains("retry_request")
                && span.style.add_modifier.contains(Modifier::CROSSED_OUT)
                && span.style.fg == Some(t.muted)),
        "struck through in muted: {hit:?}"
    );
    for line in &lines {
        for span in &line.spans {
            assert_ne!(span.style.fg, Some(t.diff_del), "no deletion red: {span:?}");
            assert_ne!(
                span.style.fg,
                Some(t.diff_add),
                "no addition green: {span:?}"
            );
            assert_ne!(span.style.fg, Some(t.accent), "no match evidence: {span:?}");
            assert_ne!(
                span.style.fg,
                Some(t.error),
                "not the failure red: {span:?}"
            );
        }
    }
}

#[test]
fn replace_rows_selected_band_covers_the_logical_block() {
    let item = grep_item("a.rs", 1, 1, 3, "foo bar");
    let t = t();
    for line in search_rows(&t, &item, Some("baz"), false, 40, true, 4) {
        assert_eq!(line.width(), 40, "the band pads to full width");
        assert!(
            line.spans.iter().all(|s| s.style.bg == Some(t.raised)),
            "every cell carries the band"
        );
        assert!(line_text(&line).starts_with('▌') || line_text(&line).starts_with(' '));
    }
}

#[test]
fn replace_rows_empty_replacement_renders_the_deletion() {
    let item = grep_item("a.rs", 1, 1, 3, "foo bar");
    let lines = search_rows(&t(), &item, Some(""), false, 60, false, 4);
    let new = line_text(&lines[1]);
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
fn theme_rows_use_registry_labels_six_swatches_and_the_applied_mark() {
    for id in strop_core::theme::ThemeId::ALL {
        let t = Tokens::new(id);
        let line = theme_row(&t, id, &[], 48, false);
        let text = line_text(&line);
        assert!(text.contains(id.label()), "{} label: {text:?}", id.name());
        assert!(text.contains(" ●"), "{} applied: {text:?}", id.name());
        assert!(!text.contains(id.description()), "{} row", id.name());
        let palette = id.palette();
        let expected = [
            palette.bg.base,
            palette.bg.surface,
            palette.fg.text,
            palette.accent,
            palette.syn.keyword,
            palette.syn.string,
        ];
        let swatches: Vec<_> = line.spans.iter().filter(|s| s.content == "■").collect();
        assert_eq!(swatches.len(), 6);
        for (span, role) in swatches.iter().zip(expected) {
            assert_eq!(span.style.fg, Some(crate::render::style::color(role)));
        }
    }
}

#[test]
fn theme_selection_band_is_independent_of_applied_mark() {
    let t = t(); // the Hone tokens: Hone is applied
    let selected = theme_row(&t, strop_core::theme::ThemeId::Slate, &[], 48, true);
    assert!(!line_text(&selected).contains('●'));
    assert!(selected
        .spans
        .iter()
        .all(|span| span.style.bg == Some(t.raised)));

    let applied = theme_row(&t, strop_core::theme::ThemeId::Hone, &[], 48, false);
    assert!(line_text(&applied).contains('●'));
    assert!(applied
        .spans
        .iter()
        .all(|span| span.style.bg != Some(t.raised)));
}

#[test]
fn hidden_theme_matches_never_leak_onto_human_labels() {
    let t = t();
    let id = strop_core::theme::ThemeId::PaperDark;
    let canonical_start = id.label().chars().count() + " — ".chars().count();
    let canonical = (canonical_start..canonical_start + id.name().chars().count())
        .map(|column| column as u32)
        .collect::<Vec<_>>();
    let line = theme_row(&t, id, &canonical, 48, false);
    let highlighted = line
        .spans
        .iter()
        .filter(|span| {
            span.style.fg == Some(t.accent) && span.content.chars().any(char::is_alphanumeric)
        })
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(
        highlighted.is_empty(),
        "hidden match painted {highlighted:?}"
    );

    let visible = theme_row(&t, id, &[0, 1, 2, 3], 48, false);
    let highlighted = visible
        .spans
        .iter()
        .filter(|span| {
            span.style.fg == Some(t.accent) && span.content.chars().any(char::is_alphanumeric)
        })
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert_eq!(highlighted, "Pape");
}

#[test]
fn tiny_theme_rows_drop_swatches_before_identity() {
    let t = Tokens::new(strop_core::theme::ThemeId::PaperDark);
    let id = strop_core::theme::ThemeId::PaperDark;
    let narrow = theme_row(&t, id, &[], 17, true);
    let text = line_text(&narrow);
    assert!(text.contains('●'), "applied mark: {text:?}");
    assert!(text.contains("Paper"), "label prefix: {text:?}");
    assert!(!text.contains('■'), "decoration must yield: {text:?}");
    for width in 0..24 {
        assert!(theme_row(&t, id, &[], width, true).width() <= width as usize);
    }
}

/// U8: diagnostics rows read severity, message, then the location.
#[test]
fn diagnostic_rows_show_severity_message_and_location() {
    let t = t();
    let item = Item {
        badge: None,
        text: "/w/src/editmap.rs:25 E cannot find function `xyz` in this scope".into(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(PathBuf::from("/w/src/editmap.rs")),
            line: 25,
            col: 4,
            match_len: 1,
            line_text: "cannot find function `xyz` in this scope".into(),
        },
    };
    let line = diagnostic_row(&t, &item, std::path::Path::new("/w"), 80, true);
    let text = line_text(&line);
    assert!(text.starts_with("▌ ● cannot find function"), "{text:?}");
    assert!(text.trim_end().ends_with("src/editmap.rs:25"), "{text:?}");
    assert!(!text.contains("/w/"), "{text:?}");
    let glyph = line.spans.iter().find(|s| s.content.contains('●')).unwrap();
    assert_eq!(glyph.style.fg, Some(t.error));
    assert_eq!(line.width(), 80);
    for width in 0..40 {
        assert!(
            diagnostic_row(&t, &item, std::path::Path::new("/w"), width, false).width()
                <= width as usize
        );
    }
}

#[test]
fn source_rows_are_total_and_cell_bounded_at_degenerate_widths() {
    let t = t();
    let item = grep_item("src/界界/é.rs", 123, 5, 0, "界 é");
    for width in 0..30 {
        for line in search_rows(&t, &item, None, false, width, true, 3)
            .into_iter()
            .chain(search_rows(&t, &item, Some("界"), false, width, true, 3))
        {
            assert!(line.width() <= width as usize, "width {width}: {line:?}");
        }
        let line = symbol_row(&t, &sym("界é", "根", 123, "fn"), &[], width, true);
        assert!(line.width() <= width as usize);
        let header = file_header(
            &t,
            &strop_workspace::ResourceLocation::local(PathBuf::from("src/界界/é.rs")),
            std::path::Path::new("/"),
            Some(12),
            width,
        );
        assert!(header.width() <= width as usize);
    }
}

/// Files rows: basename first, directory after it at one shared column.
#[test]
fn file_rows_put_the_directory_after_the_name_at_a_shared_column() {
    let t = t();
    let a = file_row(&t, "plans/STATUS.md", &[0, 1], 14, 44, true);
    let b = file_row(
        &t,
        "crates/strop-engine/src/editor/lsp/status.rs",
        &[],
        14,
        44,
        false,
    );
    let a_text = line_text(&a);
    let b_text = line_text(&b);
    let column =
        |text: &str, needle: &str| text.find(needle).map(|byte| text[..byte].chars().count());
    assert!(a_text.starts_with("▌ STATUS.md"), "{a_text:?}");
    assert_eq!(
        column(&a_text, "plans/"),
        column(&b_text, "…"),
        "shared directory column: {b_text:?}"
    );
    assert!(b_text.contains("lsp/"), "elided from the left: {b_text:?}");
    assert_eq!(a.width(), 44);
    for width in 0..30 {
        assert!(file_row(&t, "plans/STATUS.md", &[], 14, width, true).width() <= width as usize);
    }
}
