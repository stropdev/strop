//! Explicit context resizing retains the active source position.
use super::{Editor, Excerpt};

impl Editor {
    pub(crate) fn collection_context_step(&mut self, expand: bool) {
        let id = self.current();
        let head = self.head();
        let line = self.buf().line_of(head);
        let Some(collection) = self.collections.get(&id) else {
            return;
        };
        let Some(index) = collection.excerpts.iter().position(|excerpt| {
            line >= excerpt.view_line && line <= excerpt.view_line + excerpt.view_lines
        }) else {
            self.message = "place the caret on an excerpt or its gap/header".into();
            return;
        };
        let excerpt = &collection.excerpts[index];
        let source = excerpt.source;
        let offset = excerpt.start
            + head
                .saturating_sub(excerpt.view_start)
                .min(excerpt.end - excerpt.start);
        let context = if expand {
            excerpt.context.saturating_add(1)
        } else {
            excerpt.context.saturating_sub(1)
        };
        let Some(document) = self.docs.get(source) else {
            return;
        };
        let mut resized: Vec<Excerpt> = Vec::new();
        // Shrinking can split an earlier merged excerpt back into disjoint
        // spans. Keep each original hit anchor, including same-line matches.
        for &anchor in &excerpt.hit_anchors {
            let row = document.buf.line_of(anchor.min(document.buf.len_bytes()));
            let start = document.buf.line_start(row.saturating_sub(context));
            let end_line = row.saturating_add(context).saturating_add(1);
            let end = if end_line >= document.buf.len_lines() {
                document.buf.len_bytes()
            } else {
                document.buf.line_start(end_line)
            };
            if let Some(previous) = resized.last_mut().filter(|e| e.end >= start) {
                previous.end = previous.end.max(end);
                previous.hit_anchors.push(anchor);
            } else {
                let mut part = excerpt.clone();
                part.start = start;
                part.end = end;
                part.context = context;
                part.hit_anchors = vec![anchor];
                part.matches.clear();
                resized.push(part);
            }
        }
        for part in &mut resized {
            part.matches = excerpt
                .matches
                .iter()
                .copied()
                .filter(|(start, _)| *start >= part.start && *start < part.end)
                .collect();
        }
        let collection = self.collections.get_mut(&id).unwrap();
        collection.excerpts.splice(index..=index, resized);
        let mut at = 0;
        while at + 1 < collection.excerpts.len() {
            let left = &collection.excerpts[at];
            let right = &collection.excerpts[at + 1];
            if left.source == right.source && left.end >= right.start {
                let right = collection.excerpts.remove(at + 1);
                let left = &mut collection.excerpts[at];
                left.end = left.end.max(right.end);
                left.context = left.context.max(right.context);
                left.hit_anchors.extend(right.hit_anchors);
                left.hit_anchors.sort_unstable();
                left.hit_anchors.dedup();
                left.matches.extend(right.matches);
                left.matches.sort_unstable();
                left.matches.dedup();
            } else {
                at += 1;
            }
        }
        self.collection_render_view(id);
        if let Some(excerpt) = self.collections[&id]
            .excerpts
            .iter()
            .find(|e| e.source == source && offset >= e.start && offset <= e.end)
        {
            self.set_head(excerpt.view_start + offset - excerpt.start);
            self.clamp_cursor();
            self.scroll_to_cursor(self.view_rows());
        }
        self.message = format!("collection context: {context} line(s); + expand, - contract");
    }
}
