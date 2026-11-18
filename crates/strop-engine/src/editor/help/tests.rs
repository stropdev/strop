use super::*;

#[test]
fn help_buffer_lists_every_section_and_searches() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.feed_text(":help\r");
    assert_eq!(e.buf().name.as_deref(), Some("help"));
    assert!(e.buf().readonly);
    let text = e.buf().text().to_string();
    let mut last = 0;
    for section in crate::keymap::SECTIONS {
        let heading = format!("─ {} ─", section.title);
        let at = text
            .find(&heading)
            .unwrap_or_else(|| panic!("missing {}", section.title));
        assert!(at > last, "{} out of curated order", section.title);
        last = at;
    }
    let missing: Vec<_> = [
        ":theme", ":ju", ":noh", ":format", ":rename", ":recover", ":qa", ":quitall", ":symbols",
        ":bNext", ":buffers",
    ]
    .into_iter()
    .filter(|command| !text.contains(*command))
    .collect();
    assert!(
        missing.is_empty(),
        "dispatchable Ex commands missing from generated help: {missing:?}"
    );
    // 0090 §4: the terminal section opens with how to open one and
    // names the one escape chord — discoverable from `?` alone.
    for row in ["space t", ":terminal", "ctrl-\\ ctrl-n", ":bd!"] {
        assert!(text.contains(row), "help lost the {row} terminal row");
    }
    assert!(
        !text.contains("ctrl-4"),
        "help still lists the deleted Ctrl-4 chord"
    );
    assert!(
        !text.contains("terminal-refresh"),
        "help still lists :terminal-refresh"
    );
    assert!(
        text.contains(":explain") && text.contains("LSP readiness/failures"),
        "help makes retained LSP failures discoverable"
    );
    // the language list says what it is: the server each family starts
    let servers = text.find("─ LSP servers ─").expect("LSP servers section");
    assert!(text[servers..].contains("rust          rust-analyzer · rustup"));
    assert!(!text.contains("─ Languages ─"));
    // Remote opens with how to get there
    let remote = text.find("─ Remote ─").expect("Remote section");
    let first = text[remote..].lines().nth(1).unwrap();
    assert!(first.contains("How to open"), "{first:?}");
    // it's a real buffer: / searches it
    e.feed_text("/undo-tree\r");
    assert!(e.head() > 0, "search moved into the help text");
    // q closes back to the file
    e.feed_text("q");
    assert_eq!(e.buf().name.as_deref(), None);
}

/// 0109 decision 7: one row per filter key, naming every list that
/// takes it, and one per kind value — all read from the schema and
/// the kind table, so the help cannot promise what a list ignores.
#[test]
fn help_query_language_rows_come_from_the_schema() {
    use strop_picker::query::schema::{surfaces_of, Field};
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.feed_text(":help\r");
    let text = e.buf().text().to_string();
    let start = text.find("─ Query language ─").expect("the section");
    let section = &text[start..];
    let section = match section.find("\n\n─ ") {
        Some(end) => &section[..end],
        None => section,
    };
    let rows: Vec<&str> = section.lines().skip(1).map(str::trim_start).collect();
    for field in Field::ALL {
        let row: Vec<&&str> = rows
            .iter()
            .filter(|row| row.starts_with(&format!("{} ", field.example())))
            .collect();
        assert_eq!(row.len(), 1, "{field:?}: {row:?}");
        assert!(row[0].contains(field.help()), "{row:?}");
        for surface in surfaces_of(field) {
            assert!(
                row[0].contains(surface),
                "{field:?} names {surface}: {row:?}"
            );
        }
    }
    for spec in strop_core::symbol_kind::KINDS {
        let key = format!("kind:{} ", spec.value);
        assert!(rows.iter().any(|row| row.starts_with(&key)), "{key}");
    }
    assert!(!section.contains("type:"), "type: is retired");
}

#[test]
fn space_question_opens_help_and_topics_reach_sections() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.feed_text(" ?");
    assert_eq!(e.buf().name.as_deref(), Some("help"));
    e.feed_text("q");
    e.feed_text(":help git\r");
    let line = e.buf().line_of(e.head());
    let text = e.buf().text().line(line).to_string();
    assert!(text.starts_with("─ Git ─"), "{text:?}");
    // vim's `:h` abbreviation reaches the same topic
    e.feed_text("q");
    e.feed_text(":h git\r");
    let line = e.buf().line_of(e.head());
    assert!(e.buf().text().line(line).to_string().starts_with("─ Git ─"));
    // a command by any of its names lands on its row
    for topic in ["write", ":w", "w", ":wri"] {
        e.feed_text("q");
        e.feed_text(&format!(":help {topic}\r"));
        let line = e.buf().line_of(e.head());
        let row = e.buf().text().line(line).to_string();
        assert!(
            row.trim_start().starts_with(":w[!] [file]"),
            "{topic}: {row:?}"
        );
    }
}

#[test]
fn closing_help_on_a_fresh_launch_returns_to_the_scratch() {
    let mut e = Editor::new(Buffer::from_text(""));
    e.feed_text(" ?");
    assert_eq!(e.buf().name.as_deref(), Some("help"));
    e.feed_text("q");
    assert!(!e.should_quit, "q on help must not quit the editor");
    assert_eq!(e.buf().name, None);
    assert_eq!(e.buf().path, None);
}
