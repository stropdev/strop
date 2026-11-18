use super::*;

fn help() -> Editor {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.feed_text(":help\r");
    assert!(e.help_page(e.current()).is_some());
    e
}

fn page(e: &Editor) -> HelpPage {
    e.help_page(e.current()).expect("help").clone()
}

fn line(e: &Editor, at: usize) -> String {
    e.buf().text().line(at).to_string().trim_end().to_owned()
}

fn cursor_line(e: &Editor) -> String {
    line(e, e.buf().line_of(e.head()))
}

/// A row's whole description: its first line's and its continuations'.
fn description(e: &Editor, page: &HelpPage, at: usize) -> String {
    let mut words = vec![line(e, at)
        .get(page::DESC_COLUMN..)
        .unwrap_or("")
        .to_owned()];
    let mut next = at + 1;
    while matches!(page.lines.get(next), Some(LineKind::Continuation { .. })) {
        words.push(line(e, next).trim_start().to_owned());
        next += 1;
    }
    words.retain(|word| !word.is_empty());
    words.join(" ")
}

#[test]
fn help_buffer_lists_every_section_and_searches() {
    let mut e = help();
    assert_eq!(e.buf().name.as_deref(), Some("help"));
    assert!(e.buf().readonly);
    let text = e.buf().text().to_string();
    let mut last = 0;
    for tab in content::TABS {
        let heading = format!("━ {} ━", tab.title);
        let at = text
            .find(&heading)
            .unwrap_or_else(|| panic!("missing {}", tab.title));
        assert!(at > last, "{} out of order", tab.title);
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
    let page = page(&e);
    let rust = (0..page.lines.len())
        .find(|&at| {
            matches!(&page.lines[at], LineKind::Entry { cell: Cell::Label, keys, .. }
                if line(&e, at).get(keys.clone()) == Some("rust"))
        })
        .expect("the rust row");
    assert!(description(&e, &page, rust).starts_with("rust-analyzer · rustup"));
    assert!(!text.contains("─ Languages ─"));
    // Remote opens with how to get there
    let remote = text.find("─ Remote ─").expect("Remote part");
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
    let e = help();
    let text = e.buf().text().to_string();
    let start = text.find("─ Query language ─").expect("the part");
    let part = &text[start..];
    let part = match part.find("\n\n") {
        Some(end) => &part[..end],
        None => part,
    };
    // a row and its wrapped rest read as one
    let mut rows: Vec<String> = Vec::new();
    for row in part.lines().skip(1) {
        match (
            row.starts_with(&" ".repeat(page::DESC_COLUMN)),
            rows.last_mut(),
        ) {
            (true, Some(last)) => {
                last.push(' ');
                last.push_str(row.trim_start());
            }
            _ => rows.push(row.trim_start().to_owned()),
        }
    }
    for field in Field::ALL {
        let row: Vec<&String> = rows
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
    assert!(!part.contains("type:"), "type: is retired");
}

#[test]
fn space_question_opens_help_and_topics_reach_sections() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.feed_text(" ?");
    assert_eq!(e.buf().name.as_deref(), Some("help"));
    e.feed_text("q");
    e.feed_text(":help git\r");
    assert_eq!(cursor_line(&e), "━ Git ━");
    // vim's `:h` abbreviation reaches the same topic
    e.feed_text("q");
    e.feed_text(":h git\r");
    assert_eq!(cursor_line(&e), "━ Git ━");
    // a command by any of its names lands on its row
    for topic in ["write", ":w", "w", ":wri"] {
        e.feed_text("q");
        e.feed_text(&format!(":help {topic}\r"));
        let row = cursor_line(&e);
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

/// §3.2: the model and the buffer agree line for line; every row has
/// a key cell but the one that lists the languages without a server.
#[test]
fn the_model_types_every_line() {
    let e = help();
    let page = page(&e);
    assert_eq!(page.lines.len(), e.buf().last_content_line() + 1);
    assert_eq!(page.lines[0], LineKind::Tabs);
    assert!(matches!(page.lines[1], LineKind::Hints { .. }));
    for (at, kind) in page.lines.iter().enumerate() {
        if let LineKind::Entry { keys, .. } = kind {
            assert!(
                !keys.is_empty() || line(&e, at).contains("no built-in server"),
                "line {at} has no key cell: {:?}",
                line(&e, at)
            );
        }
    }
    let names: Vec<&str> = page.tabs.iter().map(|tab| tab.title).collect();
    assert_eq!(
        names,
        ["Move", "Edit", "Visual", "Leader", "Windows", "Git", "Lists", "Commands", "More"]
    );
    for tab in &page.tabs {
        assert_eq!(&line(&e, 0)[tab.name.clone()], tab.title);
        assert_eq!(page.lines[tab.heading], LineKind::TabHeading);
        assert_eq!(line(&e, tab.heading), format!("━ {} ━", tab.title));
    }
}

/// §3.1: every keymap section and every command family is in exactly
/// one section of the page.
#[test]
fn every_section_and_family_is_in_exactly_one_tab() {
    let count = |held: &dyn Fn(&Source) -> bool| {
        content::TABS
            .iter()
            .flat_map(|tab| tab.parts.iter())
            .filter(|part| held(part))
            .count()
    };
    for section in crate::keymap::SECTIONS {
        let found = count(&|part| matches!(part, Source::Section(id) if *id == section.id));
        assert_eq!(found, 1, "{}", section.id);
    }
    for family in super::super::ex::Family::ALL {
        let found = count(&|part| matches!(part, Source::Family(f) if *f == family));
        assert_eq!(found, 1, "{family:?}");
    }
}

/// §3.2, §7: nothing is cut at 80 columns — no line wider than the
/// measure, and a key cell too wide for the column has its own line.
#[test]
fn every_line_fits_the_measure() {
    let e = help();
    let page = page(&e);
    for (at, kind) in page.lines.iter().enumerate() {
        let text = line(&e, at);
        assert!(
            text.chars().count() <= page::MEASURE,
            "line {at} is wider than the measure: {text:?}"
        );
        if let LineKind::Entry { keys, .. } = kind {
            if text[keys.clone()].chars().count() > page::KEY_COLUMN {
                assert_eq!(text.len(), keys.end, "line {at} shares its line: {text:?}");
                assert!(
                    matches!(page.lines.get(at + 1), Some(LineKind::Continuation { .. })),
                    "line {at}'s description is not on the next line"
                );
            }
        }
    }
    // the Windows rows that ran off an 80-column screen now wrap
    let text = e.buf().text().to_string();
    assert!(text.contains("  ctrl-w h / j / k / l / w / W / p\n"));
}

#[test]
fn topics_reach_sections_parts_and_commands() {
    let mut e = help();
    for (topic, landing) in [
        ("move", "━ Move ━"),
        ("Windows", "━ Windows ━"),
        ("moving", "─ Moving ─"),
        ("search", "─ Search ─"),
        ("panes", "━ Windows ━"),
        ("leader", "━ Leader ━"),
        ("query language", "─ Query language ─"),
        ("lsp servers", "─ LSP servers ─"),
    ] {
        e.feed_text(&format!(":help {topic}\r"));
        assert_eq!(cursor_line(&e), landing, "{topic}");
        let line = e.buf().line_of(e.head());
        assert_eq!(e.view().view_top, line, "{topic} lands as the top row");
    }
    e.feed_text(":help :w\r");
    assert!(cursor_line(&e).starts_with("  :w[!] [file]"));
    e.feed_text(":help nothing-here\r");
    assert!(e.message.contains("no help topic"), "{}", e.message);
}

/// §7: search is the buffer's and crosses sections.
#[test]
fn search_crosses_sections() {
    let mut e = help();
    let page = page(&e);
    e.feed_text("/half\\/full page\r");
    let first = e.buf().line_of(e.head());
    assert_eq!(page.tab_at(first, 0), 0, "{}", cursor_line(&e));
    assert!(cursor_line(&e).contains("half/full page scroll"));
    e.feed_text("/register\r");
    let mut tabs = vec![page.tab_at(e.buf().line_of(e.head()), 0)];
    for _ in 0..40 {
        e.feed_text("n");
        tabs.push(page.tab_at(e.buf().line_of(e.head()), 0));
    }
    tabs.dedup();
    assert!(tabs.len() > 2, "n stayed in one section: {tabs:?}");
}

/// §6: `]]` walks every section heading in order, each the top row,
/// and stops on the last; a count skips.
#[test]
fn section_motions_walk_the_headings() {
    let mut e = help();
    let page = page(&e);
    for tab in &page.tabs {
        e.feed_text("]]");
        assert_eq!(e.buf().line_of(e.head()), tab.heading, "{}", tab.title);
        assert_eq!(
            e.view().view_top,
            tab.heading,
            "{} is the top row",
            tab.title
        );
    }
    e.feed_text("]]");
    let last = page.tabs.last().unwrap().heading;
    assert_eq!(e.buf().line_of(e.head()), last, "]] stops on the last");
    e.feed_text("gg3]]");
    assert_eq!(cursor_line(&e), "━ Visual ━");
}

#[test]
fn section_motions_go_back_and_jumps_return() {
    let mut e = help();
    e.feed_text(":help git\r");
    e.feed_text("5j");
    let inside = e.buf().line_of(e.head());
    e.feed_text("[[");
    assert_eq!(
        cursor_line(&e),
        "━ Git ━",
        "[[ inside a section: its heading"
    );
    e.feed_text("[[");
    assert_eq!(cursor_line(&e), "━ Windows ━", "then the previous");
    e.feed_text("2[[");
    assert_eq!(cursor_line(&e), "━ Visual ━");
    e.feed_text("9[[");
    assert_eq!(
        e.buf().line_of(e.head()),
        0,
        "above the first heading: line 1"
    );
    // each jump records the place it left
    e.feed_text("<c-o>");
    assert_eq!(cursor_line(&e), "━ Visual ━");
    e.feed_text("<c-o><c-o><c-o>");
    assert_eq!(e.buf().line_of(e.head()), inside);
}

#[test]
fn enter_on_the_first_line_opens_a_section() {
    let mut e = help();
    let page = page(&e);
    let git = page.tabs.iter().find(|tab| tab.title == "Git").unwrap();
    e.feed_text(&format!("0{}l", git.name.start + 1));
    e.feed_text("<cr>");
    assert_eq!(cursor_line(&e), "━ Git ━");
    assert_eq!(e.view().view_top, git.heading);
    // on any other line enter is the line-down motion
    e.feed_text("3j");
    let line = e.buf().line_of(e.head());
    e.feed_text("<cr>");
    assert_eq!(e.buf().line_of(e.head()), line + 1);
}
