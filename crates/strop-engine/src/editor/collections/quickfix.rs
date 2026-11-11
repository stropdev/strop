//! The quickfix list (0094 §3.1): the newest collection's hits, under the
//! names vim gave them. `:copen` shows the list, `]q`/`[q` and
//! `:cnext`/`:cprev`/`:cc N` visit a hit in its source, `:cdo` runs an ex
//! command at every hit and `:cfdo` once per file. Collecting loaded every
//! source already, so nothing here waits on a job.
use super::Editor;
use strop_core::id::DocumentId;

impl Editor {
    /// The list, or the reason there is none.
    fn quickfix_list(&mut self) -> Option<DocumentId> {
        let list = self
            .changes
            .quickfix
            .filter(|id| self.collections.contains_key(id));
        if list.is_none() {
            self.changes.quickfix = None;
            self.message =
                "no quickfix list — Ctrl-O in search results, references or diagnostics makes one"
                    .into();
        }
        list
    }

    /// Every hit as `(source, byte)`, in list order. The byte is the
    /// match's start when the hit came from Search, else its line's start.
    fn quickfix_hits(&self, list: DocumentId) -> Vec<(DocumentId, usize)> {
        let Some(collection) = self.collections.get(&list) else {
            return Vec::new();
        };
        let mut hits = Vec::new();
        for excerpt in &collection.excerpts {
            let Some(source) = self.docs.get(excerpt.source) else {
                continue;
            };
            let mut anchors = excerpt.hit_anchors.clone();
            anchors.sort_unstable();
            anchors.dedup();
            for anchor in anchors {
                let anchor = anchor.min(source.buf.len_bytes());
                let line_end = source.buf.line_end(source.buf.line_of(anchor));
                let at = excerpt
                    .matches
                    .iter()
                    .map(|(start, _)| *start)
                    .find(|start| *start >= anchor && *start <= line_end)
                    .unwrap_or(anchor);
                hits.push((excerpt.source, source.buf.clamp_boundary(at)));
            }
        }
        hits
    }

    /// `:copen`: show the list.
    pub(crate) fn quickfix_open(&mut self) {
        let Some(list) = self.quickfix_list() else {
            return;
        };
        if self.current() != list {
            self.push_jump();
            self.switch_to(list);
        }
        let count = self.quickfix_hits(list).len();
        self.message =
            format!("quickfix: {count} hit(s) — ]q [q visit, :cdo runs a command at each");
    }

    /// `]q` / `[q`, `:cnext` / `:cprev`: the hit `delta` away from the
    /// last one visited.
    pub fn quickfix_step(&mut self, delta: isize) {
        let Some(list) = self.quickfix_list() else {
            return;
        };
        let cursor = self.collections[&list].cursor;
        let target = match (cursor, delta >= 0) {
            (None, true) => Some(delta.unsigned_abs().saturating_sub(1)),
            (None, false) => None,
            (Some(at), _) => at.checked_add_signed(delta),
        };
        match target {
            Some(index) => self.quickfix_visit(list, index),
            None => self.message = "quickfix: first hit".into(),
        }
    }

    /// `:cc N`: hit N, counted from one.
    pub(crate) fn quickfix_goto(&mut self, argument: &str) {
        let Some(list) = self.quickfix_list() else {
            return;
        };
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
        self.quickfix_visit(list, index);
    }

    fn quickfix_visit(&mut self, list: DocumentId, index: usize) {
        let hits = self.quickfix_hits(list);
        let Some(&(source, at)) = hits.get(index) else {
            self.message = if hits.is_empty() {
                "quickfix: the list is empty".into()
            } else {
                "quickfix: last hit".into()
            };
            return;
        };
        if let Some(collection) = self.collections.get_mut(&list) {
            collection.cursor = Some(index);
        }
        self.push_jump();
        self.jump_land(source, at);
        self.lsp_attach_document(self.current());
        self.message = format!(
            "quickfix {} of {}: {}:{}",
            index + 1,
            hits.len(),
            self.cur().label(&self.cwd),
            self.buf().line_of(at) + 1
        );
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
        let Some(list) = self.quickfix_list() else {
            return;
        };
        let origin = (self.current(), self.head());
        let mut ran = 0;
        let mut files: Vec<DocumentId> = Vec::new();
        let mut index = 0;
        // Hits are re-read every round: the command edits the sources and
        // the list's anchors move with them.
        while let Some(&(source, at)) = self.quickfix_hits(list).get(index) {
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
            format!(":cfdo {command} — ran in {ran} file(s)")
        } else {
            format!(
                ":cdo {command} — ran at {ran} hit(s) in {} file(s)",
                files.len()
            )
        };
    }
}
