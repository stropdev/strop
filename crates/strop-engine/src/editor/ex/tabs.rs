//! Tab pages on the command line (plans/0113 §3): `:tabnew :tab split
//! :tabn :tabp :tabfir :tabl :tabc :tabo :tabm`. Every one is a line on
//! `editor/tabs`, which owns which tab page is current.

use super::table::{self, optional};
use super::{ExCall, ExResult};
use crate::editor::io::OpenIntent;
use crate::editor::tabs;
use crate::editor::{Document, Editor, Pane};

/// No tab page changes while the command-line window is open (Vim's E11).
fn admitted(editor: &Editor) -> ExResult {
    tabs::refusal(editor).map_or(Ok(()), |refused| Err(refused.into()))
}

/// The number a command addresses: its argument, else its count
/// (`:3tabn`), else none.
fn number(call: &ExCall<'_>) -> Result<Option<usize>, String> {
    if call.arg.is_empty() {
        return Ok(call.range.map(|range| range.last));
    }
    call.arg
        .parse()
        .map(Some)
        .map_err(|_| format!("trailing characters: {}", call.arg))
}

/// Tab page `number` (1-based) as an index, if there is one.
fn page(editor: &Editor, number: usize) -> Result<usize, String> {
    if (1..=tabs::count(editor)).contains(&number) {
        Ok(number - 1)
    } else {
        Err(format!("no tab {number}"))
    }
}

/// `:tabn[ext] [N]`: the next tab page, or tab page N.
pub(crate) fn ex_tab_next(editor: &mut Editor, call: &ExCall<'_>) -> ExResult {
    admitted(editor)?;
    match number(call)? {
        None => tabs::step(editor, false, None),
        Some(number) => {
            let index = page(editor, number)?;
            tabs::go(editor, index);
        }
    }
    Ok(())
}

/// `:tabp[revious] [N]`, `:tabN[ext] [N]`: the previous tab page, or N
/// back, wrapping.
pub(crate) fn ex_tab_previous(editor: &mut Editor, call: &ExCall<'_>) -> ExResult {
    admitted(editor)?;
    match number(call)? {
        Some(0) => Err("no tab 0".into()),
        count => {
            tabs::step(editor, true, count);
            Ok(())
        }
    }
}

/// `:tabfir[st]` / `:tabr[ewind]` and `:tabl[ast]`.
pub(crate) fn ex_tab_end(editor: &mut Editor, last: bool) -> ExResult {
    admitted(editor)?;
    let index = if last { tabs::count(editor) - 1 } else { 0 };
    tabs::go(editor, index);
    Ok(())
}

/// `:tabnew [file]`, `:tabe[dit] [file]`: a new tab page after this one,
/// on the file or on a new empty buffer.
pub(crate) fn ex_tab_new(editor: &mut Editor, call: &ExCall<'_>) -> ExResult {
    admitted(editor)?;
    tab_new(editor, optional(call.arg))
}

fn tab_new(editor: &mut Editor, path: Option<&str>) -> ExResult {
    if let Some(path) = path {
        editor.request_user_open(path, OpenIntent::Tab);
        return Ok(());
    }
    let Ok(id) = editor
        .docs
        .try_insert(Document::scratch(strop_core::Buffer::from_text("")))
    else {
        return Err("document identity space exhausted".into());
    };
    let pane = Pane::fresh(id, editor.view().jumps.clone());
    tabs::open(editor, pane);
    editor.touch_mru(id);
    Ok(())
}

/// `:tab split [file]` and `:tab new [file]`: the split in a new tab page.
/// `:tab split` shows this buffer alone, cursor and scroll kept.
pub(crate) fn ex_tab(editor: &mut Editor, call: &ExCall<'_>) -> ExResult {
    admitted(editor)?;
    let (word, rest) = call
        .arg
        .split_once(char::is_whitespace)
        .unwrap_or((call.arg, ""));
    let path = optional(rest.trim_start());
    match table::lookup(word).map(|(command, _)| command.name()) {
        Some("split") if path.is_none() => {
            let pane = editor.view().clone();
            tabs::open(editor, pane);
            Ok(())
        }
        Some("split" | "new") => tab_new(editor, path),
        _ => Err(":tab takes split or new".into()),
    }
}

/// `:tabc[lose][!] [N]`: this tab page, or tab page N. Its buffers stay
/// open, so `!` has nothing to force.
pub(crate) fn ex_tab_close(editor: &mut Editor, call: &ExCall<'_>) -> ExResult {
    admitted(editor)?;
    let closed = match number(call)? {
        None => tabs::close_current(editor),
        Some(number) => {
            let index = page(editor, number)?;
            tabs::close(editor, index)
        }
    };
    if closed {
        Ok(())
    } else {
        Err("cannot close the last tab".into())
    }
}

/// `:tabo[nly][!]`: every other tab page goes.
pub(crate) fn ex_tab_only(editor: &mut Editor, call: &ExCall<'_>) -> ExResult {
    admitted(editor)?;
    if !call.arg.is_empty() {
        return Err(format!("trailing characters: {}", call.arg));
    }
    tabs::only(editor);
    Ok(())
}

/// `:tabm[ove] [N|+N|-N|$]`: after tab page N (`0` first), to the end, or
/// N places right or left.
pub(crate) fn ex_tab_move(editor: &mut Editor, call: &ExCall<'_>) -> ExResult {
    admitted(editor)?;
    let count = tabs::count(editor);
    let current = tabs::current(editor);
    let index = move_target(call.arg, current, count)?;
    tabs::move_to(editor, index);
    Ok(())
}

/// Where `:tabm {arg}` puts the current tab page (0-based `current` of
/// `count`), plans/0113 §3.
fn move_target(arg: &str, current: usize, count: usize) -> Result<usize, String> {
    let steps = |digits: &str| -> Result<usize, String> {
        if digits.is_empty() {
            return Ok(1);
        }
        digits
            .parse()
            .map_err(|_| format!("trailing characters: {arg}"))
    };
    if arg.is_empty() || arg == "$" {
        return Ok(count - 1);
    }
    if let Some(digits) = arg.strip_prefix('+') {
        let to = current.saturating_add(steps(digits)?);
        return if to < count {
            Ok(to)
        } else {
            Err("no tab there".into())
        };
    }
    if let Some(digits) = arg.strip_prefix('-') {
        return current
            .checked_sub(steps(digits)?)
            .ok_or_else(|| "no tab there".into());
    }
    let after: usize = arg
        .parse()
        .map_err(|_| format!("trailing characters: {arg}"))?;
    match after {
        after if after > count => Err(format!("no tab {after}")),
        after if after > current => Ok(after - 1),
        after => Ok(after),
    }
}

#[cfg(test)]
mod tests {
    use super::move_target;

    /// §3's `:tabm` rows for every current tab page of up to five.
    #[test]
    fn tabmove_targets_follow_the_formula() {
        for count in 1..=5 {
            for current in 0..count {
                for after in 0..=count + 1 {
                    let expected = match after {
                        after if after > count => Err(format!("no tab {after}")),
                        after if after > current => Ok(after - 1),
                        after => Ok(after),
                    };
                    assert_eq!(move_target(&after.to_string(), current, count), expected);
                }
                assert_eq!(move_target("", current, count), Ok(count - 1));
                assert_eq!(move_target("$", current, count), Ok(count - 1));
                for steps in 0..=6 {
                    let right = (current + steps < count)
                        .then_some(current + steps)
                        .ok_or_else(|| "no tab there".to_string());
                    let left = current
                        .checked_sub(steps)
                        .ok_or_else(|| "no tab there".to_string());
                    assert_eq!(move_target(&format!("+{steps}"), current, count), right);
                    assert_eq!(move_target(&format!("-{steps}"), current, count), left);
                }
                assert_eq!(
                    move_target("+", current, count),
                    move_target("+1", current, count)
                );
            }
        }
    }
}
