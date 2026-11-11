//! The status buffer's own keys (0096 §2.1), fugitive's: `s` `u` `-`
//! `=` `X` `cc` `ca` `enter` `]c` `[c`. Each acts on the row under the
//! cursor — a file, a hunk, or a whole section from its heading.
//!
//! Pure: a key, the view and the cursor line decide an [`Action`] as
//! data; the editor carries it out. No decision here touches git.

use std::path::PathBuf;
use std::sync::Arc;

use strop_core::frontend_input::Key;
use strop_git::{Change, Hunk, Section, StatusEntry};

use super::{StatusRow, StatusView};
use crate::editor::git_memory::MutationOp;

/// A two-key sequence waiting for its second key.
pub(super) enum Step {
    /// `c` pressed: `c` commits, `a` amends.
    Commit,
    /// `X` pressed: `y` runs the discard.
    Discard { rel: PathBuf, op: MutationOp },
}

/// What a key in the status buffer comes to.
pub(super) enum Action {
    /// Not a status key: the shared readonly path takes it (motions,
    /// search, yank, `q`).
    Pass,
    /// Nothing to do, and why.
    Say(String),
    /// Queue an index or working-tree mutation.
    Mutate {
        rel: PathBuf,
        op: MutationOp,
    },
    /// Ask first; `y` then runs the step.
    Ask {
        prompt: String,
        step: Step,
    },
    /// `c`: wait for `c` or `a`.
    Await(Step),
    Commit {
        amend: bool,
    },
    /// `=`: show or hide this entry's diff.
    ToggleDiff(StatusEntry),
    /// `enter`: open the entry, at this 1-based line when known.
    Open {
        entry: StatusEntry,
        line: Option<usize>,
    },
}

/// What the cursor row stands for.
enum Target {
    /// A file row, or every file under a section heading.
    Files(Vec<StatusEntry>),
    Hunk {
        entry: StatusEntry,
        hunk: Arc<Hunk>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Move {
    Stage,
    Unstage,
    Toggle,
}

const CONFIRM: &str = "y confirms, any other key cancels";
const UNSTAGED_ONLY: &str = "discard works on unstaged changes — u unstages first";
const NOTHING_HERE: &str = "no file or hunk here";

fn say(text: &str) -> Action {
    Action::Say(text.into())
}

fn name(entry: &StatusEntry) -> String {
    entry.rel.to_string_lossy().into_owned()
}

fn target(view: &StatusView, line: usize) -> Option<Target> {
    match view.row(line)? {
        StatusRow::Section(section) => {
            Some(Target::Files(view.tree.section(section).cloned().collect()))
        }
        StatusRow::File(index) | StatusRow::Note(index) => {
            Some(Target::Files(vec![view.entry(index)?.clone()]))
        }
        row => {
            let (entry, hunk) = row.hunk()?;
            Some(Target::Hunk {
                entry: view.entry(entry)?.clone(),
                hunk: Arc::new(view.hunk(entry, hunk)?.clone()),
            })
        }
    }
}

/// One key at `line`, after `pending` (the first key of `cc`/`ca`, or
/// an armed discard).
pub(super) fn decide(view: &StatusView, line: usize, key: Key, pending: Option<Step>) -> Action {
    match pending {
        Some(Step::Discard { rel, op }) => {
            return if key == Key::Char('y') {
                Action::Mutate { rel, op }
            } else {
                say("discard cancelled")
            };
        }
        Some(Step::Commit) => match key {
            Key::Char('c') => return Action::Commit { amend: false },
            Key::Char('a') => return Action::Commit { amend: true },
            _ => {}
        },
        None => {}
    }
    match key {
        Key::Char('s') => stage(view, line, Move::Stage),
        Key::Char('u') => stage(view, line, Move::Unstage),
        Key::Char('-') => stage(view, line, Move::Toggle),
        Key::Char('X') => discard(view, line),
        Key::Char('c') => Action::Await(Step::Commit),
        Key::Char('=') => match view
            .row(line)
            .and_then(StatusRow::entry)
            .and_then(|index| view.entry(index))
        {
            Some(entry) if entry.is_directory() => {
                say("a directory has no diff — enter browses it")
            }
            Some(entry) => Action::ToggleDiff(entry.clone()),
            None => say("no file here"),
        },
        Key::Enter => open(view, line),
        _ => Action::Pass,
    }
}

/// `s` / `u` / `-`.
fn stage(view: &StatusView, line: usize, wanted: Move) -> Action {
    let Some(target) = target(view, line) else {
        return say(NOTHING_HERE);
    };
    let to_index = |staged: bool| match wanted {
        Move::Stage => true,
        Move::Unstage => false,
        Move::Toggle => !staged,
    };
    let refusal = |stage: bool| {
        say(if stage {
            "already staged"
        } else {
            "not staged"
        })
    };
    match target {
        Target::Files(entries) => {
            let staged = entries.iter().all(|entry| entry.section == Section::Staged);
            let stage = to_index(staged);
            let entries: Vec<StatusEntry> = entries
                .into_iter()
                .filter(|entry| (entry.section != Section::Staged) == stage)
                .collect();
            let Some(first) = entries.first() else {
                return refusal(stage);
            };
            Action::Mutate {
                rel: first.rel.clone(),
                op: if stage {
                    MutationOp::StageFiles { entries }
                } else {
                    MutationOp::UnstageFiles { entries }
                },
            }
        }
        Target::Hunk { entry, hunk } => {
            let staged = entry.section == Section::Staged;
            let stage = to_index(staged);
            if stage == staged {
                refusal(stage)
            } else if entry.section == Section::Conflicts {
                say("resolve the conflict, then stage the file")
            } else if entry.section == Section::Untracked {
                // an untracked file is one all-new hunk: staging it is
                // adding the file, which keeps its mode
                Action::Mutate {
                    rel: entry.rel.clone(),
                    op: MutationOp::StageFiles {
                        entries: vec![entry],
                    },
                }
            } else {
                Action::Mutate {
                    rel: entry.rel,
                    op: if stage {
                        MutationOp::Stage { hunk }
                    } else {
                        MutationOp::Unstage { hunk }
                    },
                }
            }
        }
    }
}

/// `X`: arm a discard. Only unstaged work and untracked paths — a
/// staged or conflicted row says to unstage first.
fn discard(view: &StatusView, line: usize) -> Action {
    let Some(target) = target(view, line) else {
        return say(NOTHING_HERE);
    };
    let (prompt, rel, op) = match target {
        Target::Files(entries) => {
            if entries
                .iter()
                .any(|entry| matches!(entry.section, Section::Staged | Section::Conflicts))
            {
                return say(UNSTAGED_ONLY);
            }
            let prompt = match entries.as_slice() {
                [] => return say("nothing to discard"),
                [entry] if entry.section == Section::Untracked => {
                    format!("delete untracked {}?", name(entry))
                }
                [entry] => format!("discard changes to {}?", name(entry)),
                entries if entries[0].section == Section::Untracked => {
                    format!("delete {} untracked paths?", entries.len())
                }
                entries => format!("discard changes to {} files?", entries.len()),
            };
            let rel = entries[0].rel.clone();
            (prompt, rel, MutationOp::DiscardFiles { entries })
        }
        Target::Hunk { entry, hunk } => match entry.section {
            Section::Unstaged => (
                format!("discard this hunk of {}?", name(&entry)),
                entry.rel,
                MutationOp::DiscardHunk { hunk },
            ),
            Section::Untracked => (
                format!("delete untracked {}?", name(&entry)),
                entry.rel.clone(),
                MutationOp::DiscardFiles {
                    entries: vec![entry],
                },
            ),
            Section::Staged | Section::Conflicts => return say(UNSTAGED_ONLY),
        },
    };
    Action::Ask {
        prompt: format!("{prompt} {CONFIRM}"),
        step: Step::Discard { rel, op },
    }
}

/// `enter`: the file — at the diff row's line when there is one.
fn open(view: &StatusView, line: usize) -> Action {
    let Some(row) = view.row(line) else {
        return say("no file here");
    };
    let Some(entry) = row.entry().and_then(|index| view.entry(index)) else {
        return say("no file here");
    };
    if entry.change == Change::Deleted {
        return Action::Say(format!("{} is deleted", name(entry)));
    }
    let line = match row {
        StatusRow::HunkHeader { entry, hunk } => view.hunk(entry, hunk).map(|hunk| hunk.new_start),
        StatusRow::Line { entry, hunk, line } => view
            .diff_line(entry, hunk, line)
            .and_then(|line| line.new_lineno.or(line.old_lineno)),
        _ => None,
    };
    Action::Open {
        entry: entry.clone(),
        line,
    }
}

/// `]c` / `[c`: the next or previous hunk header; with no diff
/// expanded, the next or previous file. `Err` is why there is none.
pub(super) fn hunk_step(
    view: &StatusView,
    line: usize,
    forward: bool,
) -> Result<usize, &'static str> {
    let hunks = view
        .rows
        .iter()
        .any(|row| matches!(row, StatusRow::HunkHeader { .. }));
    let stop = |row: &StatusRow| {
        if hunks {
            matches!(row, StatusRow::HunkHeader { .. })
        } else {
            matches!(row, StatusRow::File(_))
        }
    };
    let found = if forward {
        view.rows
            .iter()
            .enumerate()
            .skip(line + 1)
            .find(|(_, row)| stop(row))
            .map(|(at, _)| at)
    } else {
        view.rows[..line.min(view.rows.len())]
            .iter()
            .rposition(stop)
    };
    found.ok_or(if hunks {
        "no more hunks"
    } else {
        "no more files"
    })
}
