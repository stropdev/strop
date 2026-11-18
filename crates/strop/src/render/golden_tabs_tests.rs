//! Golden frames for the tab line (plans/0113 §5.2, §8.4), in `hone` and
//! `vellum`: `tabs-split` (a) and `tabs-two` (b) at the usual sizes,
//! `tabs-narrow` (c) at four widths, `tabs-many` (d) with twenty tab pages.
//! With one tab page nothing changes: every other golden is that check.

use super::golden::assert_golden;
use super::golden_tests::{editor, frame, settle_analysis, workspace, SIZES, THEMES};
use crate::editor::Editor;

/// Open `name` in a new tab page and let it land.
fn tab_new(editor: &mut Editor, name: &str) {
    editor.feed_text(&format!(":tabnew {name}<cr>"));
    editor.wait_io().unwrap();
}

/// (a): three tab pages, the first current and split in two, an unsaved
/// file in the first two.
fn three(editor: &mut Editor) {
    editor.feed_text(":vs src/render/mod.rs<cr>");
    editor.wait_io().unwrap();
    editor.feed_text("<c-w>h:1s/$/ /<cr>:tab split<cr>:tabnew<cr>1gt");
}

fn frames(name: &str, sizes: &[(u16, u16)], scene: impl Fn(&mut Editor)) {
    for theme in THEMES {
        for &size in sizes {
            let root = workspace();
            let mut editor = editor(&root, theme);
            scene(&mut editor);
            let mut settle = settle_analysis();
            let grid = frame(&mut editor, &mut settle, size);
            assert_golden(
                &format!("{name}-{}-{}x{}", theme.name(), size.0, size.1),
                &grid,
            );
        }
    }
}

#[test]
fn tabs_split() {
    frames("tabs-split", &SIZES, three);
}

/// (b): the same after `gt` — one pane, so no title rule.
#[test]
fn tabs_two() {
    frames("tabs-two", &SIZES, |editor| {
        three(editor);
        editor.feed_text("gt");
    });
}

/// (c): six tab pages, the third current; names give way, then the others'
/// names, at 120, 80, 64 and 48 columns.
#[test]
fn tabs_narrow() {
    let sizes = [(120, 12), (80, 12), (64, 12), (48, 12)];
    frames("tabs-narrow", &sizes, |editor| {
        for name in [
            "layout.rs",
            "golden_modeline_tests.rs",
            "Cargo.toml",
            "breadcrumb.md",
        ] {
            std::fs::write(editor.cwd().join(name), "x\n").unwrap();
        }
        editor.feed_text(":e layout.rs<cr>");
        editor.wait_io().unwrap();
        editor.feed_text(":vs<cr>:sp<cr>");
        tab_new(editor, "src/render/statusline.rs");
        editor.feed_text(":1s/$/ /<cr>");
        tab_new(editor, "golden_modeline_tests.rs");
        editor.feed_text(":vs<cr>");
        for name in ["README.md", "Cargo.toml", "breadcrumb.md"] {
            tab_new(editor, name);
        }
        editor.feed_text("3gt");
    });
}

/// (d): twenty tab pages at 48 columns, the first, the eleventh and the
/// last current: a run around the current label, `‹` `›` at the ends.
#[test]
fn tabs_many() {
    for current in [1, 11, 20] {
        frames(&format!("tabs-many-{current}"), &[(48, 12)], |editor| {
            for k in 1..=20 {
                std::fs::write(editor.cwd().join(format!("file_{k}.rs")), "x\n").unwrap();
            }
            editor.feed_text(":e file_1.rs<cr>");
            editor.wait_io().unwrap();
            for k in 2..=20 {
                tab_new(editor, &format!("file_{k}.rs"));
            }
            editor.feed_text("3gt:1s/$/ /<cr>");
            editor.feed_text(&format!("{current}gt"));
        });
    }
}
