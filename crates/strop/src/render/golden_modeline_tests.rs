//! Golden frames for the modeline and the cards beside it (0098 §5):
//! six states at four widths, twelve rows each, in `hone`. Every scene
//! carries a long branch, a long path and three diagnostics, so each
//! width shows what gives way.
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use strop_core::Buffer;
use strop_git::GitContext;

use super::golden::assert_golden;
use crate::editor::Editor;

const WIDTHS: [u16; 4] = [48, 80, 120, 200];
const ROWS: u16 = 12;

fn editor() -> Editor {
    let text: String = (0..40)
        .map(|line| format!("let value_{line} = alpha + beta * {line};\n"))
        .collect();
    let mut buffer = Buffer::from_text(&text);
    buffer.path = Some("/w/crates/strop/src/render/statusline/layout.rs".into());
    let mut editor = Editor::new_in(buffer, "/w".into());
    editor.fixture_set_git(Some(GitContext {
        repo: strop_git::RepoTarget::Local {
            workdir: "/w".into(),
        },
        head_sha: None,
        head_branch: Some("feature/modeline-design".into()),
        remotes: Vec::new(),
    }));
    editor.fixture_set_hunks_untracked(true);
    let diagnostic = |line: usize, severity| strop_lsp::ResolvedDiag {
        line: strop_core::id::LineIndex::new(line),
        col: strop_core::id::ByteColumn::new(4),
        severity,
        end_line: strop_core::id::LineIndex::new(line),
        end_col: strop_core::id::ByteColumn::new(11),
        message: "unused variable".into(),
    };
    editor.fixture_insert_diagnostics(
        editor.current(),
        crate::editor::DocumentDiagnostics {
            revision: editor.buf().revision(),
            items: vec![
                diagnostic(30, strop_lsp::Severity::Error),
                diagnostic(31, strop_lsp::Severity::Warning),
                diagnostic(32, strop_lsp::Severity::Warning),
            ],
        },
    );
    editor
}

fn each(name: &str, scene: impl Fn(&mut Editor)) {
    for columns in WIDTHS {
        let mut editor = editor();
        scene(&mut editor);
        let mut terminal = Terminal::new(TestBackend::new(columns, ROWS)).unwrap();
        terminal
            .draw(|frame| crate::render::paint(&mut editor, frame))
            .unwrap();
        assert_golden(
            &format!("modeline-{name}-hone-{columns}x{ROWS}"),
            terminal.backend().buffer(),
        );
    }
}

#[test]
fn normal() {
    each("normal", |editor| editor.feed_text("9j"));
}

/// A count and an operator pend: the keys sit in their slot.
#[test]
fn pending_operator() {
    each("pending", |editor| editor.feed_text("\"a3d"));
}

/// The leader pends: the key card hangs one row above the modeline.
#[test]
fn leader() {
    each("leader", |editor| editor.feed_text(" g"));
}

#[test]
fn macro_recording() {
    each("recording", |editor| editor.feed_text("qaj"));
}

/// `:w` with its completions: one text column inside the card.
#[test]
fn command_line() {
    each("command", |editor| editor.feed_text(":w"));
}

#[test]
fn search_count() {
    each("search", |editor| {
        editor.feed_text("/beta<cr>nn");
        editor.analysis_fixture();
    });
}
