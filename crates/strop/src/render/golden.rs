//! Golden cell-grid snapshots (0088 §5): the regression net for the
//! look. A golden file stores the character grid followed by a per-row
//! style-run summary (`len fg bg modifiers`), so a colour or weight
//! regression fails the same as a glyph regression. No snapshot crate:
//! `STROP_UPDATE_GOLDEN=1 cargo test` rewrites the files under
//! `crates/strop/src/render/golden/`; a plain run compares and prints the
//! first differing line.

use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::style::Color;

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/render/golden")
        .join(format!("{name}.golden"))
}

fn color_label(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset => "reset".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// The grid as text: rows of symbols, a blank line, then one summary
/// line per row listing style runs as `len:fg/bg[+bold][+italic]…`.
pub(crate) fn render_text(grid: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..grid.area.height {
        for symbol in crate::headless::row_symbols(grid, y) {
            out.push_str(symbol);
        }
        out.push('\n');
    }
    out.push('\n');
    for y in 0..grid.area.height {
        out.push_str(&format!("{y:>3}:"));
        let mut x = grid.area.x;
        while x < grid.area.right() {
            let cell = &grid[(x, y)];
            let mut end = x + 1;
            while end < grid.area.right() {
                let next = &grid[(end, y)];
                if next.fg != cell.fg || next.bg != cell.bg || next.modifier != cell.modifier {
                    break;
                }
                end += 1;
            }
            let mut mods = String::new();
            for (flag, name) in [
                (ratatui::style::Modifier::BOLD, "b"),
                (ratatui::style::Modifier::ITALIC, "i"),
                (ratatui::style::Modifier::UNDERLINED, "u"),
                (ratatui::style::Modifier::CROSSED_OUT, "s"),
                (ratatui::style::Modifier::DIM, "d"),
            ] {
                if cell.modifier.contains(flag) {
                    mods.push('+');
                    mods.push_str(name);
                }
            }
            out.push_str(&format!(
                " {}:{}/{}{mods}",
                end - x,
                color_label(cell.fg),
                color_label(cell.bg)
            ));
            x = end;
        }
        out.push('\n');
    }
    out
}

/// Compare `grid` against the stored golden `name`; with
/// `STROP_UPDATE_GOLDEN=1` the file is (re)written instead.
pub(crate) fn assert_golden(name: &str, grid: &Buffer) {
    let actual = render_text(grid);
    let path = golden_path(name);
    if std::env::var_os("STROP_UPDATE_GOLDEN").is_some() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("golden directory");
        }
        std::fs::write(&path, &actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing golden {}: {error}\nrun with STROP_UPDATE_GOLDEN=1 to record it",
            path.display()
        )
    });
    if expected != actual {
        let mismatch = expected
            .lines()
            .zip(actual.lines())
            .enumerate()
            .find(|(_, (want, got))| want != got)
            .map(|(index, (want, got))| {
                format!("line {}:\n  want: {want}\n  got:  {got}", index + 1)
            })
            .unwrap_or_else(|| "line counts differ".to_owned());
        panic!(
            "golden {name} differs ({})\n{mismatch}\n\nfull frame:\n{}\nrun with STROP_UPDATE_GOLDEN=1 to accept",
            path.display(),
            actual.split("\n\n").next().unwrap_or_default()
        );
    }
}
