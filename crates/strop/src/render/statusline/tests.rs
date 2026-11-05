//! Cell-grid boundaries for the modeline: what survives when the row
//! runs out of cells. These assert layout behavior — position kept,
//! filename kept, printable cells, bounded width — never message
//! wording.

use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use strop_core::Buffer;
use strop_git::GitContext;

use crate::editor::Editor;

use super::render;
use crate::render::clip::width;

/// The modeline cells, including severity styling as well as symbols.
fn grid(editor: &Editor, columns: u16, rows: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(columns, rows)).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            render(editor, frame, area)
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

fn assert_mark_color(editor: &Editor, columns: u16, severity: strop_lsp::Severity) {
    let buffer = grid(editor, columns, 4);
    let color = crate::render::severity_color(severity);
    assert!(
        buffer
            .content
            .iter()
            .any(|cell| cell.symbol() == "!" && cell.fg == color),
        "severity-colored feedback marker at {columns} cells"
    );
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
fn a_long_path_cannot_crowd_out_status_or_position() {
    let mut editor = editor(
        "alpha\nbeta\n",
        Some("/w/crates/a/very/deeply/nested/directory/tree/mod.rs"),
    );
    editor.set_message("saved the file");
    let row = row(&editor, 48, 4);
    assert!(row.contains("mod.rs"), "the filename survives: {row:?}");
    assert!(
        row.contains("saved the file"),
        "the status survives: {row:?}"
    );
    assert!(row.contains("1:1"), "the position survives: {row:?}");
    assert!(
        !row.contains("nested"),
        "the directory yields before anything essential: {row:?}"
    );
    assert!(width(&row) <= 48, "the row fits its cells: {row:?}");
}

#[test]
fn lifecycle_badges_are_scoped_and_visibly_distinct() {
    let mut e = editor("fn main() {}\n", Some("/w/src/main.rs"));
    attach(&mut e, 40, strop_engine::editor::AttachDecision::Attached);
    let starting = row(&e, 100, 4);
    assert!(
        starting.contains("lsp rust-analyzer@w starting"),
        "{starting:?}"
    );

    e.handle_app_event(strop_engine::editor::events::AppEvent::Lsp(
        strop_lsp::LspEvent::Ready {
            server: strop_lsp::ServerId::new(40),
            name: "rust-analyzer".into(),
        },
    ));
    let ready = row(&e, 100, 4);
    assert!(ready.contains("lsp rust-analyzer@w ready"), "{ready:?}");
    assert_ne!(starting, ready);

    fail(
        &mut e,
        40,
        "executable disappeared — reinstall with rustup component add rust-analyzer",
    );
    let failed = row(&e, 100, 4);
    assert!(failed.contains("lsp rust-analyzer@w failed"), "{failed:?}");
    assert!(!failed.contains("reinstall"), "{failed:?}");

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
    assert!(refused_row.contains("refused"), "{refused_row:?}");
    assert!(refused_row.contains("lsp rust@"), "{refused_row:?}");
    assert!(
        !refused_row.contains("not found on PATH"),
        "{refused_row:?}"
    );
}

#[test]
fn narrow_failure_badge_survives_without_long_prose() {
    let mut e = editor("fn main() {}\n", Some("/w/src/main.rs"));
    attach(&mut e, 42, strop_engine::editor::AttachDecision::Attached);
    fail(
        &mut e,
        42,
        "a very long actionable reason that must never monopolize the modeline: run rustup component add rust-analyzer",
    );
    for columns in [48, 24] {
        let row = row(&e, columns, 4);
        assert!(
            row.contains("LSP!") || row.contains("failed"),
            "failure survives {columns}: {row:?}"
        );
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
}

#[test]
fn presented_failure_keeps_a_severity_marker_through_bare_layout() {
    let mut e = editor("unsaved\n", None);
    e.feed_text(":w<cr>");
    assert_eq!(
        e.presented_feedback().map(|record| record.severity()),
        Some(strop_engine::editor::feedback::FeedbackSeverity::Failure)
    );
    let wide = row(&e, 80, 4);
    assert!(wide.contains("local save failed"), "{wide:?}");
    assert!(wide.contains("Space a actions"), "{wide:?}");

    for columns in [16, 12, 8, 4] {
        let shown = row(&e, columns, 4);
        assert!(shown.contains('!'), "failure at {columns}: {shown:?}");
        assert!(!shown.contains("write failed"), "{columns}: {shown:?}");
        assert!(width(&shown) <= usize::from(columns), "{shown:?}");
        assert_mark_color(&e, columns, strop_lsp::Severity::Error);
    }
}

#[test]
fn presented_warning_keeps_a_severity_marker_through_bare_layout() {
    let mut e = editor("origin", None);
    let document = e.terminal_fixture(
        &[("safe", strop_terminal::model::Style::default())],
        strop_terminal::model::Phase::Running,
    );
    e.terminal_fixture_hold_paste(document, 7);
    assert_eq!(
        e.presented_feedback().map(|record| record.severity()),
        Some(strop_engine::editor::feedback::FeedbackSeverity::Warning)
    );
    let wide = row(&e, 100, 4);
    assert!(wide.contains("i input"), "terminal action stays: {wide:?}");
    assert!(wide.contains("paste+"), "{wide:?}");
    assert!(wide.contains("Space a Review/Send/Discard"), "{wide:?}");

    for columns in [16, 12, 8, 4] {
        let shown = row(&e, columns, 4);
        assert!(shown.contains('!'), "warning at {columns}: {shown:?}");
        assert!(!shown.contains("paste held"), "{columns}: {shown:?}");
        assert!(width(&shown) <= usize::from(columns), "{shown:?}");
        assert_mark_color(&e, columns, strop_lsp::Severity::Warning);
    }
}

#[test]
fn ephemeral_messages_never_acquire_a_feedback_marker() {
    let mut e = editor("saved\n", None);
    e.set_message("saved the file");
    assert!(e.presented_feedback().is_none());
    assert!(row(&e, 80, 4).contains("saved the file"));
    for columns in [16, 12, 8, 4] {
        let shown = row(&e, columns, 4);
        assert!(!shown.contains('!'), "message at {columns}: {shown:?}");
        assert!(width(&shown) <= usize::from(columns), "{shown:?}");
    }
}

#[test]
fn bare_layout_retains_feedback_alongside_lsp_problem_badge() {
    let mut e = editor("unsaved\n", None);
    attach(&mut e, 44, strop_engine::editor::AttachDecision::Attached);
    fail(&mut e, 44, "language server unavailable");
    e.feed_text(":w<cr>");
    let shown = row(&e, 16, 4);
    assert!(shown.contains("LSP!"), "{shown:?}");
    assert_eq!(shown.matches('!').count(), 2, "{shown:?}");
    assert_mark_color(&e, 16, strop_lsp::Severity::Error);
    assert!(width(&shown) <= 16, "{shown:?}");
}

#[test]
fn save_failure_keeps_transient_precedence_over_service_detail() {
    let mut e = editor("fn main() {}\n", Some("/w/src/main.rs"));
    attach(&mut e, 43, strop_engine::editor::AttachDecision::Attached);
    fail(&mut e, 43, "server hint retained only in explain");
    e.set_message("write failed: disk full");
    let row = row(&e, 80, 4);
    assert!(row.contains("write failed: disk full"), "{row:?}");
    assert!(row.contains("failed") || row.contains("LSP!"), "{row:?}");
    assert!(!row.contains("server hint"), "{row:?}");
}

/// 0051 R08: the modeline carries the effective indent setting.
#[test]
fn modeline_shows_the_effective_indent() {
    let e = editor("x\n", None);
    let out = row(&e, 60, 4);
    assert!(out.contains("Spaces:4"), "the default: {out:?}");
    let mut e = editor("x\n", None);
    let mut config = e.config().clone();
    config.indent_style = strop_engine::config::IndentStyle::Tabs;
    e.set_config(config);
    e.reresolve_indents();
    let out = row(&e, 60, 4);
    assert!(out.contains("Tabs:4"), "the configured style: {out:?}");
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

    let clipped = row(&editor, 22, 4);
    assert!(
        clipped.contains("界界"),
        "the name clips, not the position: {clipped:?}"
    );
    assert!(clipped.contains('…'), "clipping marks itself: {clipped:?}");
    assert!(clipped.contains("1:1"), "{clipped:?}");
    assert!(width(&clipped) <= 22, "{clipped:?}");
}

#[test]
fn narrow_rows_keep_the_mode_accent_and_position() {
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
fn git_marks_and_flags_render_quietly_beside_the_message() {
    let mut editor = editor("a\n", Some("/w/f.rs"));
    editor.fixture_buf_mut().dirty = true;
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
    editor.set_message("wrote f.rs");
    // 72 cells: the 0051 R08 indent segment joined the required set
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
        row.contains("wrote f.rs"),
        "the message wins the status slot: {row:?}"
    );
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
                "abcdef1234567890".into(),
                vec![strop_git::memory::ChangedFile {
                    path: "src/reader.rs".into(),
                    added: 1,
                    deleted: 0,
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

#[test]
fn terminal_modeline_names_its_launch_namespace_at_every_width() {
    for (location, expected_namespace) in [
        (
            strop_workspace::ResourceLocation::local("/w/launch".into()),
            "local",
        ),
        (
            strop_workspace::ResourceLocation::remote(
                strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap(),
                "/srv/launch".into(),
            ),
            "ssh://fixture",
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
            if columns >= 80 {
                assert!(
                    shown.contains(expected_namespace),
                    "{columns}x{rows}: {shown}"
                );
                assert!(shown.contains("launch"), "{columns}x{rows}: {shown}");
            } else if expected_namespace == "local" {
                assert!(shown.contains("local"), "{columns}x{rows}: {shown}");
            } else {
                assert!(shown.contains("ssh://"), "{columns}x{rows}: {shown}");
                assert!(!shown.contains("local"), "{columns}x{rows}: {shown}");
            }
        }
    }
}

#[test]
fn terminal_owner_actions_and_attention_survive_golden_cell_sizes() {
    let mut editor = editor("origin", None);
    let document = editor.terminal_fixture(
        &[("safe", strop_terminal::model::Style::default())],
        strop_terminal::model::Phase::Running,
    );
    editor.feed_text("i");
    for (columns, rows) in [(100, 30), (80, 24), (48, 16)] {
        let shown = row(&editor, columns, rows);
        assert!(shown.contains("F2 inspect"), "{columns}x{rows}: {shown}");
    }
    editor.handle_app_event(strop_engine::editor::events::AppEvent::Input(
        strop_core::frontend_input::Input::Key(strop_core::frontend_input::KeyEvent::press(
            strop_core::frontend_input::KeyCode::Function(2),
        )),
    ));
    editor.terminal_fixture_publish(
        document,
        &[
            ("safe", strop_terminal::model::Style::default()),
            ("new", strop_terminal::model::Style::default()),
        ],
    );
    editor.terminal_fixture_hold_paste(document, 7);
    editor.feed_text("l");
    assert!(editor.terminal_has_new_output(document));
    assert!(editor.terminal_paste_held(document));
    for (columns, rows) in [(100, 30), (80, 24), (48, 16)] {
        let shown = row(&editor, columns, rows);
        assert!(shown.contains("SNAPSHOT"), "{columns}x{rows}: {shown}");
        assert!(shown.contains("i input"), "{columns}x{rows}: {shown}");
        assert!(shown.contains("out+"), "{columns}x{rows}: {shown}");
        if columns >= 80 {
            assert!(shown.contains("Space a Refresh"), "{shown}");
            assert!(shown.contains("paste+"), "{shown}");
        }
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
            render(&editor, frame, Rect::new(0, 0, 0, 4));
            render(&editor, frame, Rect::new(0, 0, 16, 0));
            render(&editor, frame, Rect::new(2, 1, 10, 2));
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
