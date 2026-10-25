use super::*;

#[test]
fn closer_dedents_on_indent_only_line() {
    // open a line inside fn f() { } — auto-indented, then '}' dedents
    let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
    e.feed_text("o"); // indented to one level
    assert_eq!(e.buf().line_text(1), "    ");
    e.feed_text("}"); // closer on the indent-only line → dedent first
                      // the new line sits at col 0; the file's own closing brace is untouched
    assert_eq!(e.buf().text().to_string(), "fn f() {\n}\n}\n");
}

#[test]
fn closer_noop_with_real_text_before() {
    let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
    e.feed_text("o"); // indented one level
    e.feed_text("let x = 1;"); // real text on the line
    e.feed_text("}"); // closer after text: no dedent
    assert_eq!(e.buf().line_text(1), "    let x = 1;}");
}

#[test]
fn enter_then_closer_dedents_on_indent_only_line() {
    let mut e = Editor::new(Buffer::from_text("fn f() {\n"));
    e.feed_text("A");
    e.feed(strop_core::frontend_input::Key::Enter);
    assert_eq!(e.buf().line_text(1), "    ");
    e.feed_text("}");
    assert_eq!(e.buf().text().to_string(), "fn f() {\n}\n");
}

fn settle_indentation(editor: &mut Editor) {
    while editor.resolution.pending() {
        if !editor.resolution.blocked() && !editor.resolution.queue.is_empty() {
            editor.resume_deferred_input();
        } else {
            let event = editor
                .resolution
                .rx
                .as_ref()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("indentation worker settles");
            editor.handle_resolution(event);
        }
    }
}

fn code_editor(text: &str, filename: &str) -> Editor {
    let mut buffer = Buffer::from_text(text);
    buffer.path = Some(filename.into());
    Editor::new(buffer)
}

#[test]
fn enter_between_pair_creates_inner_line_and_aligned_closer() {
    let original = "fn main() {}\n";
    let mut editor = Editor::new(Buffer::from_text(original));
    editor.feed_text("$i<cr>");
    settle_indentation(&mut editor);
    assert_eq!(editor.buf().text().to_string(), "fn main() {\n    \n}\n");
    assert_eq!(editor.head(), editor.buf().line_start(1) + 4);
    editor.feed_text("body<esc>u");
    assert_eq!(
        editor.buf().text().to_string(),
        original,
        "split and body are one undo unit"
    );
}

#[test]
fn closer_uses_matching_openers_indent_not_one_unit() {
    for (text, expected) in [
        ("    call(\n            ", "    call(\n    )"),
        ("        call(\n  ", "        call(\n        )"),
        (
            "    outer(\n        inner()\n                ",
            "    outer(\n        inner()\n    )",
        ),
    ] {
        let mut editor = Editor::new(Buffer::from_text(text));
        editor.set_head(text.len());
        editor.feed_text("i)");
        settle_indentation(&mut editor);
        assert_eq!(editor.buf().text().to_string(), expected);
    }
}

#[test]
fn unmatched_or_mismatched_closer_keeps_user_whitespace() {
    for text in ["        ", "call[\n        ", "call([)\n        "] {
        let mut editor = Editor::new(Buffer::from_text(text));
        editor.set_head(text.len());
        editor.feed_text("i)");
        settle_indentation(&mut editor);
        assert_eq!(editor.buf().text().to_string(), format!("{text})"));
    }
}

#[test]
fn unicode_before_opener_uses_byte_honest_indentation() {
    let mut editor = Editor::new(Buffer::from_text("    fn café() {}\n"));
    editor.feed_text("$i<cr>");
    settle_indentation(&mut editor);
    assert_eq!(
        editor.buf().text().to_string(),
        "    fn café() {\n        \n    }\n"
    );
}

#[test]
fn paired_enter_preserves_tabs_and_crlf() {
    let mut editor = Editor::new(Buffer::from_text("\tfn f() {}\r\n"));
    editor.config.indent_style = crate::config::IndentStyle::Tabs;
    editor.reresolve_indents();
    editor.feed_text("$i<cr>");
    settle_indentation(&mut editor);
    assert_eq!(
        editor.buf().text().to_string(),
        "\tfn f() {\r\n\t\t\r\n\t}\r\n"
    );
    editor.feed_text("<esc>u");
    assert_eq!(editor.buf().text().to_string(), "\tfn f() {}\r\n");
}

#[test]
fn plain_text_punctuation_and_scratch_colons_are_not_code_rules() {
    for (path, text) in [("note.txt", "    note {"), ("note.md", "    note:")] {
        let mut editor = code_editor(text, path);
        editor.feed_text("A<cr>");
        settle_indentation(&mut editor);
        assert_eq!(editor.buf().text().to_string(), format!("{text}\n    "));
    }
    let mut editor = Editor::new(Buffer::from_text("    note:"));
    editor.feed_text("A<cr>");
    assert_eq!(editor.buf().text().to_string(), "    note:\n    ");
}

#[test]
fn python_colon_deepens_but_rust_colon_does_not() {
    for (path, expected) in [
        ("code.py", "    if ready:\n        "),
        ("code.rs", "    label:\n    "),
    ] {
        let text = if path.ends_with(".py") {
            "    if ready:"
        } else {
            "    label:"
        };
        let mut editor = code_editor(text, path);
        editor.feed_text("A<cr>");
        settle_indentation(&mut editor);
        assert_eq!(editor.buf().text().to_string(), expected);
    }
}

#[test]
fn enter_does_not_deepen_literals_comments_or_malformed_literal_eof() {
    for (path, text) in [
        ("code.rs", "    // {"),
        ("code.rs", "    /* {"),
        ("code.rs", "    let text = \"{"),
        ("code.rs", "    let text = r#\"{\n    {"),
        ("code.rs", "    let text = br###\"{\n    {"),
        ("code.rs", "    let text = cr##\"{\n    {"),
        ("code.rs", "    let ch = '{';"),
        ("code.py", "    # note:"),
        ("code.py", "    text = \"\"\"hello\n    {"),
        ("code.py", "    text = r\"\"\"hello\n    {"),
        ("code.py", "    text = b'''hello\n    {"),
        ("code.js", "    const text = `hello\n    {"),
    ] {
        let mut editor = code_editor(text, path);
        editor.set_head(text.len());
        editor.feed_text("i<cr>");
        settle_indentation(&mut editor);
        assert_eq!(
            editor.buf().text().to_string(),
            format!("{text}\n    "),
            "{path}: {text:?}"
        );
    }
}

#[test]
fn typed_closer_preserves_whitespace_inside_multiline_literal_or_comment() {
    for text in [
        "fn f() {\n    let text = \"hello\n        \";\n}\n",
        "fn f() {\n    let text = r#\"hello\n        \"#;\n}\n",
        "fn f() {\n    /* hello\n        */\n}\n",
    ] {
        let at = text.find("\n        ").unwrap() + 9;
        let mut editor = code_editor(text, "code.rs");
        editor.set_head(at);
        editor.feed_text("i}");
        settle_indentation(&mut editor);
        let mut expected = text.to_owned();
        expected.insert(at, '}');
        assert_eq!(editor.buf().text().to_string(), expected);
    }
}

#[test]
fn code_after_complete_comments_and_strings_still_aligns() {
    for literal in ["\"}\"", "r#\"}\"#", "br###\"}\"###", "cr##\"}\"##"] {
        let text = format!(
            "    fn f() {{\n        let s = {literal};\n        /* }} */\n                "
        );
        let mut editor = code_editor(&text, "code.rs");
        editor.set_head(text.len());
        editor.feed_text("i}");
        settle_indentation(&mut editor);
        assert_eq!(editor.buf().line_text(3), "    }");
    }
}

#[test]
fn completed_python_string_nodes_do_not_poison_incomplete_container_alignment() {
    for literal in ["\"\"\"{ }\"\"\"", "r\"\"\"{ }\"\"\"", "b'''{ }'''"] {
        let text = format!("    values = [\n        {literal},\n                ");
        let mut editor = code_editor(&text, "code.py");
        editor.set_head(text.len());
        editor.feed_text("i]");
        settle_indentation(&mut editor);
        assert_eq!(editor.buf().line_text(2), "    ]", "{literal}");
    }
}

#[test]
fn open_above_does_not_deepen_from_current_lines_opener() {
    let mut editor = Editor::new(Buffer::from_text("    if ready {\n"));
    editor.feed_text("O");
    settle_indentation(&mut editor);
    assert_eq!(editor.buf().text().to_string(), "    \n    if ready {\n");
    assert_eq!(editor.head(), 4);
}

#[test]
fn paired_enter_computes_each_cursors_own_indentation() {
    let text = "fn a() {}\n    fn b() {}\n";
    let mut editor = Editor::new(Buffer::from_text(text));
    editor.set_head(text.find('}').unwrap());
    editor.feed_text("i");
    editor.sels_mut().set_extras([text.rfind('}').unwrap()]);
    editor.feed(strop_core::frontend_input::Key::Enter);
    settle_indentation(&mut editor);
    assert_eq!(
        editor.buf().text().to_string(),
        "fn a() {\n    \n}\n    fn b() {\n        \n    }\n"
    );
    assert_eq!(editor.buf().col_of(editor.head()), 4);
    assert_eq!(editor.buf().col_of(editor.extra_selections()[0].head), 8);
}

#[test]
fn closer_aligns_every_cursor_to_its_own_opener() {
    let text = "fn a() {\n            \n}\n    fn b() {\n  \n}\n";
    let mut editor = Editor::new(Buffer::from_text(text));
    let first = text.find("\n            ").unwrap() + 13;
    let second = text.find("\n  \n").unwrap() + 3;
    editor.set_head(first);
    editor.feed_text("i");
    editor.sels_mut().set_extras([second]);
    editor.feed_text("}");
    settle_indentation(&mut editor);
    assert_eq!(
        editor.buf().text().to_string(),
        "fn a() {\n}\n}\n    fn b() {\n    }\n}\n"
    );
}

#[test]
fn dot_replays_paired_enter_at_destinations_indentation() {
    let mut editor = Editor::new(Buffer::from_text("fn a() {}\n    fn b() {}\n"));
    editor.set_head(editor.buf().text().to_string().find('}').unwrap());
    editor.feed_text("i<cr>x<esc>");
    settle_indentation(&mut editor);
    editor.set_head(editor.buf().text().to_string().rfind('}').unwrap());
    editor.feed_text(".");
    settle_indentation(&mut editor);
    assert_eq!(
        editor.buf().text().to_string(),
        "fn a() {\n    x\n}\n    fn b() {\n        x\n    }\n"
    );
}

#[test]
fn counted_paired_insertion_and_backspace_repeat_as_user_intent() {
    let mut editor = Editor::new(Buffer::from_text("{}\n"));
    editor.set_head(1);
    editor.feed_text("2i<cr>xy<bs><esc>");
    settle_indentation(&mut editor);
    assert_eq!(editor.buf().text().to_string(), "{\n    x\n    x\n}\n");
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), "{}\n");
}

#[test]
fn macro_replays_structural_enter_without_recording_expanded_insert_keys() {
    let mut editor = Editor::new(Buffer::from_text("{}\n    {}\n"));
    editor.set_head(1);
    editor.feed_text("qa2i<cr>x<esc>q");
    settle_indentation(&mut editor);
    editor.set_head(editor.buf().text().to_string().rfind('}').unwrap());
    editor.feed_text("@a");
    settle_indentation(&mut editor);
    assert_eq!(
        editor.buf().text().to_string(),
        "{\n    x\n    x\n}\n    {\n        x\n        x\n    }\n"
    );
}

#[test]
fn large_source_structural_enter_orders_typeahead_and_keeps_one_undo_unit() {
    let text = format!("{}    fn main() {{}}\n", "// padding\n".repeat(600));
    let mut editor = code_editor(&text, "large.rs");
    editor.set_head(text.rfind('}').unwrap());
    editor.feed_text("i<cr>x<esc>");
    assert!(editor.resolution.blocked());
    settle_indentation(&mut editor);
    assert!(editor
        .buf()
        .text()
        .to_string()
        .ends_with("    fn main() {\n        x\n    }\n"));
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), text);
}

#[test]
fn stale_indentation_delivery_cannot_edit_a_new_revision() {
    let text = format!("{}fn main() {{}}\n", "// padding\n".repeat(600));
    let mut editor = code_editor(&text, "large.rs");
    editor.set_head(text.rfind('}').unwrap());
    editor.feed_text("i<cr>");
    assert!(editor.resolution.blocked());
    editor.buf_mut().insert(0, "// external change\n");
    settle_indentation(&mut editor);
    assert_eq!(
        editor.buf().text().to_string(),
        format!("// external change\n{text}")
    );
    assert!(editor.message.contains("resolution cancelled"));
}

#[test]
fn readonly_transition_revokes_pending_indentation_without_moving_caret() {
    let text = format!("{}fn main() {{}}\n", "// padding\n".repeat(600));
    let mut editor = code_editor(&text, "large.rs");
    editor.set_head(text.rfind('}').unwrap());
    editor.feed_text("i<cr>");
    assert!(editor.resolution.blocked());
    let head = editor.head();
    editor.docs.get_mut(editor.current()).unwrap().buf.readonly = true;
    settle_indentation(&mut editor);
    assert_eq!(editor.buf().text().to_string(), text);
    assert_eq!(editor.head(), head);
}

#[test]
fn collection_closer_resolves_opener_outside_excerpt_and_writes_back_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.rs");
    let original = format!(
        "fn f() {{\n{}            \n}}\n",
        "    // padding\n".repeat(20)
    );
    std::fs::write(&path, &original).unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), directory.path().to_owned());
    editor.open_fixture(&path).unwrap();
    let source = editor.current();
    let at = original.rfind("\n            \n").unwrap() + 13;
    let line = editor.buf().line_of(at) + 1;
    editor.open_picker(strop_picker::Kind::Search);
    editor.picker_items_fixture(vec![strop_picker::Item {
        badge: None,
        text: "source.rs: whitespace".into(),
        payload: strop_picker::Payload::Grep {
            location: strop_workspace::ResourceLocation::local(path),
            line,
            col: 1,
            match_len: 1,
            line_text: "            ".into(),
        },
    }]);
    editor.feed(strop_core::frontend_input::Key::CtrlO);
    let collection = editor.current();
    let excerpt = editor.collections[&collection]
        .excerpts
        .iter()
        .find(|excerpt| excerpt.source == source && at >= excerpt.start && at <= excerpt.end)
        .unwrap();
    assert!(
        excerpt.start > 0,
        "the matching opener is deliberately outside the excerpt"
    );
    let view = excerpt.view_start + at - excerpt.start;
    editor.set_head(view);
    editor.feed_text("i}<esc>");
    settle_indentation(&mut editor);
    assert!(editor
        .docs
        .get(source)
        .unwrap()
        .buf
        .text()
        .to_string()
        .ends_with("// padding\n}\n}\n"));
    editor.feed_text("u");
    assert_eq!(
        editor.docs.get(source).unwrap().buf.text().to_string(),
        original
    );
}

#[test]
fn changed_indent_settings_reject_a_pending_decision() {
    let text = format!("{}fn main() {{}}\n", "// padding\n".repeat(600));
    let mut editor = code_editor(&text, "large.rs");
    editor.set_head(text.rfind('}').unwrap());
    editor.feed_text("i<cr>x");
    editor.config.tab_size = 2;
    editor.reresolve_indents();
    settle_indentation(&mut editor);
    assert_eq!(editor.buf().text().to_string(), text);
    assert!(editor.message.contains("resolution cancelled"));
}

#[test]
fn unicode_backspace_stays_truthful_when_dot_replays_structural_enter() {
    let mut editor = Editor::new(Buffer::from_text("{}\n{}\n"));
    editor.set_head(1);
    editor.feed_text("i<cr>é<bs>x<esc>");
    settle_indentation(&mut editor);
    editor.set_head(editor.buf().text().to_string().rfind('}').unwrap());
    editor.feed_text(".");
    settle_indentation(&mut editor);
    assert_eq!(
        editor.buf().text().to_string(),
        "{\n    x\n}\n{\n    x\n}\n"
    );
}

#[test]
fn enter_splits_brackets_and_parentheses_with_intervening_whitespace() {
    for (text, expected) in [
        ("    values = [  ]\n", "    values = [\n        \n    ]\n"),
        ("    call( )\n", "    call(\n        \n    )\n"),
    ] {
        let mut editor = Editor::new(Buffer::from_text(text));
        editor.set_head(text.find(['[', '(']).unwrap() + 1);
        editor.feed_text("i<cr>");
        settle_indentation(&mut editor);
        assert_eq!(editor.buf().text().to_string(), expected);
    }
}

#[test]
fn large_source_enter_then_typed_closer_uses_incrementally_updated_syntax() {
    let text = format!("{}    fn main() {{", "// padding\n".repeat(600));
    let mut editor = code_editor(&text, "large.rs");
    editor.set_head(text.len());
    editor.feed_text("i<cr>}<esc>");
    settle_indentation(&mut editor);
    assert_eq!(editor.buf().text().to_string(), format!("{text}\n    }}"));
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), text);
}

#[test]
fn paired_enter_and_bracketed_literal_paste_repeat_without_reindenting_payload() {
    let original = "{}\n    {}\n";
    let mut editor = Editor::new(Buffer::from_text(original));
    editor.set_head(1);
    editor.feed_text("i<cr>");
    editor.paste_bracketed("{\n        }");
    editor.feed_text("tail<esc>");
    let first = editor.buf().text().to_string();
    assert_eq!(first, "{\n    {\n        }tail\n}\n    {}\n");
    editor.set_head(first.rfind('}').unwrap());
    editor.feed_text(".");
    settle_indentation(&mut editor);
    assert_eq!(
        editor.buf().text().to_string(),
        "{\n    {\n        }tail\n}\n    {\n        {\n        }tail\n    }\n"
    );
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), first);
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), original);
}
