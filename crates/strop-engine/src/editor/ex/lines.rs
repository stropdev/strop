//! The line commands: `:d :y :m :t :j :> :< :pu :k :delm :!`. Each works
//! on whole lines; line numbers in here are zero-based and inclusive.

use strop_core::Range;

use super::ExCall;
use crate::editor::ex::*;
use crate::editor::{Editor, Register};

/// `[x] [count]` after `:d` and `:y`: a register name, then a count.
fn register_argument(arg: &str) -> (Option<char>, &str) {
    match arg.chars().next() {
        Some(name) if !name.is_ascii_digit() => (Some(name), arg[name.len_utf8()..].trim_start()),
        _ => (None, arg),
    }
}

/// The bytes of lines `first..=last` with the line break that ends
/// them (the buffer's end, for a last line without one).
fn line_span(editor: &Editor, first: usize, last: usize) -> (usize, usize) {
    let start = editor.buf().line_start(first);
    let end = if last + 1 < editor.buf().len_lines() {
        editor.buf().line_start(last + 1)
    } else {
        editor.buf().len_bytes()
    };
    (start, end)
}

/// Lines `first..=last` as register text: every line ends in a break.
fn lines_text(editor: &Editor, first: usize, last: usize) -> String {
    let (start, end) = line_span(editor, first, last);
    let mut text = editor.buf().text().byte_slice(start..end).to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// Remove lines `first..=last`. A last line without a line break
/// takes the break before it, so no empty line is left behind.
fn remove_lines(editor: &mut Editor, first: usize, last: usize) {
    let (mut start, end) = line_span(editor, first, last);
    let unterminated = end > start && editor.buf().byte_at(end - 1) != Some(b'\n');
    if unterminated && start > 0 {
        start -= 1;
    }
    editor.filename_delete_hint(Range::charwise(start, end), true);
    editor.buf_mut().delete(Range::charwise(start, end));
    editor.clear_filename_hint();
}

/// Insert `text` (whole lines, each ending in a break) so its first
/// line becomes line `index`.
fn insert_lines(editor: &mut Editor, index: usize, text: &str) {
    let buf = editor.buf();
    let len = buf.len_bytes();
    if index <= buf.last_content_line() && len > 0 {
        let at = buf.line_start(index);
        editor.buf_mut().insert(at, text);
    } else if len == 0 || buf.byte_at(len - 1) == Some(b'\n') {
        editor.buf_mut().insert(len, text);
    } else {
        // Below a last line that has no line break: the break goes
        // in front, and the buffer still ends without one.
        let text = format!("\n{}", text.strip_suffix('\n').unwrap_or(text));
        editor.buf_mut().insert(len, &text);
    }
}

/// `:[range]d [x] [count]`
pub(crate) fn ex_delete(editor: &mut Editor, call: &ExCall<'_>) {
    let (name, rest) = register_argument(call.arg);
    let ((first, last), rest) = match call.counted(editor, rest) {
        Ok(counted) => counted,
        Err(error) => {
            editor.message = error;
            return;
        }
    };
    if !rest.is_empty() {
        editor.message = format!("trailing characters: {rest}");
        return;
    }
    if !editor.register_writable(name) {
        return;
    }
    let (start, end) = line_span(editor, first, last);
    let mut register = Register::linewise(lines_text(editor, first, last));
    register.file_provenance =
        editor.capture_filename_register([(Range::charwise(start, end), true)], true, false);
    editor.delete_register(name, register, false);
    editor.tx_begin();
    remove_lines(editor, first, last);
    editor.tx_commit();
    ex_land(editor, first);
    let count = last - first + 1;
    if count > 2 {
        editor.message = format!("{count} fewer lines");
    }
}

/// `:[range]y [x] [count]`
pub(crate) fn ex_yank(editor: &mut Editor, call: &ExCall<'_>) {
    let (name, rest) = register_argument(call.arg);
    let ((first, last), rest) = match call.counted(editor, rest) {
        Ok(counted) => counted,
        Err(error) => {
            editor.message = error;
            return;
        }
    };
    if !rest.is_empty() {
        editor.message = format!("trailing characters: {rest}");
        return;
    }
    if !editor.register_writable(name) {
        return;
    }
    let (start, end) = line_span(editor, first, last);
    let mut register = Register::linewise(lines_text(editor, first, last));
    register.file_provenance =
        editor.capture_filename_register([(Range::charwise(start, end), true)], false, false);
    editor.yank_register(name, register);
    let count = last - first + 1;
    if count > 2 {
        editor.message = format!("{count} lines yanked");
    }
}

/// The `{address}` of `:m` and `:t`: the one-based line the text goes
/// below (0 is above the first line).
pub(crate) fn destination(editor: &Editor, arg: &str) -> Result<usize, String> {
    let (range, rest) = parse_ex_range(editor, editor.head(), arg)?;
    let range = range.ok_or("address required")?;
    if !rest.trim().is_empty() {
        return Err(format!("trailing characters: {}", rest.trim()));
    }
    if range.last > ex_line_count(editor) {
        return Err("invalid range".into());
    }
    Ok(range.last)
}

/// `:[range]m {address}`
pub(crate) fn ex_move(editor: &mut Editor, call: &ExCall<'_>) {
    let (first, last) = call.lines(editor);
    let destination = match destination(editor, call.arg) {
        Ok(destination) => destination,
        Err(error) => {
            editor.message = error;
            return;
        }
    };
    let count = last - first + 1;
    let column = ex_column(editor);
    // `destination` is one-based: lines `first + 1 ..= last + 1`.
    if destination > first && destination <= last {
        editor.message = "cannot move a range of lines into itself".into();
        return;
    }
    let landing = if destination <= first {
        destination + count - 1
    } else {
        destination - 1
    };
    if destination != first && destination != last + 1 {
        let text = lines_text(editor, first, last);
        editor.tx_begin();
        if destination > last {
            insert_lines(editor, destination, &text);
            remove_lines(editor, first, last);
        } else {
            insert_lines(editor, destination, &text);
            remove_lines(editor, first + count, last + count);
        }
        editor.tx_commit();
    }
    ex_land_in_column(editor, landing, column);
    if count > 2 {
        editor.message = format!("{count} lines moved");
    }
}

/// `:[range]t {address}`, `:[range]co[py] {address}`
pub(crate) fn ex_copy(editor: &mut Editor, call: &ExCall<'_>) {
    let (first, last) = call.lines(editor);
    let destination = match destination(editor, call.arg) {
        Ok(destination) => destination,
        Err(error) => {
            editor.message = error;
            return;
        }
    };
    let count = last - first + 1;
    let text = lines_text(editor, first, last);
    let column = ex_column(editor);
    editor.tx_begin();
    insert_lines(editor, destination, &text);
    editor.tx_commit();
    ex_land_in_column(editor, destination + count - 1, column);
    if count > 2 {
        editor.message = format!("{count} more lines");
    }
}

/// `:[range]j[!] [count]`: Vim's join. Without `!` the next line's
/// leading blanks go and one space joins the two, unless the first
/// already ends in a blank, or the second is empty or starts with `)`.
pub(crate) fn ex_join(editor: &mut Editor, call: &ExCall<'_>) {
    let typed = call.range.is_some_and(|range| range.first != range.last);
    let ((mut first, mut last), rest) = match call.counted(editor, call.arg) {
        Ok(counted) => counted,
        Err(error) => {
            editor.message = error;
            return;
        }
    };
    if !rest.is_empty() {
        editor.message = format!("trailing characters: {rest}");
        return;
    }
    if rest.len() != call.arg.len() {
        // `:j N` joins N lines starting at the range's last line.
        first = call.lines(editor).1;
    } else if !typed {
        last = first + 1;
    }
    last = last.min(editor.buf().last_content_line());
    if first >= last {
        return;
    }
    let mut joined = editor.buf().line_text(first);
    for line in first + 1..=last {
        let text = editor.buf().line_text(line);
        if call.bang {
            joined.push_str(&text);
            continue;
        }
        let text = text.trim_start_matches([' ', '\t']);
        let blank_before = joined.ends_with([' ', '\t']);
        if !text.is_empty() && !text.starts_with(')') && !joined.is_empty() && !blank_before {
            joined.push(' ');
        }
        joined.push_str(text);
    }
    let start = editor.buf().line_start(first);
    let end = editor.buf().line_end(last);
    editor.tx_begin();
    editor.buf_mut().delete(Range::charwise(start, end));
    editor.buf_mut().insert(start, &joined);
    editor.tx_commit();
    ex_land(editor, first);
}

/// `:[range]> [count]`, `:[range]< [count]`; each extra `>` is one
/// more shift. Empty lines stay empty.
pub(crate) fn ex_shift(editor: &mut Editor, call: &ExCall<'_>) {
    let right = call.name == ">";
    let same = if right { '>' } else { '<' };
    let extra = call.arg.len() - call.arg.trim_start_matches(same).len();
    let ((first, last), rest) = match call.counted(editor, call.arg[extra..].trim_start()) {
        Ok(counted) => counted,
        Err(error) => {
            editor.message = error;
            return;
        }
    };
    if !rest.is_empty() {
        editor.message = format!("trailing characters: {rest}");
        return;
    }
    let indent = editor.cur_indent();
    let width = indent.width.max(1);
    let amount = (extra + 1) * width;
    editor.tx_begin();
    for line in first..=last {
        let text = editor.buf().line_text(line);
        let body = text.trim_start_matches([' ', '\t']);
        if body.is_empty() && text.is_empty() {
            continue;
        }
        let blanks = &text[..text.len() - body.len()];
        let columns = blanks.chars().fold(0, |column, c| match c {
            '\t' => (column / width + 1) * width,
            _ => column + 1,
        });
        let columns = if right {
            columns + amount
        } else {
            columns.saturating_sub(amount)
        };
        let replacement = match indent.style {
            crate::config::IndentStyle::Spaces => " ".repeat(columns),
            crate::config::IndentStyle::Tabs => {
                "\t".repeat(columns / width) + &" ".repeat(columns % width)
            }
        };
        if replacement != blanks {
            let start = editor.buf().line_start(line);
            editor
                .buf_mut()
                .delete(Range::charwise(start, start + blanks.len()));
            editor.buf_mut().insert(start, &replacement);
        }
    }
    editor.tx_commit();
    ex_land(editor, last);
    let count = last - first + 1;
    if count > 2 {
        editor.message = format!(
            "{count} lines {same}ed {} time{}",
            extra + 1,
            if extra == 0 { "" } else { "s" }
        );
    }
}

/// `:[line]pu[!] [x]`: the register as whole lines below the line
/// (above with `!`; `:0pu` is above the first).
pub(crate) fn ex_put(editor: &mut Editor, call: &ExCall<'_>) {
    let name = call.arg.chars().next();
    let rest = name.map_or("", |name| call.arg[name.len_utf8()..].trim());
    if !rest.is_empty() {
        editor.message = format!("trailing characters: {rest}");
        return;
    }
    let mut text = editor.register(name).text;
    if text.is_empty() {
        editor.message = format!("nothing in register {}", name.unwrap_or('"'));
        return;
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let cursor = editor.buf().line_of(editor.head()) + 1;
    let line = call.range.map_or(cursor, |range| range.last);
    let index = if call.bang {
        line.saturating_sub(1)
    } else {
        line
    };
    let count = text.matches('\n').count();
    editor.tx_begin();
    insert_lines(editor, index, &text);
    editor.tx_commit();
    ex_land(editor, index + count - 1);
    if count > 2 {
        editor.message = format!("{count} more lines");
    }
}

/// `:[line]k a`, `:[line]mark a`
pub(crate) fn ex_mark(editor: &mut Editor, call: &ExCall<'_>) {
    let mut names = call.arg.chars();
    let (Some(name), None) = (names.next(), names.next()) else {
        editor.message = if call.arg.is_empty() {
            "argument required".into()
        } else {
            format!("trailing characters: {}", call.arg)
        };
        return;
    };
    if !name.is_ascii_alphabetic() && !matches!(name, '\'' | '`') {
        editor.message = format!("{name} is not a mark name");
        return;
    }
    let line = call.lines(editor).1;
    let offset = editor.buf().line_start(line);
    editor.marks.insert(name, (editor.current(), offset));
}

/// `:delm a b`, `:delm a-d`, `:delm!` (every lowercase mark here).
pub(crate) fn ex_delmarks(editor: &mut Editor, call: &ExCall<'_>) {
    let document = editor.current();
    if call.bang {
        if !call.arg.is_empty() {
            editor.message = "invalid argument".into();
            return;
        }
        editor
            .marks
            .retain(|name, (owner, _)| !(name.is_ascii_lowercase() && *owner == document));
        return;
    }
    if call.arg.is_empty() {
        editor.message = "argument required".into();
        return;
    }
    let names: Vec<char> = call.arg.chars().filter(|c| *c != ' ').collect();
    let mut doomed = Vec::new();
    let mut index = 0;
    while index < names.len() {
        let name = names[index];
        if names.get(index + 1) == Some(&'-') {
            let Some(&end) = names.get(index + 2).filter(|end| **end >= name) else {
                editor.message = format!("invalid argument: {}", call.arg);
                return;
            };
            doomed.extend(name..=end);
            index += 3;
        } else {
            doomed.push(name);
            index += 1;
        }
    }
    // A lowercase mark belongs to its buffer; an uppercase one is global.
    editor.marks.retain(|name, (owner, _)| {
        !(doomed.contains(name) && (name.is_ascii_uppercase() || *owner == document))
    });
}

/// `:!cmd` runs a command; `:{range}!cmd` filters the lines through it.
pub(crate) fn ex_shell(editor: &mut Editor, call: &ExCall<'_>) {
    let Some(_) = call.range else {
        editor.shell_run(call.arg);
        return;
    };
    if editor.buf().readonly {
        editor.refuse_readonly();
        return;
    }
    let (first, last) = call.lines(editor);
    let (start, end) = line_span(editor, first, last);
    editor.pipe_run(start, end, call.arg);
}
