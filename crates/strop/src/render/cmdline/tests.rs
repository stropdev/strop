//! The command line's row and menu: golden frames in `hone` and `vellum`
//! at 80×24, 120×40 and 48×12, and the layout rules as assertions.

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use strop_core::theme::ThemeId;
use strop_core::Buffer;

use crate::editor::Editor;
use crate::render::golden::assert_golden;

const SIZES: [(u16, u16); 3] = [(80, 24), (120, 40), (48, 12)];
const THEMES: [ThemeId; 2] = [ThemeId::Hone, ThemeId::Vellum];

const SOURCE: &str = "\
//! The modeline: one quiet row at the bottom edge.

/// Gather the row.
pub struct Modeline {
    chip: String,
    dirty: bool,
    line: usize,
}

impl Modeline {
    fn gather(path: &Path, line: usize) -> Self {
        let chip = format!(\" {} \", \"NORMAL\");
        Self { chip, dirty: path.exists(), line }
    }
}
";

/// A project with a few files and three open buffers (one unsaved).
fn workspace() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src/render")).unwrap();
    for name in ["src/replay.rs", "src/remote.rs", "src/main.rs"] {
        std::fs::write(root.path().join(name), "fn main() {}\n").unwrap();
    }
    std::fs::write(root.path().join("src/render/statusline.rs"), SOURCE).unwrap();
    root
}

fn editor(root: &tempfile::TempDir, theme: ThemeId) -> Editor {
    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    editor.set_theme(theme);
    for name in ["src/main.rs", "src/remote.rs", "src/render/statusline.rs"] {
        editor.feed_text(&format!(":e {name}<cr>"));
        editor.wait_io().unwrap();
    }
    editor.feed_text("Gx");
    editor.wait_analysis();
    editor
}

fn frame(editor: &mut Editor, (columns, rows): (u16, u16)) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(columns, rows)).unwrap();
    // the first paint admits the work a frame asks for (a search count);
    // the recorded one shows it landed
    terminal
        .draw(|frame| crate::render::paint(editor, frame))
        .unwrap();
    editor.wait_analysis();
    terminal
        .draw(|frame| crate::render::paint(editor, frame))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn each(name: &str, scene: impl Fn(&mut Editor)) {
    for theme in THEMES {
        for size in SIZES {
            let root = workspace();
            let mut editor = editor(&root, theme);
            scene(&mut editor);
            let grid = frame(&mut editor, size);
            assert_golden(
                &format!("cmdline-{name}-{}-{}x{}", theme.name(), size.0, size.1),
                &grid,
            );
        }
    }
}

fn row_text(grid: &ratatui::buffer::Buffer, y: u16) -> String {
    (0..grid.area.width)
        .map(|x| grid[(x, y)].symbol())
        .collect::<String>()
}

#[test]
fn idle() {
    each("idle", |editor| editor.feed_text(":"));
}

#[test]
fn menu() {
    each("menu", |editor| editor.feed_text(":w"));
}

#[test]
fn menu_selected() {
    each("menu-selected", |editor| editor.feed_text(":c<tab><tab>"));
}

#[test]
fn files() {
    each("files", |editor| {
        editor.feed_text(":e src/re<tab>");
        editor.wait_path_completion();
    });
}

#[test]
fn buffers() {
    each("buffers", |editor| editor.feed_text(":b r<c-d>"));
}

#[test]
fn search() {
    each("search", |editor| {
        editor.feed_text("/chip");
        editor.wait_analysis();
    });
}

#[test]
fn long_line() {
    each("long-line", |editor| {
        editor.feed_text(":s/a very long pattern that runs past the row/and a replacement too/g")
    });
}

/// `:s` in place: each changed line reads as it will, the new text in
/// `diff.add`; the right side counts the changes and those out of view.
#[test]
fn substitute() {
    each("substitute", |editor| {
        editor.feed_text("gg:%s/Modeline/Statusline/g");
        editor.wait_resolution();
    });
}

#[test]
fn substitute_invalid() {
    each("substitute-invalid", |editor| {
        editor.feed_text(":%s/mode\\(line/x/")
    });
}

/// A range from Visual: its lines wear the accent, the right side names them.
#[test]
fn range() {
    each("range", |editor| editor.feed_text("4GVjj:d"));
}

#[test]
fn range_error() {
    each("range-error", |editor| editor.feed_text(":'q,$d"));
}

/// A failed line: it closes, and the echo row above the modeline says
/// why in error ink.
#[test]
fn error() {
    each("error", |editor| editor.feed_text(":wrte<cr>"));
}

#[test]
fn the_line_is_the_bottom_row_and_owns_the_caret() {
    let root = workspace();
    let mut editor = editor(&root, ThemeId::Hone);
    editor.feed_text(":wri");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| crate::render::paint(&mut editor, frame))
        .unwrap();
    let grid = terminal.backend().buffer().clone();
    let row = row_text(&grid, 23);
    assert!(row.starts_with(" COMMAND    :wri"), "{row:?}");
    assert!(
        row.trim_end().ends_with(":w[!] [file]"),
        "usage on the right"
    );
    terminal.backend_mut().assert_cursor_position((16, 23));
    // the menu stands on the line, its names under the typed word
    let above = row_text(&grid, 22);
    let at = above.find('╰').map(|byte| above[..byte].chars().count());
    assert_eq!(at, Some(10), "{above:?}");
    let first = row_text(&grid, 21);
    let at = first
        .find("write")
        .map(|byte| first[..byte].chars().count());
    assert_eq!(at, Some(13), "{first:?}");
    // Esc: the modeline is back
    editor.feed_text("<esc>");
    terminal
        .draw(|frame| crate::render::paint(&mut editor, frame))
        .unwrap();
    assert!(row_text(terminal.backend().buffer(), 23).starts_with("  NORMAL"));
}

/// While a range is half typed it may run backwards (`:3,` from line
/// 1); the hint names the lines either way.
#[test]
fn a_backwards_range_names_its_lines() {
    let root = workspace();
    let mut editor = editor(&root, ThemeId::Hone);
    editor.feed_text("gg:3,");
    let grid = frame(&mut editor, (80, 24));
    assert!(row_text(&grid, 23).trim_end().ends_with("1–3 · 3 lines"));
}

#[test]
fn the_right_side_gives_way_before_the_line_is_cut() {
    let root = workspace();
    let mut editor = editor(&root, ThemeId::Hone);
    editor.feed_text(":w averyveryverylongfilename");
    let grid = frame(&mut editor, (48, 12));
    let row = row_text(&grid, 11);
    assert!(row.contains(":w averyveryverylongfilename"), "{row:?}");
    assert!(!row.contains("[file]"), "the usage dropped: {row:?}");
}

#[test]
fn an_invalid_substitute_says_so_in_error_ink() {
    let root = workspace();
    let mut editor = editor(&root, ThemeId::Hone);
    editor.feed_text(":%s/mode\\(line/x/");
    let grid = frame(&mut editor, (120, 40));
    let row = row_text(&grid, 39);
    let at = row.find("invalid").expect("the error shows");
    let x = row[..at].chars().count() as u16;
    let t = crate::render::style::Tokens::new(ThemeId::Hone);
    assert_eq!(grid[(x, 39)].fg, t.error);
}
