//! Snippet tabstop sessions (0069 D05): after a snippet completion applies,
//! the first placeholder is selected; typing replaces it, Tab/Shift-Tab
//! cycle the stops (linked indexes move together, mirrored targets become
//! extra cursors), and `$0`/exhaustion/Esc ends the session. The expansion
//! itself stays the acceptance's single undo unit; placeholder edits join
//! the insert session's undo unit like any typed text.

use strop_core::frontend_input::Key;

use super::completion::acceptance::SnippetStop;

/// The renderer's view of the active tabstop: primary range plus linked
/// stops and mirrored targets.
pub struct SnippetRanges(pub Option<(usize, usize)>, pub Vec<(usize, usize)>);
use super::Editor;

/// The live tabstop session: every stop of the snippet, the unique
/// indexes in visit order, and the active position in that order.
pub(crate) struct SnippetSession {
    stops: Vec<SnippetStop>,
    /// Unique indexes ascending, with `$0` (index 0) always last.
    order: Vec<u32>,
    active: usize,
}

/// What one key did to the session.
pub(crate) enum SnippetKeyOutcome {
    /// The session consumed the key.
    Consumed,
    /// The session ended; the key belongs to normal insert handling.
    EndAndForward,
    /// Not a session key — insert normally (placeholder replaced first,
    /// if any was still selected).
    PassThrough,
}

impl SnippetSession {
    /// The active index's ranges, primary first — every occurrence of the
    /// active tabstop (linked stops and mirrored targets).
    fn active_ranges(&self) -> Vec<(usize, usize)> {
        let Some(index) = self.order.get(self.active) else {
            return Vec::new();
        };
        self.stops
            .iter()
            .filter(|stop| stop.index == *index)
            .flat_map(|stop| stop.offsets.iter().copied())
            .collect()
    }
}

impl Editor {
    /// Enter the tabstop session at the first placeholder (or the final
    /// stop when the snippet has none).
    pub fn snippet_enter(&mut self, stops: Vec<SnippetStop>) {
        let mut order: Vec<u32> = stops.iter().map(|stop| stop.index).collect();
        order.sort_unstable();
        order.dedup();
        order.retain(|index| *index != 0);
        order.push(0);
        let session = SnippetSession {
            stops,
            order,
            active: 0,
        };
        // a $0-only snippet is a caret landing, not a session
        if session.order.len() == 1 && session.order[0] == 0 {
            if let Some((start, _)) = session.active_ranges().first() {
                self.sels_mut().collapse_primary(*start);
            }
            return;
        }
        self.snippet = Some(session);
        self.snippet_select();
    }

    /// Paint/select the active stop: primary selection on the first
    /// occurrence, extras on linked stops and mirrored targets. An empty
    /// placeholder collapses to a caret there.
    fn snippet_select(&mut self) {
        let Some(session) = &self.snippet else {
            return;
        };
        let ranges = session.active_ranges();
        if let Some(&(start, end)) = ranges.first() {
            self.sels_mut().stretch_primary(start, end);
            self.sels_mut().set_extra_selections(
                ranges
                    .iter()
                    .skip(1)
                    .map(|&(anchor, head)| strop_core::selection::Selection { anchor, head })
                    .collect::<Vec<strop_core::selection::Selection>>(),
            );
        }
    }

    /// The active placeholder ranges for the renderer (active index only):
    /// `(primary, extras)` in byte offsets.
    pub fn snippet_selection_ranges(&self) -> SnippetRanges {
        let Some(session) = &self.snippet else {
            return SnippetRanges(None, Vec::new());
        };
        let mut ranges = session.active_ranges();
        let primary = (!ranges.is_empty()).then(|| ranges.remove(0));
        SnippetRanges(primary, ranges)
    }

    /// Whether a tabstop session is live (the renderer paints it in
    /// Insert mode).
    pub fn snippet_active(&self) -> bool {
        self.snippet.is_some()
    }

    /// Map every stop endpoint through one edit mapping (the same remap
    /// cursors take through mirrored insert edits and the placeholder
    /// deletion).
    pub(crate) fn snippet_map(&mut self, map: impl Fn(usize) -> usize) {
        let Some(session) = self.snippet.as_mut() else {
            return;
        };
        for stop in &mut session.stops {
            for offset in &mut stop.offsets {
                *offset = (map(offset.0), map(offset.1));
            }
        }
    }

    /// One key against the session (called from the insert path before
    /// normal handling).
    pub(crate) fn snippet_key(&mut self, key: Key) -> SnippetKeyOutcome {
        let Some(session) = self.snippet.as_mut() else {
            return SnippetKeyOutcome::EndAndForward;
        };
        match key {
            Key::Tab | Key::Backtab => {
                let next = if key == Key::Tab {
                    session.active + 1
                } else {
                    session.active.saturating_sub(1)
                };
                let beyond = session.order.len().saturating_sub(1);
                if key == Key::Tab && session.active > beyond {
                    return SnippetKeyOutcome::EndAndForward;
                }
                if key == Key::Backtab && session.active == 0 {
                    return SnippetKeyOutcome::EndAndForward;
                }
                session.active = next.min(session.order.len() - 1);
                // landing on $0 (the trailing final stop) ends the session
                let final_stop =
                    session.order[session.active] == 0 && session.active == session.order.len() - 1;
                if final_stop {
                    let ranges = session.active_ranges();
                    self.snippet = None;
                    if let Some(&(start, _)) = ranges.first() {
                        self.sels_mut().collapse_primary(start);
                        self.sels_mut().set_extra_selections(Vec::new());
                    }
                    return SnippetKeyOutcome::EndAndForward;
                }
                self.snippet_select();
                SnippetKeyOutcome::Consumed
            }
            Key::Esc => {
                self.snippet = None;
                SnippetKeyOutcome::EndAndForward
            }
            Key::Char(_) | Key::Backspace | Key::Enter => {
                // Replace the selected placeholder(s) with what comes
                // next: delete every active-index range, collapse each to
                // its start, and let normal insert handling type there —
                // mirrored cursors make linked stops follow. The stop is
                // then SPENT: its ranges collapse, so later typing is
                // never re-deleted.
                let ranges = session.active_ranges();
                let non_empty: Vec<(usize, usize)> =
                    ranges.into_iter().filter(|(a, b)| a < b).collect();
                if !non_empty.is_empty() {
                    let mut edits: Vec<strop_core::Replacement> = non_empty
                        .iter()
                        .map(|&(start, end)| {
                            strop_core::Replacement::new(
                                strop_core::Range::charwise(start, end),
                                "",
                            )
                        })
                        .collect();
                    edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
                    let _ = self.apply(
                        self.current(),
                        self.buf().revision(),
                        crate::editor::transact::ChangeSet {
                            edits,
                            undo_open: true,
                        },
                    );
                    // the deletion moves every later stop back, matching
                    // the transaction's descending application order —
                    // and collapses the deleted ranges to their (shifted)
                    // starts, which is exactly where cursors belong.
                    {
                        let ranges = non_empty.clone();
                        self.snippet_map(|offset| {
                            let mut mapped = offset;
                            for &(start, end) in ranges.iter().rev() {
                                mapped =
                                    strop_core::editmap::map_position(mapped, start, end, start);
                            }
                            mapped
                        });
                    }
                    let Some(session) = self.snippet.as_mut() else {
                        return SnippetKeyOutcome::PassThrough;
                    };
                    let collapsed = session.active_ranges();
                    if let Some(&(first, _)) = collapsed.first() {
                        self.sels_mut().collapse_primary(first);
                        self.sels_mut()
                            .set_extras(collapsed.iter().skip(1).map(|&(start, _)| start));
                    }
                }
                SnippetKeyOutcome::PassThrough
            }
            _ => {
                // navigation or any other key leaves the session
                self.snippet = None;
                SnippetKeyOutcome::EndAndForward
            }
        }
    }
}
