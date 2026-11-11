//! The one register file (plan 0092 §1): every yank, delete and change
//! writes here, every `p`, `Ctrl-R` and field paste reads here. The
//! rules are Neovim's `op_yank`/`op_delete`/`shift_delete_registers`
//! (`:help registers`), stated once so no caller re-derives them.

use super::{Register, RegisterShape};

/// What produced the text — the rules differ (`"0` is a yank's, the
/// numbered shift and `"-` are a delete's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    Yank,
    /// A delete or change. `reg_one` is vim's `use_reg_one`: the motion
    /// was one of `% ( ) ` / ? n N { }`, which always fills `"1`.
    Delete {
        reg_one: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    #[error("register \"{0} is read-only")]
    ReadOnly(char),
    #[error("\"{0} is not a register")]
    Invalid(char),
}

static EMPTY: Register = Register {
    text: String::new(),
    shape: RegisterShape::Characterwise,
    file_provenance: None,
};

#[derive(Debug, Default)]
pub(crate) struct RegisterStore {
    /// `""`: what the last write left (vim's `y_previous`).
    unnamed: Option<Register>,
    /// `"0` the last unnamed yank, `"1`–`"9` the delete history.
    digits: [Option<Register>; 10],
    /// `"-`: the last delete within one line.
    small: Option<Register>,
    named: [Option<Register>; 26],
    /// `"+`/`"*`: what this editor last wrote to the system clipboard.
    /// Reads of the live clipboard are a provider job, never this cell.
    clipboard: Option<Register>,
    /// `":`: the last executed ex line. Read-only to operators; the ex
    /// prompt records it on accept.
    pub(crate) command_line: Option<String>,
}

/// The vim-valid register a write may name, already lowercased.
enum Cell {
    Digit(usize),
    Named(usize),
    Small,
    Clipboard,
}

fn cell(name: char) -> Result<Cell, RegisterError> {
    match name {
        '0'..='9' => Ok(Cell::Digit(name as usize - '0' as usize)),
        'a'..='z' => Ok(Cell::Named(name as usize - 'a' as usize)),
        'A'..='Z' => Ok(Cell::Named(name as usize - 'A' as usize)),
        '-' => Ok(Cell::Small),
        '+' | '*' => Ok(Cell::Clipboard),
        '.' | ':' | '/' | '%' | '#' => Err(RegisterError::ReadOnly(name)),
        other => Err(RegisterError::Invalid(other)),
    }
}

impl RegisterStore {
    fn slot(&mut self, cell: &Cell) -> &mut Option<Register> {
        match cell {
            Cell::Digit(index) => &mut self.digits[*index],
            Cell::Named(index) => &mut self.named[*index],
            Cell::Small => &mut self.small,
            Cell::Clipboard => &mut self.clipboard,
        }
    }

    /// A stored register. `None`/`"` is the unnamed register; `_` and
    /// never-written registers read empty.
    pub(crate) fn get(&self, name: Option<char>) -> Result<&Register, RegisterError> {
        let slot = match name {
            None | Some('"') => &self.unnamed,
            Some('_') => return Ok(&EMPTY),
            Some(name) => match cell(name)? {
                Cell::Digit(index) => &self.digits[index],
                Cell::Named(index) => &self.named[index],
                Cell::Small => &self.small,
                Cell::Clipboard => &self.clipboard,
            },
        };
        Ok(slot.as_ref().unwrap_or(&EMPTY))
    }

    /// Whether an operator may name this register (`"_` and the unnamed
    /// register included): asked before the edit, so a refused name
    /// never deletes text it cannot keep.
    pub(crate) fn writable(name: Option<char>) -> Result<(), RegisterError> {
        match name.filter(|name| !matches!(name, '"' | '_')) {
            Some(name) => cell(name).map(|_| ()),
            None => Ok(()),
        }
    }

    /// Direct content (a finished macro recording): the cell only —
    /// the unnamed register and the histories do not move. Names
    /// outside `a-z`/`0-9` are not recordable and are ignored.
    pub(crate) fn set(&mut self, name: char, register: Register) {
        if let Ok(target @ (Cell::Named(_) | Cell::Digit(_))) = cell(name) {
            *self.slot(&target) = Some(register);
        }
    }

    /// One yank/delete lands. Returns the text to publish to the system
    /// clipboard: a write that named `+`/`*`, or a plain yank.
    pub(crate) fn write(
        &mut self,
        name: Option<char>,
        register: Register,
        origin: Origin,
    ) -> Result<Option<String>, RegisterError> {
        let name = name.filter(|name| *name != '"');
        if name == Some('_') {
            // the black hole: nothing stored, the unnamed register kept
            return Ok(None);
        }
        let target = name.map(cell).transpose()?;
        let append = name.is_some_and(|name| name.is_ascii_uppercase());
        let mut published = None;
        if let Some(target) = &target {
            let slot = self.slot(target);
            match slot {
                Some(existing) if append && !existing.is_empty() => existing.append(&register),
                _ => *slot = Some(register.clone()),
            }
            let written = slot.clone();
            if matches!(target, Cell::Clipboard) {
                published = written.as_ref().map(|written| written.text.clone());
            }
            self.unnamed = written;
        }
        match origin {
            Origin::Yank => {
                // A plain yank (no register, or "0 by name) is also a
                // copy: it reaches the system clipboard (0092 §1.4).
                // Named yanks, deletes and changes never do.
                if matches!(target, None | Some(Cell::Digit(0))) {
                    published = Some(register.text.clone());
                    self.clipboard = Some(register.clone());
                }
                if target.is_none() {
                    self.digits[0] = Some(register.clone());
                    self.unnamed = Some(register);
                }
            }
            Origin::Delete { reg_one } => {
                let within_line =
                    register.shape != RegisterShape::Linewise && !register.text.contains('\n');
                if !within_line || reg_one {
                    // "1 takes every delete of a line or more — even a
                    // named one — and 1..8 shift down; 9 is lost
                    self.digits[1..].rotate_right(1);
                    self.digits[1] = Some(register.clone());
                    if !append {
                        self.unnamed = Some(register.clone());
                    }
                }
                if target.is_none() && within_line {
                    self.small = Some(register.clone());
                    self.unnamed = Some(register);
                }
            }
        }
        Ok(published)
    }
}

impl Register {
    /// `"Ayw`: vim's append. Linewise wins over the other shapes;
    /// charwise onto charwise continues the last line, anything else
    /// starts a new one.
    fn append(&mut self, more: &Register) {
        let linewise =
            self.shape == RegisterShape::Linewise || more.shape == RegisterShape::Linewise;
        if linewise {
            self.shape = RegisterShape::Linewise;
        } else if let RegisterShape::Blockwise { width } = self.shape {
            // a block stays a block; its rows pad to the widest row of
            // either part when it is put
            let more_width = match more.shape {
                RegisterShape::Blockwise { width } => width.get(),
                _ => more
                    .text
                    .lines()
                    .map(|line| line.chars().count())
                    .max()
                    .unwrap_or(0),
            };
            self.shape = RegisterShape::Blockwise {
                width: strop_core::id::DisplayColumn::new(width.get().max(more_width)),
            };
        }
        if self.shape != RegisterShape::Characterwise && !self.text.ends_with('\n') {
            self.text.push('\n');
        }
        self.text.push_str(&more.text);
        if linewise && !self.text.ends_with('\n') {
            self.text.push('\n');
        }
        self.file_provenance = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appending_to_a_block_keeps_a_block_as_wide_as_its_widest_part() {
        let cols = strop_core::id::DisplayColumn::new;
        let mut block = Register::blockwise("ab\ncd".to_owned(), cols(2));
        block.append(&Register::blockwise("efgh\nijkl".to_owned(), cols(4)));
        assert_eq!(block.text, "ab\ncd\nefgh\nijkl");
        assert_eq!(block.shape, RegisterShape::Blockwise { width: cols(4) });

        let mut block = Register::blockwise("ab\ncd".to_owned(), cols(2));
        block.append(&Register::characterwise("xyz".to_owned()));
        assert_eq!(block.text, "ab\ncd\nxyz");
        assert_eq!(block.shape, RegisterShape::Blockwise { width: cols(3) });
    }

    fn text(store: &RegisterStore, name: char) -> String {
        store.get(Some(name)).unwrap().text.clone()
    }

    const DELETE: Origin = Origin::Delete { reg_one: false };

    #[test]
    fn unnamed_yank_fills_zero_and_named_yank_leaves_it() {
        let mut store = RegisterStore::default();
        store
            .write(None, Register::characterwise("one"), Origin::Yank)
            .unwrap();
        store
            .write(Some('a'), Register::characterwise("two"), Origin::Yank)
            .unwrap();
        assert_eq!(text(&store, '0'), "one");
        assert_eq!(text(&store, 'a'), "two");
        assert_eq!(text(&store, '"'), "two", "unnamed follows the last write");
        assert_eq!(store.get(None).unwrap().text, "two");
    }

    #[test]
    fn line_deletes_shift_the_numbered_history_and_nine_falls_off() {
        let mut store = RegisterStore::default();
        for line in 1..=10 {
            store
                .write(None, Register::linewise(format!("{line}\n")), DELETE)
                .unwrap();
        }
        assert_eq!(text(&store, '1'), "10\n");
        assert_eq!(text(&store, '9'), "2\n");
        assert_eq!(text(&store, '-'), "", "a line delete is not a small one");
        assert_eq!(text(&store, '0'), "", "a delete never touches the yank");
    }

    #[test]
    fn small_delete_goes_to_minus_unless_a_register_is_named() {
        let mut store = RegisterStore::default();
        store
            .write(None, Register::characterwise("w"), DELETE)
            .unwrap();
        assert_eq!(text(&store, '-'), "w");
        assert_eq!(text(&store, '1'), "");
        store
            .write(Some('a'), Register::characterwise("x"), DELETE)
            .unwrap();
        assert_eq!(text(&store, '-'), "w", "a named small delete skips \"-");
        assert_eq!(text(&store, '1'), "");
        assert_eq!(text(&store, '"'), "x");
    }

    #[test]
    fn named_line_delete_still_fills_one_and_reg_one_motions_fill_both() {
        let mut store = RegisterStore::default();
        store
            .write(Some('a'), Register::linewise("l\n"), DELETE)
            .unwrap();
        assert_eq!(text(&store, 'a'), "l\n");
        assert_eq!(text(&store, '1'), "l\n");
        store
            .write(
                None,
                Register::characterwise("para"),
                Origin::Delete { reg_one: true },
            )
            .unwrap();
        assert_eq!(text(&store, '1'), "para");
        assert_eq!(text(&store, '2'), "l\n");
        assert_eq!(text(&store, '-'), "para");
    }

    #[test]
    fn uppercase_appends_and_linewise_wins() {
        let mut store = RegisterStore::default();
        store
            .write(Some('a'), Register::characterwise("one"), Origin::Yank)
            .unwrap();
        store
            .write(Some('A'), Register::characterwise(" two"), Origin::Yank)
            .unwrap();
        assert_eq!(text(&store, 'a'), "one two");
        assert_eq!(text(&store, '"'), "one two");
        store
            .write(Some('A'), Register::linewise("three\n"), Origin::Yank)
            .unwrap();
        let appended = store.get(Some('a')).unwrap();
        assert_eq!(appended.text, "one two\nthree\n");
        assert_eq!(appended.shape, RegisterShape::Linewise);
        store
            .write(Some('B'), Register::characterwise("fresh"), Origin::Yank)
            .unwrap();
        assert_eq!(text(&store, 'b'), "fresh", "appending to nothing sets");
    }

    #[test]
    fn black_hole_read_only_and_invalid_names() {
        let mut store = RegisterStore::default();
        store
            .write(None, Register::characterwise("keep"), Origin::Yank)
            .unwrap();
        assert_eq!(
            store.write(Some('_'), Register::linewise("gone\n"), DELETE),
            Ok(None)
        );
        assert_eq!(text(&store, '"'), "keep");
        assert_eq!(text(&store, '1'), "");
        assert_eq!(text(&store, '_'), "");
        for name in ['.', ':', '/', '%', '#'] {
            assert_eq!(
                store.write(Some(name), Register::characterwise("x"), Origin::Yank),
                Err(RegisterError::ReadOnly(name))
            );
        }
        assert_eq!(
            store.write(Some('!'), Register::characterwise("x"), Origin::Yank),
            Err(RegisterError::Invalid('!'))
        );
        assert_eq!(text(&store, '"'), "keep");
    }

    #[test]
    fn a_plain_yank_is_published_and_deletes_and_named_yanks_are_not() {
        let mut store = RegisterStore::default();
        assert_eq!(
            store.write(None, Register::characterwise("word"), Origin::Yank),
            Ok(Some("word".into()))
        );
        assert_eq!(text(&store, '+'), "word");
        assert_eq!(
            store.write(Some('0'), Register::characterwise("zero"), Origin::Yank),
            Ok(Some("zero".into()))
        );
        assert_eq!(
            store.write(Some('a'), Register::characterwise("named"), Origin::Yank),
            Ok(None)
        );
        assert_eq!(
            store.write(None, Register::linewise("line\n"), DELETE),
            Ok(None)
        );
        assert_eq!(
            store.write(None, Register::characterwise("x"), DELETE),
            Ok(None)
        );
        assert_eq!(
            text(&store, '+'),
            "zero",
            "the clipboard keeps the last yank"
        );
    }

    #[test]
    fn plus_and_star_are_one_clipboard_cell_and_publish() {
        let mut store = RegisterStore::default();
        assert_eq!(
            store.write(Some('+'), Register::characterwise("clip"), Origin::Yank),
            Ok(Some("clip".into()))
        );
        assert_eq!(text(&store, '*'), "clip");
        assert_eq!(text(&store, '"'), "clip");
        assert_eq!(text(&store, '0'), "");
    }
}
