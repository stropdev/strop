//! `:ls`, `:reg`, `:marks`: Vim's listings, each in a temporary readonly
//! buffer (as `:messages` is), so it scrolls, searches and yanks.

use strop_core::id::DocumentId;

use crate::editor::registers::RegisterShape;
use crate::editor::{DocumentSource, Editor};

/// The order `:reg` lists registers in.
const REGISTER_ORDER: &str = "\"0123456789abcdefghijklmnopqrstuvwxyz-*+.:%/";

fn open_listing(editor: &mut Editor, name: &str, text: &str) {
    let mut buffer = strop_core::Buffer::from_text(text);
    buffer.name = Some(name.into());
    editor.open_temporary_output(buffer);
}

/// A buffer's number: stable while the buffer lives.
pub(crate) fn buffer_number(id: DocumentId) -> usize {
    id.index() + 1
}

/// Help, listings and git surfaces are Vim's unlisted buffers.
pub(crate) fn buffer_listed(editor: &Editor, id: DocumentId) -> bool {
    editor.docs.get(id).is_some_and(|document| {
        !matches!(
            document.source,
            DocumentSource::Output { .. } | DocumentSource::Surface(_)
        )
    })
}

/// `:ls` text: Vim's columns — number, `u` unlisted, `%` current or
/// `#` alternate, `a` shown in a window or `h` hidden, `=` readonly
/// (`R` / `F` a running / finished terminal), `+` modified — then
/// the quoted name and the cursor line.
pub(crate) fn buffer_listing(editor: &Editor, all: bool) -> String {
    let current = editor.current();
    let alternate = editor
        .mru
        .iter()
        .copied()
        .find(|id| *id != current && editor.docs.get(*id).is_some());
    let mut text = String::new();
    for (id, document) in editor.docs.iter() {
        let listed = buffer_listed(editor, id);
        if !listed && !all {
            continue;
        }
        let pane = editor
            .panes
            .iter()
            .enumerate()
            .filter(|(_, pane)| pane.doc == id)
            .max_by_key(|(index, _)| *index == editor.active_pane)
            .map(|(_, pane)| pane);
        let state = match editor.terminal_phase(id) {
            Some(phase) if phase.live() => 'R',
            Some(_) => 'F',
            None if document.buf.readonly => '=',
            None => ' ',
        };
        let row = format!(
            "{:>3}{}{}{}{}{} \"{}\"",
            buffer_number(id),
            if listed { ' ' } else { 'u' },
            if id == current {
                '%'
            } else if Some(id) == alternate {
                '#'
            } else {
                ' '
            },
            if pane.is_some() { 'a' } else { 'h' },
            state,
            if document.buf.is_dirty() { '+' } else { ' ' },
            document.label(&editor.cwd),
        );
        // A buffer no window shows has no cursor: Vim says line 0.
        let line = pane.map_or(0, |pane| document.buf.line_of(pane.sels.primary().head) + 1);
        let width = row.chars().count();
        text.push_str(&row);
        text.push_str(&" ".repeat(40usize.saturating_sub(width).max(1)));
        text.push_str(&format!("line {line}\n"));
    }
    text
}

pub(crate) fn open_buffer_listing(editor: &mut Editor, all: bool) {
    let text = buffer_listing(editor, all);
    open_listing(editor, "buffers", &text);
}

/// `:reg [names]` text: `Type Name Content`, one row per non-empty
/// register, control characters in Vim's `^J` notation.
pub(crate) fn register_listing(editor: &Editor, names: &str) -> String {
    let wanted: Vec<char> = names.chars().filter(|c| *c != ' ').collect();
    let mut text = String::from("Type Name Content\n");
    for name in REGISTER_ORDER.chars() {
        if !wanted.is_empty() && !wanted.contains(&name) {
            continue;
        }
        let register = editor.register(Some(name));
        if register.is_empty() {
            continue;
        }
        let shape = match register.shape {
            RegisterShape::Characterwise => 'c',
            RegisterShape::Linewise => 'l',
            RegisterShape::Blockwise { .. } => 'b',
        };
        let mut content = String::new();
        for c in register.text.chars().take(200) {
            match c {
                '\n' => content.push_str("^J"),
                '\t' => content.push_str("^I"),
                c if c.is_control() => {
                    content.push('^');
                    content.push((c as u8 ^ 0x40) as char);
                }
                c => content.push(c),
            }
        }
        text.push_str(&format!("  {shape}  \"{name}   {content}\n"));
    }
    text
}

pub(crate) fn open_registers_listing(editor: &mut Editor, names: &str) {
    let text = register_listing(editor, names);
    open_listing(editor, "registers", &text);
}

/// `:marks` text: `mark line  col file/text` — a mark in this buffer
/// shows its line's text, a mark elsewhere its file.
pub(crate) fn mark_listing(editor: &Editor) -> String {
    let mut text = String::from("mark line  col file/text\n");
    let mut names: Vec<char> = editor.marks.keys().copied().collect();
    names.sort_by_key(|name| (name.is_ascii_uppercase(), *name));
    let current = editor.current();
    let mut row = |name: char, document: DocumentId, offset: usize| {
        let Some(owner) = editor.docs.get(document) else {
            return;
        };
        let offset = offset.min(owner.buf.len_bytes());
        let line = owner.buf.line_of(offset);
        let what = if document == current {
            owner.buf.line_text(line).trim_start().to_owned()
        } else {
            owner.label(&editor.cwd)
        };
        text.push_str(&format!(
            " {name} {:>6} {:>4} {what}\n",
            line + 1,
            owner.buf.col_of(offset)
        ));
    };
    for name in names {
        let (document, offset) = editor.marks[&name];
        row(name, document, offset);
    }
    for name in ['<', '>'] {
        if let Some(offset) = editor.mark_offset(name) {
            row(name, current, offset);
        }
    }
    text
}

pub(crate) fn open_marks_listing(editor: &mut Editor) {
    let text = mark_listing(editor);
    open_listing(editor, "marks", &text);
}
