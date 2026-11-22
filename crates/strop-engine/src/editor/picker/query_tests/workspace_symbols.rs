//! Workspace symbols (`Space S`): the list, its wire probe, server replies
//! and their admission, and the jump that lands on a declaration.

use super::*;

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
    assert!(!editor.buf().is_dirty());
}

/// Arm the deterministic reply seam just as a successful workspace/symbol
/// wire admission would. A current generation alone no longer owns a reply.
pub(in crate::editor::picker) fn arm_workspace_reply(editor: &mut Editor, root: &std::path::Path) {
    let server = strop_lsp::ServerId::new(9);
    if !editor
        .lsp_state
        .attach
        .attached
        .iter()
        .any(|a| a.server == server)
    {
        editor
            .lsp_state
            .attach
            .attached
            .push(crate::editor::lsp::attach::Attachment {
                root: root.to_owned(),
                language: "rust".into(),
                server,
                target: strop_workspace::Filesystem::Local,
            });
    }
    editor
        .picker
        .as_mut()
        .unwrap()
        .wsources
        .insert(server, (strop_lsp::PositionEncoding::Utf16, true));
}

#[test]
fn workspace_reply_cannot_claim_a_nested_or_same_basename_sibling_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for child in ["team-a/service", "team-a/service/nested", "team-b/service"] {
        let child = root.join(child);
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("Cargo.toml"), "").unwrap();
        std::fs::write(child.join("lib.rs"), "fn render() {}\n").unwrap();
    }
    let mut editor = Editor::new_in(Buffer::from_text(""), root.to_owned());
    editor.open_picker(Kind::WorkspaceSymbols);
    editor.wait_picker();
    arm_workspace_reply(&mut editor, &root.join("team-a/service"));
    let generation = editor.picker.as_ref().unwrap().wsymbols_generation;
    let symbols = ["team-a/service", "team-a/service/nested", "team-b/service"].map(|child| {
        strop_lsp::ProtoSymbol {
            name: "semantic_render".into(),
            kind: strop_core::symbol_kind::SymbolKind::Function,
            container: String::new(),
            span: strop_lsp::LineSpan::line(strop_core::id::LineIndex::new(0)),
            location: strop_lsp::ServerLocation {
                doc: strop_workspace::ResourceLocation::local(root.join(child).join("lib.rs")),
                position: strop_lsp::ServerPosition {
                    line: strop_core::id::LineIndex::new(0),
                    column: strop_lsp::ServerColumn::new(3),
                },
            },
        }
    });
    editor.handle_lsp_event(strop_lsp::LspEvent::WorkspaceSymbols {
        server: strop_lsp::ServerId::new(9),
        generation,
        encoding: strop_lsp::PositionEncoding::Utf16,
        symbols: symbols.into(),
    });
    editor.wait_picker();
    let items = &editor.picker.as_ref().unwrap().picker.items;
    assert_eq!(
        items
            .iter()
            .filter(|row| row.facts().text == "render")
            .count(),
        3
    );
    let semantic: Vec<_> = items
        .iter()
        .filter(|row| row.facts().text == "semantic_render")
        .collect();
    assert_eq!(semantic.len(), 1);
    assert!(
        matches!(&semantic[0].payload, Payload::Symbol { location, .. }
        if location.path == root.join("team-a/service/lib.rs"))
    );
}

#[test]
fn changed_disk_witness_refuses_a_same_coordinate_jump_before_focus_moves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn alpha() {}\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), dir.path().to_owned());
    let origin = editor.current();
    editor.open_picker(Kind::WorkspaceSymbols);
    editor.wait_picker();
    let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, "fn omega() {}\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(stamp)
        .unwrap();
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    assert_eq!(editor.current(), origin);
    assert!(editor.message.contains("changed"), "{}", editor.message);
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
    arm_workspace_reply(&mut editor, dir.path());
    // The syntax tier listed both declarations; the live generation is 1.
    let generation = editor.picker.as_ref().unwrap().wsymbols_generation;
    assert_eq!(generation, 1);
    let symbol = |name: &str, line: usize| ProtoSymbol {
        name: name.into(),
        container: String::new(),
        kind: strop_core::symbol_kind::SymbolKind::Function,
        location: ServerLocation {
            doc: ResourceLocation::local(dir.path().join("lib.rs")),
            position: ServerPosition {
                line: strop_core::id::LineIndex::new(line),
                column: ServerColumn::new(3),
            },
        },
        span: strop_lsp::LineSpan::line(strop_core::id::LineIndex::new(line)),
    };
    // One new semantic hit, one duplicate of the syntax tier (dropped).
    editor.handle_lsp_event(LspEvent::WorkspaceSymbols {
        server: ServerId::new(9),
        encoding: strop_lsp::PositionEncoding::Utf16,
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
        encoding: strop_lsp::PositionEncoding::Utf16,
        generation: generation - 1,
        symbols: vec![symbol("stale", 0)],
    });
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert!(!picker
        .items
        .iter()
        .any(|item| item.text.starts_with("stale  ")));
}
/// The rows the list shows, in order.
fn visible(editor: &Editor) -> Vec<String> {
    let picker = &editor.picker.as_ref().unwrap().picker;
    picker
        .rows
        .iter()
        .map(|row| picker.items[row.item].text.clone())
        .collect()
}

#[test]
fn workspace_symbols_server_rows_are_filtered_like_syntax_rows() {
    use strop_lsp::protocol::{
        ProtoSymbol, ServerColumn, ServerId, ServerLocation, ServerPosition,
    };
    use strop_lsp::LspEvent;
    use strop_workspace::ResourceLocation;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "struct St;\nfn parse_config() {}\nfn wrap() {}\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("semantic.rs"), "").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::WorkspaceSymbols);
    editor.wait_picker();
    assert_eq!(
        editor.picker.as_ref().unwrap().picker.items.len(),
        3,
        "the full declaration list"
    );
    // `kind:` narrows and the word beside it stays fuzzy (0109): the
    // ranking pass filters the syntax tier's rows.
    editor.paste_bracketed("kind:function pcfg");
    editor.wait_picker();
    assert_eq!(visible(&editor), vec!["parse_config  lib.rs · :2"]);
    for _ in 0.."pcfg".len() {
        editor.feed(strop_core::frontend_input::Key::Backspace);
    }
    editor.paste_bracketed("parse");
    editor.wait_picker();
    let generation = editor.picker.as_ref().unwrap().wsymbols_generation;
    {
        let picker = &editor.picker.as_ref().unwrap().picker;
        assert!(picker.error.is_none(), "{:?}", picker.error);
        assert_eq!(picker.items.len(), 3, "the walk was not repeated");
        assert_eq!(visible(&editor), vec!["parse_config  lib.rs · :2"]);
    }
    let symbol = |file: &str, name: &str, kind: &str, container: &str, line: usize| ProtoSymbol {
        name: name.into(),
        container: container.into(),
        kind: crate::editor::symbols::lsp_kind(kind),
        location: ServerLocation {
            doc: ResourceLocation::local(dir.path().join(file)),
            position: ServerPosition {
                line: strop_core::id::LineIndex::new(line),
                column: ServerColumn::new(3),
            },
        },
        span: strop_lsp::LineSpan::line(strop_core::id::LineIndex::new(line)),
    };
    arm_workspace_reply(&mut editor, dir.path());
    editor.handle_lsp_event(LspEvent::WorkspaceSymbols {
        server: ServerId::new(9),
        encoding: strop_lsp::PositionEncoding::Utf16,
        generation,
        symbols: vec![
            symbol("semantic.rs", "parse_semantic", "Function", "", 0),
            // A true duplicate of the syntax row (path, line, name).
            symbol("lib.rs", "parse_config", "Function", "", 1),
            symbol("semantic.rs", "parser_field", "Field", "", 2), // kind says No
            symbol("semantic.rs", "wrap", "Function", "", 3),      // the needle says No
            symbol("semantic.rs", "parse_struct", "Struct", "", 4), // kind says No
            symbol("semantic.rs", "Parse", "EnumMember", "", 5),   // kind says No
        ],
    });
    editor.wait_picker();
    let mut texts = visible(&editor);
    texts.sort();
    assert_eq!(
        texts,
        vec![
            "parse_config  lib.rs · :2",
            "parse_semantic  semantic.rs · :1"
        ],
        "the servers' rows are filtered like the syntax rows: {texts:?}"
    );
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert_eq!(
        picker
            .items
            .iter()
            .filter(|item| item.text.starts_with("parse_config  "))
            .count(),
        1,
        "the duplicate still collapses against the syntax tier"
    );
}

#[test]
fn workspace_symbols_operator_free_keeps_static_list_fuzzy_narrowing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "struct St;\nfn parse_config() {}\nfn wrap() {}\n",
    )
    .unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::WorkspaceSymbols);
    editor.wait_picker();
    editor.paste_bracketed("wrap");
    editor.wait_picker();
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert!(picker.error.is_none(), "{:?}", picker.error);
    // Literal compatibility (0063 §3): the syntax tier does not
    // respawn per keystroke — the catalog stays whole and the local
    // fuzzy ranking narrows, exactly as before the grammar existed.
    assert_eq!(picker.items.len(), 3, "the static list is untouched");
    let visible: Vec<&str> = picker
        .rows
        .iter()
        .map(|row| picker.items.get(row.item).unwrap().text.as_str())
        .collect();
    assert_eq!(visible, vec!["wrap  lib.rs · :3"], "{visible:?}");
}

#[test]
fn workspace_symbols_wire_probe_never_carries_qualifiers() {
    use strop_trace::replay::{Node, Tape};

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "fn wrap() {}\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::WorkspaceSymbols);
    editor.wait_picker();
    // A hermetic recorder from here on: every `lsp.wsymbols` ask is
    // captured; native launches are suppressed.
    let tape = std::rc::Rc::new(Tape::fixture(|_, _| {
        Err(std::io::Error::other("native observation forbidden"))
    }));
    editor.tape = tape.clone();
    editor
        .lsp_state
        .attach
        .attached
        .push(crate::editor::lsp::attach::Attachment {
            language: "rust".into(),
            root: dir.path().to_path_buf(),
            server: strop_lsp::protocol::ServerId::new(7),
            target: strop_workspace::Filesystem::Local,
        });
    let probes = || {
        tape.fixture_nodes()
            .iter()
            .filter_map(|node| match node {
                Node::Request {
                    operation,
                    arguments,
                } if operation == "lsp.wsymbols" => {
                    Some(arguments["query"].as_str().unwrap().to_string())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    editor.paste_bracketed("kind:function wrap");
    assert_eq!(probes(), vec!["wrap"], "only bare content text crosses");
    // A qualifier-only query probes empty — never the raw input.
    editor
        .picker
        .as_mut()
        .unwrap()
        .picker
        .input
        .set_text("kind:function");
    editor.picker_input_changed();
    assert_eq!(probes(), vec!["wrap", ""], "{:?}", probes());
}

#[test]
fn workspace_symbols_field_edits_notify_exactly_once_per_completed_command() {
    use strop_trace::replay::{Node, Tape};

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "fn wrap() {}\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::WorkspaceSymbols);
    editor.wait_picker();
    // A hermetic recorder from here on: every `lsp.wsymbols` ask is
    // captured; each picker_input_changed re-asks exactly once.
    let tape = std::rc::Rc::new(Tape::fixture(|_, _| {
        Err(std::io::Error::other("native observation forbidden"))
    }));
    editor.tape = tape.clone();
    editor
        .lsp_state
        .attach
        .attached
        .push(crate::editor::lsp::attach::Attachment {
            language: "rust".into(),
            root: dir.path().to_path_buf(),
            server: strop_lsp::protocol::ServerId::new(7),
            target: strop_workspace::Filesystem::Local,
        });
    let probes = || {
        tape.fixture_nodes()
            .iter()
            .filter_map(|node| match node {
                Node::Request {
                    operation,
                    arguments,
                } if operation == "lsp.wsymbols" => {
                    Some(arguments["query"].as_str().unwrap().to_string())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    editor.paste_bracketed("one two");
    assert_eq!(probes(), vec!["one two"]);
    editor.feed(strop_core::frontend_input::Key::Esc);
    // Pure motions must never rerank.
    editor.feed_text("0lhw$");
    assert_eq!(probes(), vec!["one two"], "motions do not notify");
    // One completed operator = one notification (the revision diff,
    // not a key whitelist — dw ranks exactly like x did).
    editor.feed_text("0dw");
    assert_eq!(probes(), vec!["one two", "two"], "dw notifies once");
    editor.feed_text("x");
    assert_eq!(probes(), vec!["one two", "two", "wo"], "x notifies once");
    // A motion that finds nothing notifies nothing.
    editor.feed_text("db");
    assert_eq!(probes().len(), 3, "db at the line start finds nothing");
    // Linewise clears the field's one line — one notification.
    editor.feed_text("dd");
    assert_eq!(probes().len(), 4, "dd notifies once");
    assert_eq!(editor.picker.as_ref().unwrap().picker.input.text(), "");
}

#[test]
fn workspace_symbols_invalid_query_keeps_the_list_and_retires_replies() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "struct St;\nfn parse_config() {}\n",
    )
    .unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::WorkspaceSymbols);
    editor.wait_picker();
    editor.paste_bracketed("kind:struct");
    editor.wait_picker();
    assert_eq!(visible(&editor), vec!["St  lib.rs · :1"]);
    // An unknown value is an error (0109): the list stays as it was,
    // and in-flight LSP replies for the left-behind query are retired.
    let generation = editor.picker.as_ref().unwrap().wsymbols_generation;
    editor.paste_bracketed(" kind:bogus");
    {
        let picker = &editor.picker.as_ref().unwrap().picker;
        let error = picker.error.as_deref().unwrap_or_default();
        assert!(error.starts_with("unknown kind: bogus"), "{error}");
    }
    assert_eq!(visible(&editor), vec!["St  lib.rs · :1"], "the list stays");
    assert_eq!(
        editor.picker.as_ref().unwrap().wsymbols_generation,
        generation + 1,
        "in-flight replies for the previous query are retired"
    );
}

#[test]
fn workspace_symbols_status_rows_survive_ast_relaunch_unconsumed() {
    use crate::editor::picker::ProjectStatus;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "struct St;\nfn wrap() {}\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::WorkspaceSymbols);
    editor.wait_picker();
    editor.record_project_status(
        dir.path().join("web"),
        ProjectStatus::ambiguous("package.json"),
    );
    editor.wait_picker();
    assert!(editor
        .picker
        .as_ref()
        .unwrap()
        .picker
        .items
        .iter()
        .any(|item| matches!(item.payload, Payload::ProjectStatus(_))));
    // A filter and a scope change (which re-walks the syntax tier):
    // the informational status row is neither filtered as a symbol nor
    // ranked into the results — it stays the pinned tail.
    editor.paste_bracketed("kind:function hidden:include");
    editor.wait_picker();
    let picker = &editor.picker.as_ref().unwrap().picker;
    let texts = visible(&editor);
    assert!(
        texts.iter().any(|text| text.starts_with("wrap  ")),
        "{texts:?}"
    );
    assert!(
        !texts.iter().any(|text| text.starts_with("St  ")),
        "kind: narrowed the declarations: {texts:?}"
    );
    let tail = picker.items.iter().last().unwrap();
    assert!(
        matches!(tail.payload, Payload::ProjectStatus(_)),
        "the pinned tail survived the relaunch: {tail:?}"
    );
    let last_row = picker.rows.last().expect("rows are visible");
    assert!(
        matches!(
            picker.items.get(last_row.item).unwrap().payload,
            Payload::ProjectStatus(_)
        ),
        "visible, unranked, at the tail"
    );
}
