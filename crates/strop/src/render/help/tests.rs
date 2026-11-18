use super::*;
use crate::editor::help::Tab;
use crate::editor::Editor;

fn tokens() -> Tokens {
    Tokens::new(strop_core::theme::ThemeId::Hone)
}

fn joined(spans: &[Span<'static>]) -> String {
    spans.iter().map(|span| span.content.as_ref()).collect()
}

/// The help as opened: its page and its lines.
fn help() -> (HelpPage, Vec<String>) {
    let mut editor = Editor::new(strop_core::Buffer::from_text("x\n"));
    editor.feed_text(":help\r");
    let page = editor.help_page(editor.current()).expect("help").clone();
    let lines = (0..page.lines.len())
        .map(|at| {
            let text = editor.buf().text().line(at).to_string();
            text.trim_end_matches('\n').to_owned()
        })
        .collect();
    (page, lines)
}

/// The span holding `needle` on a line.
fn span<'a>(spans: &'a [Span<'static>], needle: &str) -> &'a Span<'static> {
    spans
        .iter()
        .find(|span| span.content == needle)
        .unwrap_or_else(|| panic!("no {needle:?} span in {spans:?}"))
}

/// Every line's spans are its exact bytes (a heading's rule runs on to
/// the pane's width), and nothing sits on the raised plane.
#[test]
fn every_line_keeps_its_bytes() {
    let t = tokens();
    let (page, lines) = help();
    for (at, text) in lines.iter().enumerate() {
        let spans = line_spans(&t, &page, at, text, 80, 0);
        let painted = joined(&spans);
        match page.lines[at] {
            LineKind::Heading | LineKind::TabHeading => {
                assert!(painted.starts_with(text.as_str()), "line {at}: {painted:?}")
            }
            _ => assert_eq!(&painted, text, "line {at}"),
        }
        assert!(
            spans
                .iter()
                .all(|span| span.style.bg.is_none() || matches!(page.lines[at], LineKind::Tabs)),
            "line {at} paints a background"
        );
    }
}

#[test]
fn each_token_wears_its_role() {
    let t = tokens();
    let (page, lines) = help();
    let row = |cell: &str| {
        let at = (0..lines.len())
            .find(|&at| {
                matches!(&page.lines[at], LineKind::Entry { keys, .. }
                    if lines[at].get(keys.clone()) == Some(cell))
            })
            .unwrap_or_else(|| panic!("no {cell:?} row"));
        line_spans(&t, &page, at, &lines[at], 80, 0)
    };
    let bold = |span: &Span<'_>| span.style.add_modifier.contains(Modifier::BOLD);

    let spans = row("space w v / -");
    assert_eq!(span(&spans, "space").style.fg, Some(t.syn.link));
    assert!(bold(span(&spans, "space")));
    assert_eq!(span(&spans, "w").style.fg, Some(t.accent));
    assert!(bold(span(&spans, "w")));
    assert_eq!(span(&spans, "/").style.fg, Some(t.muted));
    assert_eq!(spans.last().map(|span| span.style.fg), Some(Some(t.text)));

    let spans = row("ctrl-w =");
    assert_eq!(span(&spans, "ctrl-w").style.fg, Some(t.syn.r#type));
    assert!(bold(span(&spans, "ctrl-w")));

    let spans = row("f<c> F<c> t<c> T<c>");
    assert_eq!(span(&spans, "<c>").style.fg, Some(t.syn.string));
    assert!(!bold(span(&spans, "<c>")));

    let spans = row(":bd[elete][!] [number]");
    assert_eq!(span(&spans, ":bd").style.fg, Some(t.accent));
    assert!(bold(span(&spans, ":bd")));
    assert_eq!(span(&spans, "elete").style.fg, Some(t.accent));
    assert!(!bold(span(&spans, "elete")));
    assert_eq!(span(&spans, "[").style.fg, Some(t.muted));
    assert_eq!(span(&spans, "number").style.fg, Some(t.syn.string));

    let spans = row("language:rust");
    assert_eq!(span(&spans, "language:").style.fg, Some(t.accent));
    assert_eq!(span(&spans, "rust").style.fg, Some(t.syn.string));

    let spans = row("detection");
    assert_eq!(span(&spans, "detection").style.fg, Some(t.secondary));
}

#[test]
fn headings_and_hints_wear_theirs() {
    let t = tokens();
    let (page, lines) = help();
    let move_heading = page.tabs[0].heading;
    let spans = line_spans(&t, &page, move_heading, &lines[move_heading], 80, 0);
    assert_eq!(span(&spans, "Move").style.fg, Some(t.accent));
    assert_eq!(spans[0].style.fg, Some(t.faint));
    assert_eq!(
        joined(&spans).chars().count(),
        80,
        "the rule runs to the edge"
    );

    let part = move_heading + 2;
    assert_eq!(page.lines[part], LineKind::Heading);
    let spans = line_spans(&t, &page, part, &lines[part], 80, 0);
    assert_eq!(span(&spans, "Moving").style.fg, Some(t.text));

    let spans = line_spans(&t, &page, 1, &lines[1], 80, 0);
    let version = format!("strop {}", env!("CARGO_PKG_VERSION"));
    assert_eq!(span(&spans, &version).style.fg, Some(t.text));
    assert_eq!(span(&spans, "]]").style.fg, Some(t.accent));
    assert_eq!(span(&spans, "enter").style.fg, Some(t.syn.link));
    assert_eq!(span(&spans, " section · ").style.fg, Some(t.muted));
}

/// The active section is a filled chip, and it moves with the cursor
/// column along the first line.
#[test]
fn the_active_tab_is_the_chip() {
    let t = tokens();
    let (page, lines) = help();
    let git = page.tabs.iter().position(|tab| tab.title == "Git").unwrap();
    let Tab { name, .. } = &page.tabs[git];
    assert_eq!(page.tab_at(0, name.start), git);
    assert_eq!(page.tab_at(0, name.end), git, "the gap after a name is its");
    assert_eq!(page.tab_at(0, 0), 0);
    let spans = line_spans(&t, &page, 0, &lines[0], 80, git);
    let chip = span(&spans, " Git ");
    assert_eq!(chip.style.fg, Some(t.on_accent));
    assert_eq!(chip.style.bg, Some(t.accent));
    assert_eq!(
        span(&spans, &lines[0][..name.start - 1]).style.fg,
        Some(t.muted)
    );
    let last = page.tabs.len() - 1;
    let spans = line_spans(&t, &page, 0, &lines[0], 80, last);
    assert_eq!(span(&spans, " More ").style.bg, Some(t.accent));
}

#[test]
fn a_planned_row_has_no_accent() {
    let t = tokens();
    let page = HelpPage {
        tabs: Vec::new(),
        lines: vec![
            LineKind::Entry {
                cell: Cell::Keys,
                keys: 2..11,
                planned: true,
            },
            LineKind::Continuation { planned: true },
        ],
    };
    for (at, text) in ["  space j   jumplist picker", "             (soon)"]
        .iter()
        .enumerate()
    {
        let spans = line_spans(&t, &page, at, text, 80, 0);
        assert_eq!(joined(&spans), *text);
        assert!(spans.iter().all(|span| span.style.fg == Some(t.muted)));
        assert!(spans[0].style.add_modifier.contains(Modifier::ITALIC));
    }
}
