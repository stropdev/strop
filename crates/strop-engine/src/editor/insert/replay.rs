//! Insert intent distinguishes keys from verbatim accepted/pasted text.
use crate::editor::{completion::acceptance::SnippetStop, resolution::DeferredInput, Editor};
use strop_core::{frontend_input::Key, id::ByteOffset};

#[derive(Debug, Clone, Copy)]
pub enum InsertTargets {
    Primary,
    All,
}

#[derive(Debug, Clone)]
pub struct RelativeSnippetStop {
    index: u32,
    start_from_end: isize,
    end_from_end: isize,
}

#[derive(Debug, Clone)]
pub enum InsertStep {
    Key(Key),
    Literal {
        text: String,
        targets: InsertTargets,
    },
    Snippet(Vec<RelativeSnippetStop>),
}

impl Editor {
    pub(crate) fn record_insert_key(&mut self, key: Key) {
        if let Some(recording) = &mut self.insert_replay {
            recording.push(InsertStep::Key(key));
        }
    }

    pub(crate) fn replay_insert_key(&mut self, key: Key) {
        self.replay_insert_step(InsertStep::Key(key));
    }

    /// Generated insert intent keeps typeahead ordering without recording an
    /// expanded dot/count sequence into a user's macro a second time.
    pub(crate) fn replay_insert_step(&mut self, step: InsertStep) {
        if self.resolution.blocked()
            || !self.resolution.staged.is_empty()
            || self.completion.defers_input()
        {
            self.resolution.staged.push_back(DeferredInput::InsertStep {
                step,
                depth: self.macro_depth,
            });
        } else {
            self.apply_insert_step(step);
        }
    }

    pub(crate) fn apply_insert_step(&mut self, step: InsertStep) {
        match step {
            InsertStep::Key(key) => self.feed_insert(key),
            InsertStep::Literal { text, targets } => self.insert_literal(&text, targets),
            InsertStep::Snippet(stops) => {
                let cursors = self.all_cursors();
                let restored: Option<Vec<SnippetStop>> = stops
                    .iter()
                    .map(|stop| {
                        let offsets: Option<Vec<_>> = cursors
                            .iter()
                            .map(|&cursor| {
                                Some((
                                    cursor.checked_add_signed(stop.start_from_end)?,
                                    cursor.checked_add_signed(stop.end_from_end)?,
                                ))
                            })
                            .collect();
                        Some(SnippetStop {
                            index: stop.index,
                            offsets: offsets?,
                        })
                    })
                    .collect();
                let Some(restored) = restored else {
                    self.message = "snippet repeat has no preceding placeholder text".into();
                    return;
                };
                if let Some(recording) = &mut self.insert_replay {
                    recording.push(InsertStep::Snippet(stops));
                }
                self.snippet_enter(restored);
            }
        }
    }

    pub(crate) fn insert_literal(&mut self, text: &str, targets: InsertTargets) {
        let mut cursors = match targets {
            InsertTargets::Primary => vec![self.head()],
            InsertTargets::All => self.all_cursors(),
        };
        cursors.sort_unstable();
        cursors.dedup();
        let edits: Vec<_> = cursors
            .into_iter()
            .map(|cursor| {
                let position = ByteOffset::new(cursor);
                crate::editor::structural_indent::Edit {
                    cursor: position,
                    start: position,
                    end: position,
                    text: text.to_owned(),
                    landing: text.len(),
                }
            })
            .collect();
        self.tx_begin();
        self.apply_indentation(&edits);
        if let Some(recording) = &mut self.recording_insert {
            recording.push_str(text);
        }
        if let Some(recording) = &mut self.insert_replay {
            recording.push(InsertStep::Literal {
                text: text.to_owned(),
                targets,
            });
        }
    }

    pub(crate) fn record_snippet_replay(&mut self, stops: &[SnippetStop], accepted_end: usize) {
        let relative = stops
            .iter()
            .filter_map(|stop| {
                let &(start, end) = stop.offsets.first()?;
                Some(RelativeSnippetStop {
                    index: stop.index,
                    start_from_end: start as isize - accepted_end as isize,
                    end_from_end: end as isize - accepted_end as isize,
                })
            })
            .collect();
        if let Some(recording) = &mut self.insert_replay {
            recording.push(InsertStep::Snippet(relative));
        }
    }
}
