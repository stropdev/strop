//! Binding (0095 §2.1): some targets name editor state the pure resolver
//! cannot see — a mark, a line of the window, "the last search". The
//! parser emits them unbound; this fills them in immediately before
//! resolution. Dot-repeat keeps the unbound command, so `.` reads the
//! mark or the search again.

use std::borrow::Cow;

use strop_grammar::{Command, Motion, Target, WindowLine};

use crate::editor::Editor;

/// `command` with its editor-bound target filled in; `Err` is the
/// message for a target that names nothing (an unset mark).
pub(crate) fn bind_command<'a>(
    editor: &Editor,
    command: &'a Command,
) -> Result<Cow<'a, Command>, String> {
    let target = match &command.target {
        Target::Motion(Motion::Mark {
            name,
            exact,
            at: None,
        }) => Target::Motion(Motion::Mark {
            name: *name,
            exact: *exact,
            at: Some(
                editor
                    .mark_offset(*name)
                    .ok_or_else(|| format!("mark {name} not set"))?,
            ),
        }),
        Target::Motion(Motion::Window { which, line: None }) => {
            let rows = editor.view_rows();
            let (top, count) = (editor.view_top(), command.count.unwrap_or(1));
            let line = match which {
                WindowLine::Top => top + count.saturating_sub(1),
                WindowLine::Bottom => {
                    (top + rows.saturating_sub(1)).saturating_sub(count.saturating_sub(1))
                }
                WindowLine::Middle => top + rows.saturating_sub(1) / 2,
            };
            Target::Motion(Motion::Window {
                which: *which,
                line: Some(line.min(editor.buf().last_content_line())),
            })
        }
        Target::Motion(
            motion @ (Motion::ScreenDown { column: None } | Motion::ScreenUp { column: None }),
        ) => {
            let column = editor.view().desired_column.map_or_else(
                || editor.buf().col_of(editor.head()),
                strop_core::id::DisplayColumn::get,
            );
            Target::Motion(if matches!(motion, Motion::ScreenDown { .. }) {
                Motion::ScreenDown {
                    column: Some(column),
                }
            } else {
                Motion::ScreenUp {
                    column: Some(column),
                }
            })
        }
        Target::Motion(Motion::RepeatFind { reverse }) => {
            let (ch, backward, till) = editor.last_find.ok_or("no previous find")?;
            let backward = backward ^ reverse;
            let mut bound = command.clone();
            bound.target = Target::Motion(Motion::FindChar { ch, till, backward });
            // Vim: repeating `t` from right before its target moves on
            // to the next one instead of standing still.
            if till && till_is_adjacent(editor, ch, backward) {
                bound.count = Some(command.count.unwrap_or(1) + 1);
            }
            return Ok(Cow::Owned(bound));
        }
        Target::Motion(Motion::RepeatSearch { reverse }) => {
            let search = editor.last_search.as_ref().ok_or("no previous search")?;
            Target::Motion(if search.backward ^ reverse {
                Motion::SearchBackward(search.query.clone())
            } else {
                Motion::Search(search.query.clone())
            })
        }
        Target::Match {
            backward,
            query: None,
        } => Target::Match {
            backward: *backward,
            query: Some(
                editor
                    .last_search
                    .as_ref()
                    .ok_or("no previous search")?
                    .query
                    .clone(),
            ),
        },
        _ => return Ok(Cow::Borrowed(command)),
    };
    let mut bound = command.clone();
    bound.target = target;
    Ok(Cow::Owned(bound))
}

/// The next `ch` in this direction is the character right beside the
/// cursor, where a `t` / `T` would not move.
fn till_is_adjacent(editor: &Editor, ch: char, backward: bool) -> bool {
    let head = editor.head();
    strop_grammar::find_character(editor.buf(), head.into(), ch, backward, 1).is_some_and(
        |target| {
            let landing = if backward {
                editor.buf().ceil_boundary(target.get() + 1)
            } else {
                editor.buf().clamp_boundary(target.get().saturating_sub(1))
            };
            landing == head
        },
    )
}
