//! Golden frames for every surface in 0088 §5, in `hone` and `vellum`
//! at 140×40 and 80×24. Scenes drive the engine through its public
//! surface and paint through the same `paint` the other render tests
//! use; the goldens live beside this file under `golden/`.
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use strop_core::theme::ThemeId;
use strop_core::Buffer;

use super::golden::assert_golden;
use crate::editor::Editor;

const SIZES: [(u16, u16); 2] = [(140, 40), (80, 24)];
const THEMES: [ThemeId; 2] = [ThemeId::Hone, ThemeId::Vellum];

const SOURCE: &[&str] = &[
    "//! The modeline: one quiet row at the bottom edge.",
    "//!",
    "//! Every width is a display cell, never a character.",
    "",
    "use std::path::Path;",
    "",
    "/// Gather the row.",
    "pub struct Modeline {",
    "    chip: String,",
    "    dirty: bool,",
    "    line: usize,",
    "}",
    "",
    "impl Modeline {",
    "    fn gather(path: &Path, line: usize) -> Self {",
    "        let chip = format!(\" {} \", \"NORMAL\");",
    "        // the filename must survive truncation",
    "        Self { chip, dirty: path.exists(), line }",
    "    }",
    "}",
];

fn workspace() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src/render")).unwrap();
    std::fs::create_dir_all(root.path().join("docs")).unwrap();
    let mut source = SOURCE.join("\n");
    source.push('\n');
    std::fs::write(root.path().join("src/render/statusline.rs"), source).unwrap();
    std::fs::write(
        root.path().join("src/render/mod.rs"),
        "//! Rendering root.\npub mod statusline;\n\npub fn render() {\n    statusline::render();\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("docs/remote.md"),
        "# Remote\n\nThe modeline identifies the endpoint.\n\nThe modeline names the fixed root.\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("README.md"),
        "# strop\n\nsee the cut before you make it.\n",
    )
    .unwrap();
    root
}

fn editor(root: &tempfile::TempDir, theme: ThemeId) -> Editor {
    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    editor.set_theme(theme);
    editor
        .open_fixture(&root.path().join("src/render/statusline.rs"))
        .unwrap();
    editor.wait_analysis();
    editor
}

/// How a scene lets admitted work (syntax analysis, previews) settle
/// between the first paint and the recorded one.
type Settle = Box<dyn FnMut(&mut Editor)>;

fn settle_analysis() -> Settle {
    Box::new(|editor| editor.wait_analysis())
}

/// Pickers: the preview read and its analysis both settle before the
/// recorded paint, so no golden pins a `loading…` placeholder.
fn settle_picker() -> Settle {
    Box::new(|editor| {
        editor.wait_previews();
        editor.wait_analysis();
    })
}

fn frame(
    editor: &mut Editor,
    settle: &mut Settle,
    (columns, rows): (u16, u16),
) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(columns, rows)).unwrap();
    terminal
        .draw(|frame| crate::render::paint(editor, frame))
        .unwrap();
    settle(editor);
    terminal
        .draw(|frame| crate::render::paint(editor, frame))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn each(name: &str, scene: impl Fn(&mut Editor) -> Settle) {
    for theme in THEMES {
        for size in SIZES {
            let root = workspace();
            let mut editor = editor(&root, theme);
            let mut settle = scene(&mut editor);
            let grid = frame(&mut editor, &mut settle, size);
            assert_golden(
                &format!("{name}-{}-{}x{}", theme.name(), size.0, size.1),
                &grid,
            );
        }
    }
}

#[test]
fn editor_and_statusline() {
    each("editor", |editor| {
        editor.feed_text("9j");
        settle_analysis()
    });
}

#[test]
fn echo_row() {
    each("echo", |editor| {
        editor.feed_text("9j");
        editor.set_message("unsaved changes — :e! to force");
        settle_analysis()
    });
}

/// B5: a refusal longer than the row wraps (at most three rows, then
/// clips) instead of losing its tail to `…` on the first row.
#[test]
fn echo_row_wraps() {
    each("echo-wrapped", |editor| {
        editor.feed_text("9j");
        editor.set_message(
            "terminal refused: this buffer belongs to ssh://build-host/srv/project and a remote \
             terminal needs an admitted worker on that host — run :remote worker to admit one, or \
             :terminal-local for a shell on this machine; the full text of every message stays in \
             :messages after the echo row moves on",
        );
        settle_analysis()
    });
}

/// 0090 §3: opening a file with an earlier session's draft asks first.
#[test]
fn recovery_offer() {
    each("recovery", |editor| {
        let path = editor.buf().path.clone().unwrap();
        editor.fixture_recovery_draft(&path, "//! a draft\n");
        editor.open_fixture(&path).unwrap();
        assert!(editor.recovery_offer().is_some());
        settle_analysis()
    });
}

#[test]
fn split() {
    each("split", |editor| {
        editor.feed_text("5j");
        editor.split(true, None);
        editor.feed_text("3j");
        settle_analysis()
    });
}

#[test]
fn files_picker() {
    each("files", |editor| {
        editor.open_picker(strop_picker::Kind::Files);
        editor.wait_picker();
        editor.paste_bracketed("mod");
        editor.wait_picker();
        settle_picker()
    });
}

#[test]
fn grep_picker() {
    each("grep", |editor| {
        editor.open_search(false);
        editor.paste_bracketed("modeline");
        editor.wait_picker();
        settle_picker()
    });
}

#[test]
fn help() {
    each("help", |editor| {
        editor.feed_text(" ?");
        settle_analysis()
    });
}

#[test]
fn which_key() {
    each("which-key", |editor| {
        editor.feed_text(" ");
        settle_analysis()
    });
}

#[test]
fn completion() {
    each("completion", |editor| {
        // completion needs the app event channel; analysis then settles
        // through it rather than the local fixture channel
        let (sender, receiver) = strop_engine::editor::events::channel();
        editor.connect_events(sender);
        editor.feed_text("Goi    let ch<c-n>");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while editor
            .completion_menu()
            .is_none_or(|menu| menu.selected().is_none())
        {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!remaining.is_zero(), "completion never became selectable");
            editor.handle_app_event(receiver.recv_timeout(remaining).unwrap());
        }
        Box::new(move |editor| {
            while editor.analysis_pending() {
                let event = receiver
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .expect("analysis settles through the app channel");
                editor.handle_app_event(event);
            }
        })
    });
}

/// A terminal session in a split below the file, as `Space t` opens it:
/// a fixed launch directory and a small styled transcript, no PTY.
fn terminal_below(editor: &mut Editor) {
    use strop_terminal::model::{Color, Phase, Style};
    let plain = Style::default();
    let green = Style {
        foreground: Color::Indexed(2),
        ..plain
    };
    let bold = Style {
        bold: true,
        ..plain
    };
    editor.split_document(false, editor.current());
    editor.terminal_fixture_at(
        strop_workspace::ResourceLocation::local("/work".into()),
        &[
            ("$ cargo test", bold),
            ("   Compiling demo v0.1.0", plain),
            ("running 3 tests", plain),
            ("test render::chip ... ok", green),
            ("test render::echo ... ok", green),
            ("test result: ok. 3 passed", green),
            ("$ ", plain),
        ],
        Phase::Running,
    );
}

fn ctrl_key(editor: &mut Editor, ch: char) {
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
}

/// TERMINAL mode (0090 §4): the child's grid in the lower pane, the
/// `TERMINAL` chip and the one escape chord as the hint.
#[test]
fn terminal_mode() {
    each("terminal", |editor| {
        terminal_below(editor);
        editor.feed_text("i");
        settle_analysis()
    });
}

/// NORMAL on the same buffer: a numbered, read-only live transcript with
/// the cursor at the top, plain `NORMAL` chip, no `[RO]`, no hint.
#[test]
fn terminal_normal_mode() {
    each("terminal-normal", |editor| {
        terminal_below(editor);
        editor.feed_text("i");
        ctrl_key(editor, '\\');
        ctrl_key(editor, 'n');
        editor.feed_text("gg");
        settle_analysis()
    });
}

/// 0094 §3.1: the collection after an edit through it. The edited excerpt
/// shows the new text, its card is marked modified, and the chip reads
/// NORMAL without `[RO]`: it is an editing surface.
#[test]
fn editable_collection() {
    each("collection", |editor| {
        editor.open_search(false);
        editor.paste_bracketed("modeline");
        editor.wait_picker();
        editor.feed(strop_core::frontend_input::Key::CtrlO);
        editor.wait_io().unwrap();
        editor.feed_text("/identifies<cr>x");
        settle_analysis()
    });
}

fn replace_review(editor: &mut Editor) {
    editor.open_search(true);
    editor.paste_bracketed("modeline");
    editor.wait_picker();
    editor.feed(strop_core::frontend_input::Key::Tab);
    editor.paste_bracketed("status row");
    editor.feed(strop_core::frontend_input::Key::Enter);
    editor.wait_io().unwrap();
}

/// 0094 §3.2: the review a project replace opens — the diff, and the one
/// key that applies it.
#[test]
fn review() {
    each("review", |editor| {
        replace_review(editor);
        settle_analysis()
    });
}

/// The same buffer after `Enter`: the diff stays under a heading that says
/// what happened and how to take it back.
#[test]
fn review_applied() {
    each("review-applied", |editor| {
        replace_review(editor);
        editor.feed(strop_core::frontend_input::Key::Enter);
        editor.wait_io().unwrap();
        settle_analysis()
    });
}

/// 0094 §3.4: `Space a` on a source line with a diagnostic lists the
/// language server's fixes and nothing about files.
#[test]
fn actions_on_a_source_buffer() {
    each("actions", |editor| {
        editor.feed_text("8j");
        editor.fixture_insert_diagnostics(
            editor.current(),
            crate::editor::DocumentDiagnostics {
                revision: editor.buf().revision(),
                items: vec![strop_lsp::ResolvedDiag {
                    line: strop_core::id::LineIndex::new(8),
                    col: strop_core::id::ByteColumn::new(4),
                    severity: strop_lsp::Severity::Warning,
                    end_line: strop_core::id::LineIndex::new(8),
                    end_col: strop_core::id::ByteColumn::new(8),
                    message: "field `chip` is never read".into(),
                }],
            },
        );
        editor.fixture_code_actions(&[
            "Remove unused field `chip`",
            "Prefix with an underscore: `_chip`",
            "Allow `dead_code` on this struct",
        ]);
        editor.wait_picker();
        settle_picker()
    });
}

/// 0091 B2: the `:s` card with a change far to the right of a long line.
/// Both rows are cut to the same window, on the change.
#[test]
fn substitute_card() {
    each("substitute", |editor| {
        editor.fixture_buf_mut().insert(
            0,
            "const LABELS: &[&str] = &[\"alpha\", \"beta\", \"gamma\", \"delta\", \"epsilon\", \
             \"zeta\", \"eta\", \"theta\", \"iota\", \"kappa\", \"lambda\", \"mu\", \"nu\", \
             \"xi\", \"omicron\", \"modeline\", \"rho\"];\n",
        );
        editor.feed_text(":%s/modeline/statusline/g");
        settle_analysis()
    });
}
