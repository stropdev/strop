//! The field input machine (plan 0003 §2: the input line is insert mode
//! in a prompt buffer). Picker query/replace fields and the `: / ? |`
//! line speak the REAL vim grammar in their normal mode: one Walker per
//! surface accumulates counts, operators and motion text; completed
//! commands resolve through `strop_grammar::resolve` — THE resolver
//! document editing consumes — against the field's own one-line buffer.
//!
//! Admission (one-line semantics): motions `h l 0 $ w b e ge gE W B E
//! f/F/t/T %` (the grammar's other motions — `gg G { } ^ j k` — resolve
//! too; they simply cannot leave the one line), counts, text objects
//! (`iw/aw/iW/aW`, quotes, brackets), operators `d c y` over all of
//! them, `x/X`, `i/a/A`, and the semantic aliases `D C Y s`. `c`
//! re-enters the field's own insert mode (the `LineEdit.normal` flag —
//! never `Editor.mode`). Linewise `dd/cc/yy` act on the whole field —
//! the field IS one line; on a sigil line (`: / ? |`) that range
//! includes the sigil, so the standing sigil rule closes the prompt.
//!
//! Exclusions refuse with a message — an unsupported key in a field is
//! surfaced, never swallowed: nested prompts (`: / ? |`, `d/search`),
//! registers (yank resolves — a dead target still refuses — but stores
//! nothing in v1), surround, indent (`>`/`<` is meaningless on one
//! line), replace-char `r`, marks, macros, dot-repeat, and every other
//! document-surface leaf the Walker can complete (`gb`, `p`, …).

use strop_grammar::{self as grammar, Command, Motion, Op, Parse, Target};
use strop_picker::LineEdit;

use super::input::{Action, Walker};
use super::Key;

/// One key's outcome in a field's normal mode.
pub(crate) enum FieldReply {
    /// Consumed: a sequence is pending, a motion moved the caret, or an
    /// operator applied. The caller diffs text revision / modal flag
    /// for its own notifications.
    Consumed,
    /// Not admitted in a field — the reason to surface (the refusal is
    /// the feedback; nothing was silently dropped).
    Refused(String),
}

/// A field-scoped Walker: the typed parser state (counts, pending
/// operator, motion text) between key events of one field's normal mode.
#[derive(Debug, Default)]
pub(crate) struct FieldMachine {
    walker: Walker,
}

impl FieldMachine {
    /// Ground the machine (surface opened/closed, field toggled, Esc).
    pub(crate) fn clear(&mut self) {
        self.walker.clear();
    }

    /// Nothing typed, nothing pending. The picker's j/k result walk
    /// consults this: mid-composition `j`/`k` are grammar (a motion or
    /// an operator's target), not list navigation.
    pub(crate) fn is_ground(&self) -> bool {
        self.walker.is_ground()
    }

    /// One key while the field is in normal mode.
    pub(crate) fn feed(&mut self, field: &mut LineEdit, key: Key) -> FieldReply {
        match self.walker.feed(key) {
            Action::Pending => FieldReply::Consumed,
            Action::Invalid(keys) => {
                FieldReply::Refused(format!("not an input-field command: {keys}"))
            }
            Action::QueryError(error) => FieldReply::Refused(error.to_string()),
            Action::EnterText { .. } => {
                FieldReply::Refused("an input field cannot open another prompt".into())
            }
            Action::VisualSurround(_) => unreachable!("fields never drive the visual walker"),
            Action::Row {
                row,
                count,
                register,
                arg: _,
                key,
            } => Self::row(field, row, count, register, key),
            Action::Grammar(command) => Self::apply(field, &command),
        }
    }

    /// A completed table row in a field: aliases re-parse into grammar
    /// (`D` is `d$`), x/X and the insert entries are field-local; every
    /// other leaf/absorber is a document-surface command and refuses.
    fn row(
        field: &mut LineEdit,
        row: &'static crate::keymap::Binding,
        count: Option<usize>,
        register: Option<char>,
        key: char,
    ) -> FieldReply {
        use crate::keymap::Handler;
        match row.handler {
            // aliases are semantic (0016): the expansion parses once
            // into a grammar command and the walker's count merges in
            Handler::Alias(expansion) => match grammar::parse(expansion) {
                Parse::Complete(mut command) => {
                    command.count = Some(
                        count
                            .unwrap_or(1)
                            .saturating_mul(command.count.unwrap_or(1)),
                    );
                    if register.is_some() {
                        command.register = register;
                    }
                    Self::apply(field, &command)
                }
                Parse::QueryError(error) => FieldReply::Refused(error.to_string()),
                Parse::Incomplete | Parse::Invalid => {
                    FieldReply::Refused(format!("not an input-field command: {key}"))
                }
            },
            Handler::Leaf(_) => match row.id {
                "char-delete" => {
                    Self::delete_chars(field, count.unwrap_or(1), key == 'X');
                    FieldReply::Consumed
                }
                "insert-entries" => match key {
                    'i' => {
                        field.normal = false;
                        FieldReply::Consumed
                    }
                    'a' => {
                        field.move_right();
                        field.normal = false;
                        FieldReply::Consumed
                    }
                    'A' => {
                        field.set_cursor(usize::MAX);
                        field.normal = false;
                        FieldReply::Consumed
                    }
                    _ => FieldReply::Refused(format!("'{key}' is not supported in an input field")),
                },
                _ => {
                    FieldReply::Refused(format!("{} is not supported in an input field", row.desc))
                }
            },
            _ => FieldReply::Refused(format!("{} is not supported in an input field", row.desc)),
        }
    }

    /// x/X with a count: chars under / before the caret.
    fn delete_chars(field: &mut LineEdit, count: usize, backward: bool) {
        for _ in 0..count {
            if backward {
                if field.cursor() == 0 {
                    break;
                }
                field.backspace();
            } else {
                let cursor = field.cursor();
                if cursor >= field.text().len() {
                    break;
                }
                let mut end = cursor + 1;
                while end < field.text().len() && !field.text().is_char_boundary(end) {
                    end += 1;
                }
                field.delete_range(cursor, end);
            }
        }
    }

    /// A completed grammar command against the field's one-line buffer —
    /// the same `resolve`/`cursor_after` document editing consumes.
    fn apply(field: &mut LineEdit, command: &Command) -> FieldReply {
        if command.register.is_some() {
            return FieldReply::Refused("registers are not supported in an input field".into());
        }
        match &command.target {
            Target::SurroundDelete(_)
            | Target::SurroundChange { .. }
            | Target::SurroundAdd { .. } => {
                return FieldReply::Refused("surround is not supported in an input field".into());
            }
            Target::Motion(Motion::Search(_) | Motion::SearchBackward(_)) => {
                return FieldReply::Refused(
                    "search motions are not supported in an input field".into(),
                );
            }
            _ => {}
        }
        if matches!(command.op, Some(Op::Indent | Op::Dedent)) {
            return FieldReply::Refused("indent is not supported in a one-line field".into());
        }
        let resolved = match grammar::resolve(field.buffer(), field.cursor(), command) {
            Ok(Some(resolved)) => resolved,
            // A motion that finds nothing fails the vim way: nothing
            // happens, nothing is said (`dw` at the buffer end, `f`
            // with no target ahead, `j` on the one line).
            Ok(None) => return FieldReply::Consumed,
            Err(error) => return FieldReply::Refused(error.to_string()),
        };
        match command.op {
            None => {
                let cursor =
                    grammar::cursor_after(field.buffer(), field.cursor(), command, &resolved);
                field.set_cursor(cursor);
            }
            // Yank resolves (a dead target still refuses) but stores
            // nothing: register writes are out of v1 — module docs.
            Some(Op::Yank) => {}
            Some(Op::Delete) => {
                let start = resolved.range.start.get();
                field.delete_range(start, resolved.range.end.get());
                field.set_cursor(start);
            }
            Some(Op::Change) => {
                let start = resolved.range.start.get();
                field.delete_range(start, resolved.range.end.get());
                field.set_cursor(start);
                field.normal = false;
            }
            Some(Op::Indent | Op::Dedent) => unreachable!("indent refuses above"),
        }
        FieldReply::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a key string through the machine on a fresh normal-mode
    /// field; return the field for assertions.
    fn drive(text: &str, cursor: usize, keys: &str) -> LineEdit {
        let mut field = LineEdit::new(text);
        field.normal = true;
        field.set_cursor(cursor);
        let mut machine = FieldMachine::default();
        for c in keys.chars() {
            match machine.feed(&mut field, Key::Char(c)) {
                FieldReply::Consumed => {}
                FieldReply::Refused(message) => panic!("{keys} refused: {message}"),
            }
        }
        field
    }

    #[test]
    fn word_operators_are_the_real_grammar() {
        let field = drive("main", 4, "db");
        assert_eq!(field.text(), "");
        let field = drive("main one", 0, "de");
        assert_eq!(field.text(), " one");
        let field = drive("ma:in", 0, "df:");
        assert_eq!(field.text(), "in");
        let field = drive("one two three", 0, "2dw");
        assert_eq!(field.text(), "three");
        assert_eq!(field.cursor(), 0);
        let field = drive("one two", 5, "diw");
        assert_eq!(field.text(), "one ");
        assert_eq!(field.cursor(), 4);
        let field = drive("one two", 5, "daw");
        assert_eq!(field.text(), "one");
    }

    #[test]
    fn change_enters_the_fields_own_insert_mode() {
        let field = drive("main", 0, "cw");
        assert_eq!(field.text(), "");
        assert!(!field.normal, "cw leaves the field in insert mode");
        let field = drive("main one", 0, "cf:");
        assert!(field.normal, "no ':' ahead: the change found nothing");
        let field = drive("main", 0, "cc");
        assert_eq!(field.text(), "", "cc clears the field's one line");
        assert!(!field.normal);
    }

    #[test]
    fn linewise_and_aliases_and_char_delete() {
        let field = drive("main", 2, "dd");
        assert_eq!(field.text(), "", "dd clears the field's one line");
        let field = drive("main", 0, "D");
        assert_eq!(field.text(), "", "D is d$ — the alias re-parses");
        let field = drive("main", 1, "x");
        assert_eq!(field.text(), "min");
        let field = drive("main", 2, "X");
        assert_eq!(field.text(), "min");
        let field = drive("main", 0, "3x");
        assert_eq!(field.text(), "n");
        let field = drive("main", 0, "s");
        assert_eq!(field.text(), "ain");
        assert!(!field.normal, "s is cl: change enters insert");
        let field = drive("main", 0, "yw");
        assert_eq!(field.text(), "main", "yank stores nothing in v1");
        assert_eq!(field.cursor(), 0);
    }

    #[test]
    fn exclusions_refuse_loudly() {
        let mut field = LineEdit::new("main");
        field.normal = true;
        let mut machine = FieldMachine::default();
        let refused = |machine: &mut FieldMachine, field: &mut LineEdit, keys: &str| {
            let mut message = None;
            for c in keys.chars() {
                if let FieldReply::Refused(m) = machine.feed(field, Key::Char(c)) {
                    message = Some(m);
                    break;
                }
            }
            message.unwrap_or_else(|| panic!("{keys} should refuse"))
        };
        assert!(refused(&mut machine, &mut field, "\"ayw").contains("registers"));
        assert!(refused(&mut machine, &mut field, "ds\"").contains("surround"));
        assert!(refused(&mut machine, &mut field, ">w").contains("indent"));
        assert!(refused(&mut machine, &mut field, ":").contains("prompt"));
        assert!(refused(&mut machine, &mut field, "d/").contains("prompt"));
        assert!(refused(&mut machine, &mut field, "o").contains("'o'"));
        assert!(refused(&mut machine, &mut field, "rx").contains("replace char"));
        assert!(refused(&mut machine, &mut field, "gb").contains("occurrence"));
        assert!(refused(&mut machine, &mut field, "dq").contains("not an input-field command"));
        assert_eq!(field.text(), "main", "refusals never edit");
    }

    #[test]
    fn pure_motions_never_bump_the_revision() {
        let mut field = LineEdit::new("one two");
        field.normal = true;
        let mut machine = FieldMachine::default();
        let clean = field.revision();
        for c in "0whb$ge".chars() {
            machine.feed(&mut field, Key::Char(c));
        }
        assert_eq!(field.revision(), clean);
        assert_eq!(field.text(), "one two");
        machine.feed(&mut field, Key::Char('x'));
        assert_eq!(
            field.revision().get(),
            clean.get() + 1,
            "one completed operator, one bump"
        );
    }

    /// The differential check: the field machine's result IS
    /// `strop_grammar::resolve` + `cursor_after` on the same text —
    /// compute both sides independently and compare.
    #[test]
    fn field_editing_matches_grammar_resolution() {
        let cases: &[(&str, usize, &str, Option<usize>)] = &[
            ("one two three", 0, "dw", None),
            ("one two three", 0, "dw", Some(2)),
            ("main one", 7, "db", None),
            ("ma:in", 0, "df:", None),
            ("one two", 5, "diw", None),
            ("call(a, b)", 6, "da(", None),
            ("one two", 0, "cw", None),
            ("main", 0, "dd", None),
            ("one (two) three", 4, "%", None),
        ];
        for &(text, cursor, keys, count) in cases {
            // side A: the machine (counts type as digit keys)
            let mut field = LineEdit::new(text);
            field.normal = true;
            field.set_cursor(cursor);
            let mut machine = FieldMachine::default();
            let typed = match count {
                Some(n) => format!("{n}{keys}"),
                None => keys.to_string(),
            };
            for c in typed.chars() {
                match machine.feed(&mut field, Key::Char(c)) {
                    FieldReply::Consumed => {}
                    FieldReply::Refused(message) => panic!("{typed} refused: {message}"),
                }
            }
            // side B: parse + resolve by hand
            let buf = strop_core::Buffer::from_text(text);
            let Parse::Complete(mut command) = grammar::parse(keys) else {
                panic!("{keys} must parse");
            };
            command.count = count;
            let resolved = grammar::resolve(&buf, cursor, &command)
                .expect("resolvable")
                .expect("finds a target");
            let (want_text, want_cursor) = match command.op {
                None => (
                    text.to_string(),
                    grammar::cursor_after(&buf, cursor, &command, &resolved),
                ),
                Some(Op::Delete) | Some(Op::Change) => {
                    let mut t = text.to_string();
                    t.drain(resolved.range.start.get()..resolved.range.end.get());
                    (t, resolved.range.start.get())
                }
                Some(Op::Yank) => (text.to_string(), cursor),
                Some(Op::Indent | Op::Dedent) => unreachable!("refused in fields"),
            };
            assert_eq!(field.text(), want_text, "{keys} on {text:?}@{cursor}");
            assert_eq!(field.cursor(), want_cursor, "{keys} on {text:?}@{cursor}");
            assert_eq!(
                field.normal,
                !matches!(command.op, Some(Op::Change)),
                "{keys} on {text:?}@{cursor}"
            );
        }
    }
}
