//! Cell-grid boundaries for the modeline: what survives when the row
//! runs out of cells. These assert layout behavior — position kept,
//! filename kept, printable cells, bounded width, the fixed chip — never
//! message wording (messages live in the echo row now).

use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use strop_core::Buffer;
use strop_git::GitContext;

use crate::editor::Editor;

use super::render;
use crate::render::clip::width;
use crate::render::tokens;

mod stability;

/// The modeline cells, including severity styling as well as symbols.
fn grid(editor: &Editor, columns: u16, rows: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(columns, rows)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            render(editor, &tokens(), frame, area)
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

/// The modeline row as one string, measured in display cells.
fn row(editor: &Editor, columns: u16, rows: u16) -> String {
    let buffer = grid(editor, columns, rows);
    let y = rows - 1;
    crate::headless::row_symbols(&buffer, y).collect()
}

fn editor(text: &str, path: Option<&str>) -> Editor {
    let mut buffer = Buffer::from_text(text);
    buffer.path = path.map(std::path::PathBuf::from);
    Editor::new_in(buffer, "/w".into())
}

fn attach(editor: &mut Editor, id: u64, outcome: strop_engine::editor::AttachDecision) {
    use strop_engine::editor::{AttachDecision, AttachRecord};
    let server = matches!(&outcome, AttachDecision::Attached).then(|| strop_lsp::ServerId::new(id));
    editor.fixture_lsp_attach(AttachRecord {
        ticket: strop_core::worker::WorkerId::new(id),
        server,
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: "/w".into(),
        target: strop_workspace::Filesystem::Local,
        outcome,
        layers: Vec::new(),
    });
}

fn fail(editor: &mut Editor, id: u64, reason: &str) {
    editor.handle_app_event(strop_engine::editor::events::AppEvent::Lsp(
        strop_lsp::LspEvent::Failed {
            server: strop_lsp::ServerId::new(id),
            name: "rust-analyzer".into(),
            hint: reason.into(),
        },
    ));
}

#[test]
fn the_chip_is_ten_cells_with_padding_in_every_mode() {
    let mut e = editor("alpha beta\n", Some("/w/f.rs"));
    let chip = |e: &Editor| -> String {
        let grid = grid(e, 80, 4);
        let bg = grid[(0, 3)].bg;
        (0..80)
            .take_while(|&x| grid[(x, 3)].bg == bg)
            .map(|x| grid[(x, 3)].symbol())
            .collect()
    };
    assert_eq!(chip(&e), "  NORMAL  ");
    e.feed_text("i");
    assert_eq!(chip(&e), "  INSERT  ");
    e.feed(strop_core::frontend_input::Key::Esc);
    e.feed_text("v");
    assert_eq!(chip(&e), "  VISUAL  ");
    e.feed_text("V");
    assert_eq!(chip(&e), "  V-LINE  ");
    e.feed(strop_core::frontend_input::Key::Esc);
    e.feed_text("d");
    assert_eq!(chip(&e), " PENDING  ");
    e.feed(strop_core::frontend_input::Key::Esc);
    e.feed_text(":");
    assert_eq!(chip(&e), " COMMAND  ");
    // the path starts at the same column whatever the mode
    e.feed(strop_core::frontend_input::Key::Esc);
    e.feed(strop_core::frontend_input::Key::Esc);
    let normal = row(&e, 80, 4);
    e.feed_text("i");
    let insert = row(&e, 80, 4);
    assert_eq!(
        normal.find("f.rs"),
        insert.find("f.rs"),
        "{normal:?} vs {insert:?}"
    );
}

#[test]
fn modeline_relativizes_to_the_longest_selected_project_root() {
    let mut editor = editor("fn main() {}\n", Some("/w/one/src/main.rs"));
    editor
        .select_root(strop_workspace::ResourceLocation::local("/w".into()))
        .unwrap();
    editor
        .select_root(strop_workspace::ResourceLocation::local("/w/one".into()))
        .unwrap();
    let row = row(&editor, 80, 4);
    assert!(row.contains("src/"), "{row:?}");
    assert!(row.contains("main.rs"), "{row:?}");
    assert!(!row.contains("one/src"), "{row:?}");
}

#[test]
fn a_long_path_cannot_crowd_out_the_spec_or_position() {
    let mut editor = editor(
        "alpha\nbeta\n",
        Some("/w/crates/a/very/deeply/nested/directory/tree/mod.rs"),
    );
    editor.feed_text("d");
    let row = row(&editor, 48, 4);
    assert!(row.contains("mod.rs"), "the filename survives: {row:?}");
    assert!(row.contains("PENDING"), "the chip survives: {row:?}");
    assert!(row.contains("1:1"), "the position survives: {row:?}");
    assert!(
        !row.contains("nested"),
        "the directory yields before anything essential: {row:?}"
    );
    assert!(width(&row) <= 48, "the row fits its cells: {row:?}");
}

#[test]
fn messages_never_enter_the_modeline() {
    let mut editor = editor("alpha\n", Some("/w/f.rs"));
    editor.set_message("saved the file");
    let row = row(&editor, 80, 4);
    assert!(!row.contains("saved"), "{row:?}");
    assert!(row.contains("f.rs"), "{row:?}");
}

/// U3: ready and starting services show nothing; a failure is the
/// 4-cell `LSP!` badge, a refusal `LSP?` — never a 30-cell sentence.
#[test]
fn service_badges_appear_only_for_problems() {
    let mut e = editor("fn main() {}\n", Some("/w/src/main.rs"));
    attach(&mut e, 40, strop_engine::editor::AttachDecision::Attached);
    let starting = row(&e, 100, 4);
    assert!(
        !starting.contains("LSP") && !starting.contains("lsp"),
        "{starting:?}"
    );

    e.handle_app_event(strop_engine::editor::events::AppEvent::Lsp(
        strop_lsp::LspEvent::Ready {
            server: strop_lsp::ServerId::new(40),
            name: "rust-analyzer".into(),
        },
    ));
    let ready = row(&e, 100, 4);
    assert!(!ready.contains("LSP"), "{ready:?}");

    fail(
        &mut e,
        40,
        "executable disappeared — reinstall with rustup component add rust-analyzer",
    );
    let failed = row(&e, 100, 4);
    assert!(failed.contains("LSP!"), "{failed:?}");
    assert!(
        !failed.contains("reinstall") && !failed.contains("rust-analyzer"),
        "{failed:?}"
    );
    let grid = grid(&e, 100, 4);
    let badge = failed.find("LSP!").unwrap() as u16;
    assert_eq!(grid[(badge, 3)].fg, tokens().error);

    let mut refused = editor("fn main() {}\n", Some("/w/src/main.rs"));
    attach(
        &mut refused,
        41,
        strop_engine::editor::AttachDecision::NotExecutable {
            command: "rust-analyzer".into(),
            reason: "not found on PATH".into(),
            hint: "rustup component add rust-analyzer".into(),
        },
    );
    let refused_row = row(&refused, 100, 4);
    assert!(refused_row.contains("LSP?"), "{refused_row:?}");
    assert!(
        !refused_row.contains("not found on PATH"),
        "{refused_row:?}"
    );
}

#[test]
fn narrow_failure_badge_survives_every_degradation() {
    let mut e = editor("fn main() {}\n", Some("/w/src/main.rs"));
    attach(&mut e, 42, strop_engine::editor::AttachDecision::Attached);
    fail(
        &mut e,
        42,
        "a very long actionable reason that must never monopolize the modeline: run rustup component add rust-analyzer",
    );
    for columns in [48, 24] {
        let row = row(&e, columns, 4);
        assert!(row.contains("LSP!"), "failure survives {columns}: {row:?}");
        assert!(row.contains("NORMAL"), "mode survives {columns}: {row:?}");
        assert!(row.contains("1:1"), "position survives {columns}: {row:?}");
        assert!(
            !row.contains("rustup"),
            "prose stays out {columns}: {row:?}"
        );
        assert!(
            width(&row) <= usize::from(columns),
            "row fits {columns}: {row:?}"
        );
    }
    for columns in [16, 12, 8, 4] {
        let shown = row(&e, columns, 4);
        assert!(shown.contains('!'), "failure mark at {columns}: {shown:?}");
        assert!(width(&shown) <= usize::from(columns), "{shown:?}");
    }
}

/// Rule 4: the indent segment appears only when the effective setting
/// differs from the configured default.
#[test]
fn indent_shows_only_when_it_differs_from_the_config() {
    let e = editor("x\n", None);
    let out = row(&e, 60, 4);
    assert!(!out.contains("Spaces:4"), "the default is silent: {out:?}");
    let mut e = editor("x\n", None);
    let mut config = e.config().clone();
    config.indent_style = strop_engine::config::IndentStyle::Tabs;
    e.set_config(config);
    e.reresolve_indents();
    let out = row(&e, 60, 4);
    assert!(
        !out.contains("Tabs:4"),
        "the configured style is the default: {out:?}"
    );
    e.feed_text(":tab-size 2\r");
    let out = row(&e, 60, 4);
    assert!(out.contains("Tabs:2"), "an override shows: {out:?}");
}

#[test]
fn diagnostics_show_only_when_non_zero() {
    let mut e = editor("fn main() {}\n", Some("/w/src/main.rs"));
    assert!(!row(&e, 80, 4).contains('●'));
    let diag = |col: usize, severity, message: &str| strop_lsp::ResolvedDiag {
        line: strop_core::id::LineIndex::new(0),
        col: strop_core::id::ByteColumn::new(col),
        severity,
        end_line: strop_core::id::LineIndex::new(0),
        end_col: strop_core::id::ByteColumn::new(col + 1),
        message: message.into(),
    };
    e.fixture_insert_diagnostics(
        e.current(),
        crate::editor::DocumentDiagnostics {
            revision: e.buf().revision(),
            items: vec![
                diag(0, strop_lsp::Severity::Error, "boom"),
                diag(3, strop_lsp::Severity::Warning, "hm"),
            ],
        },
    );
    let shown = row(&e, 80, 4);
    assert!(shown.contains("●1 ▲1"), "{shown:?}");
}

#[test]
fn wide_and_control_labels_stay_printable_whole_graphemes() {
    let mut editor = editor("a\n", None);
    editor.fixture_buf_mut().name = Some("界界\x1b㌔z\r".into());
    let wide = row(&editor, 30, 4);
    assert!(
        wide.contains("界界"),
        "wide graphemes are not split: {wide:?}"
    );
    assert!(
        wide.contains('�'),
        "controls become replacement cells: {wide:?}"
    );
    assert!(!wide.contains('\x1b') && !wide.contains('\r'), "{wide:?}");
    assert!(width(&wide) <= 30, "{wide:?}");

    let clipped = row(&editor, 27, 4);
    assert!(
        clipped.contains("界界"),
        "the name clips, not the position: {clipped:?}"
    );
    assert!(clipped.contains('…'), "clipping marks itself: {clipped:?}");
    assert!(clipped.contains("1:1"), "{clipped:?}");
    assert!(width(&clipped) <= 27, "{clipped:?}");
}

#[test]
fn narrow_rows_keep_the_mode_chip_and_position() {
    let mut editor = editor("hello\n", Some("/w/longfilename.rs"));
    editor.fixture_set_git(Some(GitContext {
        repo: strop_git::RepoTarget::Local {
            workdir: "/w".into(),
        },
        head_sha: None,
        head_branch: Some("main".into()),
        remotes: Vec::new(),
    }));
    editor.fixture_set_hunks_untracked(true);
    let row = row(&editor, 16, 4);
    assert!(row.contains("NORMAL"), "the mode stays legible: {row:?}");
    assert!(row.contains("1:1"), "the position stays legible: {row:?}");
    assert!(width(&row) <= 16, "{row:?}");
}

#[test]
fn percent_counts_content_lines_not_the_phantom_row() {
    let mut editor = editor("one\ntwo\nthree\n", None);
    let top = row(&editor, 60, 4);
    assert!(top.contains("33%"), "line 1 of 3: {top:?}");
    editor.feed_text("j");
    let mid = row(&editor, 60, 4);
    assert!(
        mid.contains("66%"),
        "line 2 of 3 — the phantom row is not in the denominator: {mid:?}"
    );
    editor.feed_text("G");
    let last = row(&editor, 60, 4);
    assert!(
        last.contains("100%"),
        "the last content row is 100%: {last:?}"
    );
}

#[test]
fn modeline_search_position_follows_repeat_navigation() {
    let mut editor = editor("zero hit\none hit\ntwo hit\nthree hit\n", None);
    editor.feed_text("/hit<cr>");
    editor.analysis_fixture();
    let first = row(&editor, 60, 4);
    assert!(first.contains("1/4"), "initial match: {first:?}");

    editor.feed_text("n");
    let second = row(&editor, 60, 4);
    assert!(second.contains("2/4"), "next match: {second:?}");

    editor.feed_text("N");
    let again = row(&editor, 60, 4);
    assert!(again.contains("1/4"), "reverse match: {again:?}");
}

#[test]
fn modeline_search_position_includes_zero_width_line_boundaries() {
    let mut line_start = editor("alpha\n\nbeta", None);
    line_start.feed_text("/^<cr>");
    line_start.analysis_fixture();
    let first = row(&line_start, 60, 4);
    assert!(first.contains("2/3"), "line-start match: {first:?}");
    line_start.feed_text("n");
    let following = row(&line_start, 60, 4);
    assert!(
        following.contains("3/3"),
        "next line-start match: {following:?}"
    );

    let mut line_end = editor("alpha\n\nbeta", None);
    line_end.feed_text("/$<cr>");
    line_end.analysis_fixture();
    let end = row(&line_end, 60, 4);
    assert!(end.contains("1/2"), "line-end match: {end:?}");

    let mut empty = editor("alpha\n\nbeta", None);
    empty.feed_text("/^$<cr>");
    empty.analysis_fixture();
    let row = row(&empty, 60, 4);
    assert!(row.contains("1/1"), "empty-line match: {row:?}");
}

#[test]
fn git_marks_and_flags_render_quietly() {
    let mut editor = editor("a\n", Some("/w/f.rs"));
    editor.fixture_buf_mut().mark_dirty();
    editor.fixture_buf_mut().readonly = true;
    editor.fixture_set_git(Some(GitContext {
        repo: strop_git::RepoTarget::Local {
            workdir: "/w".into(),
        },
        head_sha: Some("abc123".into()),
        head_branch: Some("main".into()),
        remotes: Vec::new(),
    }));
    editor.fixture_set_hunks_untracked(true);
    let row = row(&editor, 72, 4);
    assert!(
        row.contains("main*"),
        "branch quiet, worktree dirt marked: {row:?}"
    );
    assert!(
        row.contains("f.rs ●"),
        "the modified signal follows the name: {row:?}"
    );
    assert!(row.contains("[RO]"), "{row:?}");
    assert!(
        row.contains("  main*  f.rs"),
        "two-space separators: {row:?}"
    );
    assert!(!row.contains('│'), "no bar separators: {row:?}");
}

#[test]
fn the_pending_spec_takes_the_right_side_in_the_accent() {
    let mut editor = editor("alpha beta\n", Some("/w/f.rs"));
    editor.feed_text("d");
    let grid = grid(&editor, 80, 4);
    let text: String = (0..80).map(|x| grid[(x, 3)].symbol()).collect();
    let spec = text
        .find('d')
        .filter(|&at| at > 20)
        .expect("pending spec on the right");
    assert_eq!(grid[(spec as u16, 3)].fg, tokens().accent, "{text:?}");
    assert!(text.contains("PENDING"), "{text:?}");
}

#[test]
fn historical_delta_names_its_revision_and_file_not_the_worktree() {
    let mut editor = editor("a\n", Some("/w/live.rs"));
    editor.fixture_set_git(Some(GitContext {
        repo: strop_git::RepoTarget::Local {
            workdir: "/w".into(),
        },
        head_sha: None,
        head_branch: Some("main".into()),
        remotes: Vec::new(),
    }));
    editor.open_delta(
        "delta",
        crate::editor::PreparedDiff::new("src/reader.rs".into(), Vec::new()),
        None,
        Some(crate::editor::CommitFiles {
            repo: strop_git::RepoTarget::Local {
                workdir: "/w".into(),
            },
            sha: "abcdef1234567890".into(),
            files: crate::editor::PreparedFiles::new(
                strop_git::memory::CommitSummary {
                    sha: "abcdef1234567890".into(),
                    ..Default::default()
                },
                vec![strop_git::memory::ChangedFile {
                    path: "src/reader.rs".into(),
                    added: 1,
                    deleted: 0,
                    ..Default::default()
                }],
            ),
            current: "src/reader.rs".into(),
        }),
    );
    let row = row(&editor, 80, 4);
    assert!(row.contains("abcdef12"), "the revision is visible: {row}");
    assert!(
        row.contains("reader.rs"),
        "the actual file is visible: {row}"
    );
    assert!(
        !row.contains("main"),
        "history is not labelled as today's branch: {row}"
    );
}

/// 0090 §4: the modeline names a terminal by its `term://` buffer name —
/// the launch directory (or remote endpoint) is inside it — never with
/// `[RO]`, at every width.
#[test]
fn terminal_modeline_shows_the_term_name_at_every_width() {
    for (location, expected) in [
        (
            strop_workspace::ResourceLocation::local("/w/launch".into()),
            "term:///w/launch//1:sh",
        ),
        (
            strop_workspace::ResourceLocation::remote(
                strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap(),
                "/srv/launch".into(),
            ),
            "term://ssh://fixture/srv/launch//1:sh",
        ),
    ] {
        let mut editor = editor("origin", None);
        editor.terminal_fixture_at(
            location,
            &[("safe", strop_terminal::model::Style::default())],
            strop_terminal::model::Phase::Running,
        );
        for (columns, rows) in [(100, 30), (80, 24), (48, 16)] {
            let shown = row(&editor, columns, rows);
            assert!(shown.contains("NORMAL"), "{columns}x{rows}: {shown}");
            assert!(!shown.contains("[RO]"), "{columns}x{rows}: {shown}");
            if columns >= 80 {
                assert!(shown.contains(expected), "{columns}x{rows}: {shown}");
            } else {
                assert!(shown.contains("term://"), "{columns}x{rows}: {shown}");
            }
        }
    }
}

/// Terminal hints are whole chords (rule 10): the one escape chord in
/// TERMINAL mode, nothing in NORMAL on a live session, the phase once it
/// ended; every width stays cell-bounded.
#[test]
fn terminal_hints_survive_golden_cell_sizes() {
    let mut editor = editor("origin", None);
    let document = editor.terminal_fixture(
        &[("safe", strop_terminal::model::Style::default())],
        strop_terminal::model::Phase::Running,
    );
    editor.feed_text("i");
    for (columns, rows) in [(100, 30), (80, 24), (48, 16)] {
        let shown = row(&editor, columns, rows);
        assert!(shown.contains("TERMINAL"), "{columns}x{rows}: {shown}");
        if columns >= 80 {
            assert!(
                shown.contains(" ctrl-\\ ctrl-n  NORMAL"),
                "{columns}x{rows}: {shown}"
            );
        }
        assert!(!shown.contains("F2"), "{columns}x{rows}: {shown}");
        assert!(
            !shown.contains("ctrl-\\ ctrl-n alt"),
            "no clipped chords: {shown}"
        );
    }
    let ctrl = |editor: &mut Editor, ch: char| {
        use strop_core::frontend_input::{Input, KeyCode, KeyEvent, Modifiers};
        let code = KeyCode::Char(ch);
        editor.handle_app_event(strop_engine::editor::events::AppEvent::Input(Input::Key(
            KeyEvent {
                code,
                modifiers: Modifiers {
                    control: true,
                    ..Default::default()
                },
                ..KeyEvent::press(code)
            },
        )));
    };
    ctrl(&mut editor, '\\');
    ctrl(&mut editor, 'n');
    editor.terminal_fixture_publish(
        document,
        &[
            ("safe", strop_terminal::model::Style::default()),
            ("new", strop_terminal::model::Style::default()),
        ],
    );
    editor.feed_text("l");
    for (columns, rows) in [(100, 30), (80, 24), (48, 16)] {
        let shown = row(&editor, columns, rows);
        assert!(shown.contains("NORMAL"), "{columns}x{rows}: {shown}");
        assert!(!shown.contains("out+"), "{columns}x{rows}: {shown}");
        assert!(!shown.contains("input"), "{columns}x{rows}: {shown}");
    }
    editor.terminal_fixture_exit(document, 0);
    for (columns, rows) in [(100, 30), (80, 24), (48, 16)] {
        let shown = row(&editor, columns, rows);
        assert!(shown.contains("exited"), "{columns}x{rows}: {shown}");
    }
    for (columns, rows) in [(24, 8), (8, 2), (1, 1)] {
        let shown = row(&editor, columns, rows);
        assert!(
            width(&shown) <= usize::from(columns),
            "{columns}x{rows}: {shown}"
        );
    }
}

#[test]
fn modeline_respects_its_rectangle_and_empty_areas() {
    let editor = editor("x\n", None);
    let mut terminal = Terminal::new(TestBackend::new(16, 4)).unwrap();
    terminal
        .draw(|frame| {
            for cell in &mut frame.buffer_mut().content {
                cell.set_symbol(".");
            }
            let t = tokens();
            render(&editor, &t, frame, Rect::new(0, 0, 0, 4));
            render(&editor, &t, frame, Rect::new(0, 0, 16, 0));
            render(&editor, &t, frame, Rect::new(2, 1, 10, 2));
        })
        .unwrap();
    let grid = terminal.backend().buffer();
    for y in [0, 1, 3] {
        assert!((0..16).all(|x| grid[(x, y)].symbol() == "."));
    }
    for x in [0, 1, 12, 13, 14, 15] {
        assert_eq!(grid[(x, 2)].symbol(), ".");
    }
    let position: String = (2..12).map(|x| grid[(x, 2)].symbol()).collect();
    assert!(
        position.contains("1:1"),
        "position stays inside the supplied rectangle"
    );
}
