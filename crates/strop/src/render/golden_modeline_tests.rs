//! Golden frames for the modeline and the cards beside it (0098 §5):
//! six states at four widths, twelve rows each, in `hone`. Every scene
//! carries a long branch, a long path and three diagnostics, so each
//! width shows what gives way. The `modeline-scope-*` family (0107
//! §10.4) adds the breadcrumb: a small Rust file through the real
//! outline.
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use strop_core::Buffer;
use strop_git::GitContext;

use super::golden::assert_golden;
use crate::editor::Editor;

const WIDTHS: [u16; 4] = [48, 80, 120, 200];
const ROWS: u16 = 12;

/// A file with no declarations: no breadcrumb.
fn editor() -> Editor {
    let text: String = (0..40)
        .map(|line| format!("let value_{line} = alpha + beta * {line};\n"))
        .collect();
    editor_with(&text, [30, 31, 32])
}

/// The breadcrumb's file: `impl Modeline › fit`, `allot`, and
/// `tests › impl Fixture › a_long_test_function` (43 cells).
const RUST: &str = "\
//! The modeline's width plan.
pub(super) struct Modeline {
    chip: String,
}

impl Modeline {
    pub(super) fn fit(&self, cells: usize) -> usize {
        let sizes = self.sizes();
        cells.min(sizes)
    }

    fn hints_shown(&self) -> usize {
        0
    }
}

pub(super) fn allot(sizes: usize, cells: usize) -> usize {
    sizes.min(cells)
}

mod tests {
    struct Fixture;

    impl Fixture {
        fn a_long_test_function() {
            let value = 1;
        }
    }
}
";

fn editor_with(text: &str, diagnostics: [usize; 3]) -> Editor {
    let mut buffer = Buffer::from_text(text);
    buffer.path = Some("/w/crates/strop/src/render/statusline/layout.rs".into());
    let mut editor = Editor::new_in(buffer, "/w".into());
    editor.set_theme(strop_core::theme::ThemeId::Hone);
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
                diagnostic(diagnostics[0], strop_lsp::Severity::Error),
                diagnostic(diagnostics[1], strop_lsp::Severity::Warning),
                diagnostic(diagnostics[2], strop_lsp::Severity::Warning),
            ],
        },
    );
    editor
}

fn each(name: &str, scene: impl Fn(&mut Editor)) {
    each_in(name, editor, scene);
}

/// The breadcrumb's scenes: the outline has landed and the breadcrumb
/// follows it, as after the first event in a live session.
fn scoped(name: &str, scene: impl Fn(&mut Editor)) {
    let outlined = || {
        let mut editor = editor_with(RUST, [7, 8, 17]);
        editor.wait_symbols();
        editor
    };
    each_in(&format!("scope-{name}"), outlined, scene);
}

fn each_in(name: &str, make: impl Fn() -> Editor, scene: impl Fn(&mut Editor)) {
    for columns in WIDTHS {
        let mut editor = make();
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

#[test]
fn scope_in_a_method() {
    scoped("method", |editor| editor.feed_text("8G"));
}

/// Between two declarations the slot is blank and keeps its cells.
#[test]
fn scope_at_top_level() {
    scoped("top", |editor| editor.feed_text("5G"));
}

#[test]
fn scope_deep_and_cut() {
    scoped("deep", |editor| editor.feed_text("26G"));
}

#[test]
fn scope_beside_pending_keys() {
    scoped("pending", |editor| editor.feed_text("8G\"a3d"));
}

/// Typing a line inside `fit`: the breadcrumb is the carried one.
#[test]
fn scope_while_typing() {
    scoped("insert", |editor| {
        editor.feed_text("9Golet more = 1;");
        editor.wait_resolution();
    });
}
