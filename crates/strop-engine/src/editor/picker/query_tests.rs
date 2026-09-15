use super::*;
use strop_core::Buffer;

#[test]
fn filter_only_files_and_literal_content_share_scope() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "request.id() request.id()\n").unwrap();
    std::fs::write(dir.path().join("b.py"), "request.id()\n").unwrap();
    std::fs::write(dir.path().join(".hidden.rs"), "request.id()\n").unwrap();
    for kind in [Kind::Files, Kind::Search] {
        let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
        editor.open_picker(kind);
        editor.paste_bracketed(if kind == Kind::Files {
            "language:rust hidden:exclude"
        } else {
            "language:rust hidden:exclude text:\"request.id()\""
        });
        editor.wait_picker();
        let picker = &editor.picker.as_ref().unwrap().picker;
        assert_eq!(
            picker.rows.len(),
            if kind == Kind::Files { 1 } else { 2 },
            "{kind:?}: {:?}",
            picker.error
        );
        for row in &picker.rows {
            match &picker.items.get(row.item).unwrap().payload {
                Payload::File(path) => {
                    assert_eq!(path, &PathBuf::from("a.rs"))
                }
                Payload::Grep { location, .. } => {
                    assert_eq!(
                        location,
                        &strop_workspace::ResourceLocation::local(dir.path().join("a.rs"))
                    );
                }
                _ => panic!("unexpected source kind"),
            }
        }
    }
}

#[test]
fn invalid_file_query_cannot_accept_the_previous_ranked_result() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "needle\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::Files);
    editor.wait_picker();
    let origin = editor.current();
    editor.paste_bracketed("language:");
    editor.feed(Key::Enter);
    assert!(editor.picker_open());
    assert_eq!(editor.current(), origin);
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert!(picker.error.is_some());
    assert!(!picker.streaming);
}

#[test]
fn dirty_source_search_and_cancelled_replace_preserve_working_context() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "disk old\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_fixture(&path).unwrap();
    editor.feed_text("iunsaved <esc>");
    let source = editor.current();
    editor.open_search(true);
    editor.paste_bracketed("text:unsaved");
    editor.wait_picker();
    editor.feed(Key::Tab);
    editor.paste_bracketed("changed");
    editor.wait_picker();
    editor.feed(Key::Enter);
    assert!(!editor.picker_open());
    assert_eq!(
        editor.docs.get(source).unwrap().buf.text().to_string(),
        "unsaved disk old\n"
    );
    editor.feed_text(":cancel-change<cr>");
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert_eq!(picker.input.text, "text:unsaved");
    assert_eq!(picker.replace_input.text, "changed");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "disk old\n");
}

#[test]
fn relative_spelling_keeps_the_dirty_live_preview() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "disk\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    let source = editor.open_fixture(&dir.path().join("a.txt")).unwrap();
    editor.doc_mut(source).buf.path = Some("a.txt".into());
    editor.feed_text("iunsaved <esc>");
    editor.open_picker(Kind::Files);
    editor.wait_picker();
    let (_, _, preview) = editor.picker_preview().unwrap();
    assert!(matches!(preview, PreviewSource::Buffer(document) if document == source));
    assert_eq!(editor.doc(source).buf.text().to_string(), "unsaved disk\n");
}

#[test]
fn reopening_a_picker_revalidates_successful_disk_previews() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "before\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    for expected in ["before\n", "after\n"] {
        std::fs::write(&path, expected).unwrap();
        editor.open_picker(Kind::Files);
        editor.wait_picker();
        assert!(matches!(
            editor.picker_preview().unwrap().2,
            PreviewSource::Loading
        ));
        let result = editor
            .preview_rx
            .as_ref()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        editor.handle_preview(result);
        let (_, _, preview) = editor.picker_preview().unwrap();
        let PreviewSource::Cached(path) = preview else {
            panic!("disk preview ready")
        };
        assert_eq!(editor.previews[&path].rope.to_string(), expected);
        editor.close_picker();
    }
}

/// A successful source's warning (rg's exit-0 stderr, e.g. a malformed
/// ignore line) is advisory: it displays, but the streamed results stay
/// valid and Enter still accepts (0051 §3 named boundaries, R9 partials).
#[test]
fn a_source_warning_keeps_results_acceptable() {
    use strop_core::worker::Outcome;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "needle\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::Search);
    // register a stream by hand (the worker_lifecycle pattern: no real rg)
    let picker = editor.picker.as_ref().unwrap().id;
    let request = editor.worker_ids.allocate().unwrap();
    let ticket = Ticket {
        request,
        key: PickerKey {
            picker,
            cwd: editor.cwd.clone(),
        },
    };
    editor.picker.as_mut().unwrap().active = Some(ticket.clone());
    editor.handle_picker_event(PickerEvent {
        ticket: ticket.clone(),
        msg: PickerMsg::Items(
            vec![Item {
                badge: None,
                text: "a.txt:1 · needle".into(),
                payload: Payload::Grep {
                    location: strop_workspace::ResourceLocation::local(dir.path().join("a.txt")),
                    line: 1,
                    col: 1,
                    match_len: 6,
                    line_text: "needle".into(),
                },
            }]
            .into(),
        ),
    });
    editor.handle_picker_event(PickerEvent {
        ticket: ticket.clone(),
        msg: PickerMsg::Warning("rg: .gitignore line 3: malformed pattern".into()),
    });
    editor.handle_picker_event(PickerEvent {
        ticket,
        msg: PickerMsg::Finished(Outcome::Success(())),
    });
    editor.wait_picker();
    let glue = editor.picker.as_ref().unwrap();
    assert!(glue.picker.error.is_none(), "a warning is not an error");
    assert_eq!(
        glue.picker.warning.as_deref(),
        Some("rg: .gitignore line 3: malformed pattern"),
        "the warning still shows"
    );
    editor.feed(Key::Enter);
    assert!(!editor.picker_open(), "Enter accepts under a warning");
}

/// Editing qualifiers (never part of the rank needle) must not clear the
/// ranked results: staleness compares the effective needle, not the raw
/// input (0051 §4: previous results stay until the new ranking lands).
#[test]
fn qualifier_edits_do_not_clear_ranked_results() {
    let mut editor = Editor::new(Buffer::from_text("x\n"));
    editor.open_picker(Kind::Search);
    let glue = editor.picker.as_mut().unwrap();
    glue.picker.append(vec![Item {
        badge: None,
        text: "src/a.rs:1 · hit".into(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(editor.cwd.join("src/a.rs")),
            line: 1,
            col: 1,
            match_len: 3,
            line_text: "hit".into(),
        },
    }]);
    // a parsed qualifier query: raw input differs from the rank needle
    glue.picker.input.text = "language:rust".into();
    glue.picker.rank_query = Some(String::new());
    let ranking = strop_picker::rank::rank(&glue.picker.filter_request(), || false)
        .unwrap()
        .unwrap();
    assert!(glue.picker.install_ranking(ranking));
    glue.ranked_query = Some(String::new());
    // typing another qualifier leaves the needle unchanged
    glue.picker.input.text = "language:rust path:src/".into();
    editor.request_picker_ranking();
    let glue = editor.picker.as_ref().unwrap();
    assert_eq!(
        glue.picker.rows.len(),
        1,
        "results stay while the effective needle is unchanged"
    );
}

#[test]
fn workspace_symbols_lists_and_jump_lands_on_the_declaration() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "struct St;\nfn wrap() {\n    let x = 1;\n}\n",
    )
    .unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::WorkspaceSymbols);
    editor.wait_picker();
    let picker = &editor.picker.as_ref().unwrap().picker;
    let texts: Vec<&str> = picker.items.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["St  lib.rs · :1", "wrap  lib.rs · :2"],
        "path-sorted declaration rows with the symbol convention"
    );
    assert_eq!(
        picker.items.get(0).unwrap().badge.as_deref(),
        Some("struct")
    );
    assert_eq!(picker.items.get(1).unwrap().badge.as_deref(), Some("fn"));
    editor.feed(Key::Down);
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    assert!(!editor.picker_open());
    let path = strop_workspace::ResourceLocation::local(dir.path().join("lib.rs"));
    assert!(editor
        .doc(editor.current())
        .matches_target(&crate::files::FileTarget::from_location(&path).unwrap()));
    // The cursor sits on the name: line 2, byte column 4 (`fn |wrap`).
    assert_eq!(editor.head(), editor.buf().line_start(1) + 3);
    assert!(!editor.buf().dirty);
}

#[test]
fn warm_server_workspace_symbols_merge_dedup_and_supersede() {
    use strop_lsp::protocol::{
        ProtoSymbol, ServerColumn, ServerId, ServerLocation, ServerPosition,
    };
    use strop_lsp::LspEvent;
    use strop_workspace::ResourceLocation;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "struct St;\nfn wrap() {\n    let x = 1;\n}\n",
    )
    .unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::WorkspaceSymbols);
    editor.wait_picker();
    // The syntax tier listed both declarations; the live generation is 1.
    let generation = editor.picker.as_ref().unwrap().wsymbols_generation;
    assert_eq!(generation, 1);
    let symbol = |name: &str, line: usize| ProtoSymbol {
        name: name.into(),
        container: String::new(),
        kind: "Function".into(),
        location: ServerLocation {
            doc: ResourceLocation::local(dir.path().join("lib.rs")),
            position: ServerPosition {
                line: strop_core::id::LineIndex::new(line),
                column: ServerColumn::new(3),
            },
        },
    };
    // One new semantic hit, one duplicate of the syntax tier (dropped).
    editor.handle_lsp_event(LspEvent::WorkspaceSymbols {
        server: ServerId::new(9),
        generation,
        symbols: vec![symbol("semantic_only", 0), symbol("wrap", 1)],
    });
    editor.wait_picker();
    let picker = &editor.picker.as_ref().unwrap().picker;
    let texts: Vec<&str> = picker.items.iter().map(|item| item.text.as_str()).collect();
    assert!(texts.contains(&"semantic_only  lib.rs · :1"));
    assert_eq!(
        texts
            .iter()
            .filter(|text| text.starts_with("wrap  "))
            .count(),
        1,
        "the duplicate collapses against the syntax tier: {texts:?}"
    );
    // A stale generation's reply never lands.
    editor.handle_lsp_event(LspEvent::WorkspaceSymbols {
        server: ServerId::new(9),
        generation: generation - 1,
        symbols: vec![symbol("stale", 0)],
    });
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert!(!picker
        .items
        .iter()
        .any(|item| item.text.starts_with("stale  ")));
}

#[test]
fn boolean_queries_search_through_the_engine_surface() {
    // 0063 §3: the Search surface accepts the Boolean grammar — the
    // gate that demanded a flat content expression was a migration
    // gap the replacement flow exposed.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "retry request\nretry only\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_search(false);
    editor.paste_bracketed("retry AND request");
    editor.wait_picker();
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert!(picker.error.is_none(), "{:?}", picker.error);
    assert!(
        !picker.items.is_empty(),
        "the conjunction matches its line: {:?}",
        picker.items.iter().map(|i| &i.text).collect::<Vec<_>>()
    );
    assert!(editor
        .picker
        .as_ref()
        .unwrap()
        .query_summary
        .contains("boolean"));
}

#[test]
fn boolean_negative_guards_narrow_the_engine_surface() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "retry request\nretry only\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_search(false);
    editor.paste_bracketed("retry NOT request");
    editor.wait_picker();
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert!(picker.error.is_none(), "{:?}", picker.error);
    assert_eq!(picker.items.len(), 1, "only the unguarded line survives");
}
