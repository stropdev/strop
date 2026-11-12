//! The hits of a collection (0094 §3.1, 0097 §3): Vim's quickfix names over
//! the collection's own list. A collection has a current hit; in the
//! collection the cursor is it, so `]q`/`[q` step from the cursor and stay
//! there, and from any other buffer they visit the next hit in its file.
//! `Enter` opens the file under the cursor, `:copen` comes back to the
//! current hit, `:cdo` runs an ex command at every hit line and `:cfdo` once
//! per file. Collecting loaded every source already, so nothing here waits
//! on a job.
use super::{counted, hits_in_files, Editor, Excerpt};
use strop_core::id::DocumentId;
use strop_core::Buffer;

/// One hit of a collection: where it is in its file and in the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hit {
    pub source: DocumentId,
    /// Source byte: the matched text's start when the hit came from
    /// Search and is still there, else its line's start.
    pub at: usize,
    /// The same place in the collection's view.
    pub view: usize,
}

/// An excerpt's hits as source bytes, in order: every match still standing
/// on a hit line, or the line's start when none is (a reference, or a
/// match that has been edited).
pub(super) fn excerpt_hits(excerpt: &Excerpt, source: &Buffer) -> Vec<usize> {
    let mut anchors = excerpt.hit_anchors.clone();
    anchors.sort_unstable();
    anchors.dedup();
    let mut hits = Vec::new();
    for anchor in anchors {
        let anchor = anchor.min(source.len_bytes());
        let line_end = source.line_end(source.line_of(anchor));
        let before = hits.len();
        for (start, _) in &excerpt.matches {
            if *start >= anchor && *start <= line_end {
                hits.push(source.clamp_boundary(*start));
            }
        }
        hits[before..].sort_unstable();
        if hits.len() == before {
            hits.push(source.clamp_boundary(anchor));
        }
    }
    hits.dedup();
    hits
}

impl Editor {
    /// The collection the hit commands act on: the one in view, else the
    /// one last in view or last made. `None` leaves the reason in the
    /// message.
    fn hit_list(&mut self) -> Option<DocumentId> {
        if self.collections.contains_key(&self.current()) {
            self.changes.quickfix = Some(self.current());
        }
        let list = self
            .changes
            .quickfix
            .filter(|id| self.collections.contains_key(id));
        if list.is_none() {
            self.changes.quickfix = None;
            self.message =
                "nothing collected yet — Ctrl-O in search results, references or diagnostics collects them"
                    .into();
        }
        list
    }

    /// Every hit of a collection, in list order.
    pub(crate) fn collection_hits(&self, list: DocumentId) -> Vec<Hit> {
        let Some(collection) = self.collections.get(&list) else {
            return Vec::new();
        };
        let mut hits = Vec::new();
        for excerpt in &collection.excerpts {
            let Some(source) = self.docs.get(excerpt.source) else {
                continue;
            };
            for at in excerpt_hits(excerpt, &source.buf) {
                let offset = at.saturating_sub(excerpt.start);
                hits.push(Hit {
                    source: excerpt.source,
                    at,
                    view: (excerpt.view_start + offset).min(excerpt.view_end),
                });
            }
        }
        hits
    }

    fn files_of(hits: &[Hit]) -> usize {
        let mut files: Vec<DocumentId> = Vec::new();
        for hit in hits {
            if !files.contains(&hit.source) {
                files.push(hit.source);
            }
        }
        files.len()
    }

    fn set_current_hit(&mut self, list: DocumentId, index: usize) {
        if let Some(collection) = self.collections.get_mut(&list) {
            collection.cursor = Some(index);
        }
    }

    /// `hit 2 of 4: src/a.rs:11`.
    fn announce_hit(&mut self, hits: &[Hit], index: usize) {
        let Some(hit) = hits.get(index) else {
            return;
        };
        let Some(source) = self.docs.get(hit.source) else {
            return;
        };
        self.message = format!(
            "hit {} of {}: {}:{}",
            index + 1,
            hits.len(),
            source.label(&self.cwd),
            source.buf.line_of(hit.at) + 1
        );
    }

    /// `:copen`: show the collection, cursor on the current hit.
    pub(crate) fn quickfix_open(&mut self) {
        let Some(list) = self.hit_list() else {
            return;
        };
        let hits = self.collection_hits(list);
        if self.current() != list {
            self.push_jump();
            self.switch_to(list);
            let current = self.collections[&list].cursor.unwrap_or(0);
            if let Some(hit) = hits.get(current) {
                self.set_head(hit.view);
                self.clamp_cursor();
                self.place_jump_target();
            }
        }
        self.message = format!(
            "{} — ]q next, Enter opens the file, :cdo runs a command at each",
            hits_in_files(hits.len(), Self::files_of(&hits))
        );
    }

    /// `]q` / `[q`, `:cnext` / `:cprev`. In the collection: the hit after
    /// or before the cursor. Anywhere else: the hit after or before the
    /// current one, in its file.
    pub fn quickfix_step(&mut self, forward: bool) {
        let Some(list) = self.hit_list() else {
            return;
        };
        let hits = self.collection_hits(list);
        let target = if self.current() == list {
            let head = self.head();
            match forward {
                true => hits.iter().position(|hit| hit.view > head),
                false => hits.iter().rposition(|hit| hit.view < head),
            }
        } else {
            match (self.collections[&list].cursor, forward) {
                (None, true) => Some(0),
                (None, false) => None,
                (Some(at), true) => Some(at + 1),
                (Some(at), false) => at.checked_sub(1),
            }
        };
        match target.filter(|index| *index < hits.len()) {
            Some(index) => self.quickfix_visit(list, &hits, index),
            None if hits.is_empty() => self.message = "the collection has no hits left".into(),
            None => self.message = if forward { "last hit" } else { "first hit" }.into(),
        }
    }

    /// `:cc N`: hit N, counted from one.
    pub(crate) fn quickfix_goto(&mut self, argument: &str) {
        let Some(list) = self.hit_list() else {
            return;
        };
        let hits = self.collection_hits(list);
        let index = match argument.trim() {
            "" => self.collections[&list].cursor.unwrap_or(0),
            number => match number.parse::<usize>() {
                Ok(number) if number > 0 => number - 1,
                _ => {
                    self.message = ":cc takes a hit number, counted from 1".into();
                    return;
                }
            },
        };
        if index < hits.len() {
            self.quickfix_visit(list, &hits, index);
        } else {
            self.message = format!("there are {}", counted(hits.len(), "hit"));
        }
    }

    /// Make hit `index` current and put the cursor on it: in the
    /// collection when that is the buffer in view, else in its file.
    fn quickfix_visit(&mut self, list: DocumentId, hits: &[Hit], index: usize) {
        let hit = hits[index];
        self.set_current_hit(list, index);
        self.push_jump();
        if self.current() == list {
            self.set_head(hit.view);
            self.clamp_cursor();
            self.place_jump_target();
        } else {
            self.jump_land(hit.source, hit.at);
            self.lsp_attach_document(self.current());
        }
        self.announce_hit(hits, index);
    }

    /// `Enter` in a collection: open the file at the cursor. The hit on
    /// that line, or the nearest one above it, becomes current. False on a
    /// row that shows no file (the title, a gap, a card's lower border).
    pub(crate) fn collection_enter(&mut self) -> bool {
        let list = self.current();
        if !self.collections.contains_key(&list) {
            return false;
        }
        let Some((source, at)) = self.navigation_source() else {
            return false;
        };
        if self.docs.get(source).is_none() {
            self.message = "that file was closed".into();
            return true;
        }
        let line_end = self.buf().line_end(self.buf().line_of(self.head()));
        let hits = self.collection_hits(list);
        let current = hits
            .iter()
            .rposition(|hit| hit.view <= line_end)
            .unwrap_or(0);
        self.changes.quickfix = Some(list);
        self.set_current_hit(list, current);
        self.push_jump();
        self.jump_land(source, at);
        self.lsp_attach_document(self.current());
        true
    }

    /// `:cdo {command}` / `:cfdo {command}`: run an ex command with the
    /// cursor on every hit line, or on the first hit of every file. Focus
    /// returns to where it was.
    pub(crate) fn quickfix_do(&mut self, command: &str, per_file: bool) {
        let name = if per_file { "cfdo" } else { "cdo" };
        let command = command.trim().trim_start_matches(':');
        if command.is_empty() {
            self.message = format!(":{name} needs a command, e.g. :{name} s/old/new/g");
            return;
        }
        if crate::editor::ex::split_command(command)
            .is_some_and(|(nested, _, _)| matches!(nested.name, "cdo" | "cfdo"))
        {
            self.message = format!(":{name} does not nest");
            return;
        }
        let Some(list) = self.hit_list() else {
            return;
        };
        let origin = (self.current(), self.head());
        let mut ran = 0;
        let mut files: Vec<DocumentId> = Vec::new();
        let mut index = 0;
        // Lines are re-read every round: the command edits the sources and
        // the list's anchors move with them.
        while let Some(&(source, at)) = self.hit_lines(list).get(index) {
            index += 1;
            let seen = files.contains(&source);
            if !seen {
                files.push(source);
            }
            if per_file && seen {
                continue;
            }
            self.switch_to(source);
            self.set_head(at);
            crate::editor::ex::run_ex_in_batch(self, command);
            ran += 1;
        }
        if self.docs.get(origin.0).is_some() {
            self.switch_to(origin.0);
            let head = self
                .buf()
                .clamp_boundary(origin.1.min(self.buf().len_bytes()));
            self.set_head(head);
            self.clamp_cursor();
        }
        self.message = if per_file {
            format!(":cfdo {command} — ran in {}", counted(ran, "file"))
        } else {
            format!(
                ":cdo {command} — ran on {} in {}",
                counted(ran, "line"),
                counted(files.len(), "file")
            )
        };
    }

    /// Every hit line once, as `(source, byte)`: several matches on one
    /// line are one line to `:cdo`.
    fn hit_lines(&self, list: DocumentId) -> Vec<(DocumentId, usize)> {
        let mut lines: Vec<(DocumentId, usize)> = Vec::new();
        let mut last = None;
        for hit in self.collection_hits(list) {
            let Some(source) = self.docs.get(hit.source) else {
                continue;
            };
            let line = (hit.source, source.buf.line_of(hit.at));
            if last != Some(line) {
                lines.push((hit.source, hit.at));
                last = Some(line);
            }
        }
        lines
    }
}
