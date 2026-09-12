use super::*;

#[test]
fn external_header_keeps_the_originating_server() {
    // 0049 §4: gd out of the working tree retains the replying server;
    // the second gd resolves through the carried binding.
    let mut e = cpp_editor();
    let context = arm_cpp(&mut e, 0, 7, "/proj");
    let target = insert_target(&mut e, "/usr/include/vector.hpp", "// vector\n");
    e.finish_lsp_jump(target, at_origin(), context);
    let carried = e
        .lsp_state
        .jump_contexts
        .get(&target)
        .expect("the jump carried the originating server's context");
    assert_eq!(carried.server, ServerId::new(7));
    assert_eq!(carried.root, PathBuf::from("/proj"));
    assert_eq!(carried.language, "cpp");
    assert!(
        !e.lsp_state.bindings.contains_key(&target),
        "a carried context is a routing hint, not open state — didOpen follows"
    );
    let resolved = e.lsp_server_for(
        target,
        Path::new("/usr/include/vector.hpp"),
        "cpp",
        &strop_workspace::Filesystem::Local,
    );
    assert_eq!(resolved, Some((ServerId::new(7), PathBuf::from("/proj"))));
    assert!(
        e.docs.get(target).unwrap().buf.readonly,
        "outside-root content stays read-only"
    );
}

#[test]
fn extensionless_header_inherits_the_navigation_language() {
    let mut e = cpp_editor();
    let context = arm_cpp(&mut e, 0, 7, "/proj");
    let target = insert_target(&mut e, "/usr/include/vector", "// no extension\n");
    e.finish_lsp_jump(target, at_origin(), context);
    assert_eq!(
        e.lsp_state
            .jump_contexts
            .get(&target)
            .map(|c| c.language.as_str()),
        Some("cpp"),
        "extensionless inherits the jump's language (0049 §4.4)"
    );
    assert_eq!(
        e.lsp_doc_language(target, Path::new("/usr/include/vector"))
            .as_deref(),
        Some("cpp")
    );
}

#[test]
fn ambiguous_c_header_inherits_cpp_context() {
    let mut e = cpp_editor();
    let context = arm_cpp(&mut e, 0, 7, "/proj");
    let target = insert_target(&mut e, "/usr/include/legacy.h", "// c header\n");
    e.finish_lsp_jump(target, at_origin(), context);
    assert_eq!(
        e.lsp_state
            .jump_contexts
            .get(&target)
            .map(|c| c.language.as_str()),
        Some("cpp"),
        "an ambiguous .h keeps the cpp navigation context (0049 §4.4)"
    );
}

#[test]
fn a_second_project_never_switches_an_existing_binding() {
    // 0049 §4.5: the shared header reached from project B keeps the
    // context project A established — no silent switch.
    let mut e = cpp_editor();
    let first = insert_target(&mut e, "/usr/include/vector.hpp", "// shared\n");
    e.lsp_state.bindings.insert(
        first,
        state::Binding {
            server: ServerId::new(9),
            revision: e.docs.get(first).unwrap().buf.revision(),
            path: PathBuf::from("/usr/include/vector.hpp"),
            root: PathBuf::from("/proj-a"),
            language: "cpp".into(),
            target: strop_workspace::Filesystem::Local,
        },
    );
    e.switch_to(e.current()); // no-op clarity: current is the target
    let origin = e.current();
    // jump from the /proj (server 7) document into the shared header
    e.switch_to(origin);
    let context = arm_cpp(&mut e, 0, 7, "/proj");
    e.finish_lsp_jump(first, at_origin(), context);
    assert_eq!(
        e.lsp_state.bindings.get(&first).map(|b| b.server),
        Some(ServerId::new(9)),
        "the first project's context stands"
    );
    assert!(
        !e.lsp_state.jump_contexts.contains_key(&first),
        "and no second context queues behind it"
    );
}

#[test]
fn manual_external_open_names_the_context_route() {
    // 0049 §4.6: a server exists but doesn't cover this path — the
    // message says so and names the way in, no install red herring.
    let mut e = Editor::new_in(Buffer::from_text("int main() {}\n"), PathBuf::from("/proj"));
    e.buf_mut().path = Some(PathBuf::from("/usr/include/lonely.hpp"));
    e.lsp_state.attach.attached.push(super::attach::Attachment {
        language: "cpp".into(),
        root: PathBuf::from("/proj"),
        server: ServerId::new(7),
        target: strop_workspace::Filesystem::Local,
    });
    e.lsp_request(RequestKind::Goto);
    assert!(
        e.message.contains("no language context"),
        "truthful context route, got: {}",
        e.message
    );
    // And with no server at all, the install advice is the honest one.
    let mut e = Editor::new_in(Buffer::from_text("int main() {}\n"), PathBuf::from("/proj"));
    e.buf_mut().path = Some(PathBuf::from("/usr/include/lonely.hpp"));
    e.lsp_request(RequestKind::Goto);
    assert!(
        e.message.contains("no language server"),
        "install advice only when nothing serves the language: {}",
        e.message
    );
}

#[test]
fn document_symbols_open_a_picker_and_accepting_jumps() {
    // 0047 §1: the reply becomes picker rows; Enter lands on the
    // symbol and records a jumplist entry for ctrl-o.
    let mut e = editor("struct Foo;\nimpl Foo { fn bar() {} }\nfn main() {}\n");
    e.feed_text("G"); // cursor to line 3, so the jump back is observable
    let origin = e.head();
    let context = arm(
        &mut e,
        0,
        RequestKind::DocumentSymbols,
        PositionEncoding::Utf8,
    );
    e.handle_lsp_event(LspEvent::Symbols {
        context,
        symbols: vec![
            strop_lsp::protocol::ProtoSymbol {
                name: "Foo".into(),
                container: String::new(),
                kind: "Struct".into(),
                location: strop_lsp::ServerLocation {
                    doc: strop_workspace::ResourceLocation::local(PathBuf::from(
                        "/workspace/origin.txt",
                    )),
                    position: ServerPosition {
                        line: LineIndex::new(0),
                        column: ServerColumn::new(7),
                    },
                },
            },
            strop_lsp::protocol::ProtoSymbol {
                name: "bar".into(),
                container: "Foo".into(),
                kind: "Method".into(),
                location: strop_lsp::ServerLocation {
                    doc: strop_workspace::ResourceLocation::local(PathBuf::from(
                        "/workspace/origin.txt",
                    )),
                    position: ServerPosition {
                        line: LineIndex::new(1),
                        column: ServerColumn::new(16),
                    },
                },
            },
        ],
    });
    let glue = e.picker.as_ref().expect("the symbols picker opened");
    assert_eq!(glue.picker.kind, strop_picker::Kind::Symbols);
    let items = &glue.picker.items;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].badge.as_deref(), Some("struct"), "kind chip");
    assert!(items[0].text.contains("Foo") && items[0].text.contains(":1"));
    assert_eq!(items[1].badge.as_deref(), Some("meth"));
    assert!(
        items[1].text.contains("bar")
            && items[1].text.contains("Foo")
            && items[1].text.contains(":2")
    );
    e.wait_picker();
    e.feed(Key::Enter);
    assert!(!e.picker_open());
    assert_eq!(e.head(), 7, "landed on the struct identifier");
    e.feed(Key::CtrlO);
    assert_eq!(e.head(), origin, "ctrl-o returns to the pre-jump spot");
}

#[test]
fn document_symbols_empty_reply_names_the_document() {
    let mut e = editor("x\n");
    let context = arm(
        &mut e,
        0,
        RequestKind::DocumentSymbols,
        PositionEncoding::Utf8,
    );
    e.handle_lsp_event(LspEvent::Symbols {
        context,
        symbols: Vec::new(),
    });
    assert!(!e.picker_open(), "an empty reply opens nothing");
    assert_eq!(e.message, "no symbols in this document");
}

#[test]
fn goto_completion_uses_target_encoding_and_records_original_jump() {
    let mut e = editor("origin text\n");
    e.feed_text("$");
    let origin = (e.current(), e.head());
    let context = arm(&mut e, 0, RequestKind::Goto, PositionEncoding::Utf8);
    let mut target = Buffer::from_text("a😀z\n");
    target.path = Some(PathBuf::from("/workspace/target.txt"));
    let target = e.docs.insert(Document::new(target));
    e.finish_lsp_jump(
        target,
        ServerPosition {
            line: LineIndex::new(0),
            column: ServerColumn::new(5),
        },
        context,
    );
    assert_eq!(e.current(), target);
    assert_eq!(e.head(), 5);
    e.jump_back();
    assert_eq!((e.current(), e.head()), origin);
}

#[test]
fn navigation_changed_during_load_does_not_switch_or_consume_new_request() {
    let mut e = editor("origin text\n");
    let old = arm(&mut e, 0, RequestKind::Goto, PositionEncoding::Utf8);
    let current = arm(&mut e, 1, RequestKind::Goto, PositionEncoding::Utf8);
    let target = e.docs.insert(Document::new(Buffer::from_text("target\n")));
    let before = (e.current(), e.head());
    e.finish_lsp_jump(
        target,
        ServerPosition {
            line: LineIndex::new(0),
            column: ServerColumn::new(2),
        },
        old,
    );
    assert_eq!((e.current(), e.head()), before);
    assert!(e.lsp_reply_fresh(&current));
}
