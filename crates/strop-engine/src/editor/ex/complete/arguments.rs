//! Argument rows by kind: open buffers, a command's fixed words, theme
//! names, help topics. Paths are `paths` (a worker job).

use strop_core::theme::ThemeId;

use super::super::table::{Arg, COMMANDS};
use super::{rank, MenuKind, Row, Text};
use crate::editor::ex::{buffer_listed, buffer_number};
use crate::editor::Editor;

/// Rows for a non-path argument at `[start, end)` of the body.
pub(super) fn rows(editor: &mut Editor, arg: Arg, start: usize, end: usize) {
    let buffers = (arg == Arg::Buffer).then(|| buffer_candidates(editor));
    let (line, completion) = editor.cmdline.parts_mut();
    let Some(line) = line else { return };
    let body = line.body();
    let word = &body[start..end];
    let (kind, title) = match arg {
        Arg::Buffer => {
            buffer_rows(word, buffers.unwrap_or_default(), &mut completion.rows);
            (MenuKind::Buffers, "buffers")
        }
        Arg::Theme => {
            let themes = ThemeId::ALL
                .iter()
                .map(|theme| (theme.name(), theme.description()));
            rank::fixed(word, themes, &mut completion.rows, &mut completion.scratch);
            (MenuKind::Themes, "themes")
        }
        Arg::HelpTopic => {
            let sections = crate::keymap::SECTIONS
                .iter()
                .map(|section| (section.id, section.title));
            let commands = COMMANDS.iter().map(|command| (command.name(), command.doc));
            rank::fixed(
                word,
                sections.chain(commands),
                &mut completion.rows,
                &mut completion.scratch,
            );
            (MenuKind::Topics, "help")
        }
        Arg::Words(words) => {
            rank::fixed(
                word,
                words.iter().copied(),
                &mut completion.rows,
                &mut completion.scratch,
            );
            (MenuKind::Words, "words")
        }
        _ => {
            completion.close();
            return;
        }
    };
    completion.show(kind, Text::Static(title), body, start, end);
}

/// Every listed buffer: number, name, unsaved.
fn buffer_candidates(editor: &Editor) -> Vec<(usize, String, bool)> {
    editor
        .docs
        .iter()
        .filter(|(id, _)| buffer_listed(editor, *id))
        .map(|(id, document)| {
            (
                buffer_number(id),
                document.label(&editor.cwd),
                document.buf.is_dirty(),
            )
        })
        .collect()
}

/// Buffers whose number starts with the typed digits, or whose name
/// holds the typed text (Vim's `:b` rule), then fuzzy ones.
fn buffer_rows(word: &str, buffers: Vec<(usize, String, bool)>, rows: &mut Vec<Row>) {
    rows.clear();
    let mut indices = Vec::new();
    let mut fuzzy = Vec::new();
    for (number, name, dirty) in buffers {
        let digits = number.to_string();
        let matched = if !word.is_empty() && digits.starts_with(word) {
            Some(0)
        } else if let Some(at) = name.find(word) {
            let skip = name[..at].chars().count();
            Some(rank::prefix_mask(skip + word.chars().count()) & !rank::prefix_mask(skip))
        } else {
            None
        };
        let row = |matched| Row {
            text: Text::Owned(name.as_str().into()),
            label: None,
            abbrev: Text::Owned(digits.as_str().into()),
            doc: Text::Static(""),
            key: None,
            matched,
            mark: dirty,
        };
        match matched {
            Some(matched) => rows.push(row(matched)),
            None => {
                if let Some((score, matched)) = rank::fuzzy(word, &name, &mut indices) {
                    fuzzy.push((score, row(matched)));
                }
            }
        }
    }
    fuzzy.sort_by_key(|row| std::cmp::Reverse(row.0));
    rows.extend(fuzzy.into_iter().map(|(_, row)| row));
}
