//! Owned snapshot publication preserves source-relative views and saved anchors.
use super::Document;
use crate::editor::Editor;
use ropey::Rope;
use strop_core::id::DocumentId;

#[derive(Debug, thiserror::Error)]
pub(crate) enum SnapshotError {
    #[error(transparent)]
    Edit(#[from] strop_core::EditError),
    #[error("filename draft source identity changed; draft retained; discard explicitly before rebinding")]
    DraftScopeChanged,
}

impl Editor {
    /// Replace text through the shared mutation gateway, then restore meaningful
    /// view positions. Only small view/mark metadata is copied on the event loop.
    pub(crate) fn publish_source_snapshot(
        &mut self,
        document: DocumentId,
        mut replacement: Document,
        following: bool,
    ) -> Result<(), SnapshotError> {
        let retain_draft = self
            .doc(document)
            .directory_metadata_ref()
            .and_then(|source| source.draft.as_ref())
            .is_some_and(|draft| {
                !matches!(
                    draft.phase,
                    crate::editor::filesystem::draft::Phase::Reloading
                )
            });
        if retain_draft {
            let fresh = replacement
                .directory_metadata_ref()
                .ok_or(SnapshotError::DraftScopeChanged)?;
            let source = self
                .docs
                .get_mut(document)
                .and_then(|doc| doc.directory_metadata_mut())
                .ok_or(SnapshotError::DraftScopeChanged)?;
            if source.location != fresh.location {
                return Err(SnapshotError::DraftScopeChanged);
            }
            let draft = source
                .draft
                .as_mut()
                .ok_or(SnapshotError::DraftScopeChanged)?;
            draft.latest = Some(strop_workspace::DirectorySnapshot {
                location: fresh.location.clone(),
                entries: fresh.entries.clone(),
                state: fresh.state.clone(),
            });
            source.stale = Some("filesystem listing refreshed; filename draft retained and sources will be revalidated".into());
            return Ok(());
        }
        let before = self.doc(document).buf.snapshot();
        let old_window = self
            .doc(document)
            .remote_metadata()
            .map(|source| source.window);
        let new_window = replacement.remote_metadata().map(|source| source.window);
        let old_directory = self.doc(document).directory_metadata_ref().cloned();
        let new_directory = replacement.directory_metadata_ref().cloned();
        let after = replacement.buf.snapshot();
        let old_tail = following.then(|| last_position(&before));
        let content_tail = last_position(&after);
        let new_tail = following.then_some(content_tail);
        let view_rows = self.view_rows();
        let position = |offset: usize| {
            let offset = if following {
                let old_start = old_window.map_or(0, |window| window.start().get());
                let new_start = new_window.map_or(0, |window| window.start().get());
                old_start
                    .saturating_add(offset as u64)
                    .saturating_sub(new_start)
                    .min(after.len_bytes() as u64) as usize
            } else {
                let offset = offset.min(before.len_bytes());
                let line = before.byte_to_line(offset);
                let column = offset - before.line_to_byte(line);
                let line = if let (Some(old), Some(new)) = (&old_directory, &new_directory) {
                    old.restoration_location(strop_core::id::LineIndex::new(line))
                        .and_then(|location| new.line_for(&location))
                        .map(|line| line.get())
                        .unwrap_or(line.min(1))
                } else {
                    line
                };
                let line = line.min(after.len_lines().saturating_sub(1));
                let start = after.line_to_byte(line);
                let mut end = if line + 1 < after.len_lines() {
                    after.line_to_byte(line + 1)
                } else {
                    after.len_bytes()
                };
                while end > start && matches!(after.byte(end - 1), b'\n' | b'\r') {
                    end -= 1;
                }
                start.saturating_add(column).min(end)
            };
            after.char_to_byte(after.byte_to_char(offset.min(content_tail)))
        };
        let views: Vec<_> = self
            .panes
            .iter()
            .enumerate()
            .filter(|(_, pane)| pane.doc == document)
            .map(|(index, pane)| {
                (
                    index,
                    old_tail == Some(pane.sels.primary().head),
                    before.line_to_byte(pane.view_top.min(before.len_lines().saturating_sub(1))),
                )
            })
            .collect();
        if replacement.return_point().is_none() {
            if let Some(point) = self.doc(document).return_point().cloned() {
                replacement.set_return_point(point);
            }
        }
        self.doc_mut(document)
            .replace_snapshot(after.clone(), position)?;
        if let Some(doc) = self.docs.get_mut(document) {
            if matches!(replacement.source, super::DocumentSource::File) {
                doc.buf.adopt_file_binding(&replacement.buf);
            }
            doc.source = replacement.source;
            doc.buf.name = replacement.buf.name;
            doc.buf.readonly = replacement.buf.readonly;
            doc.buf.dirty = false;
            let revision = doc.buf.revision();
            if let Some(source) = doc.directory_metadata_mut() {
                source.view_revision = revision;
                if let Some(draft) = source.draft.as_mut() {
                    draft.revision = revision;
                }
            }
        }
        for (index, was_tail, old_top) in views {
            let pane = &mut self.panes[index];
            pane.view_top = if let Some(tail) = new_tail.filter(|_| was_tail) {
                pane.sels.set_head(tail);
                pane.desired_column = None;
                after
                    .byte_to_line(tail)
                    .saturating_sub(view_rows.saturating_sub(1))
            } else {
                after.byte_to_line(position(old_top))
            };
        }
        if !self.docs.is_empty() && self.current() == document {
            self.clamp_cursor();
        }
        Ok(())
    }
}
pub(crate) fn last_position(rope: &Rope) -> usize {
    let mut line = rope.len_lines().saturating_sub(1);
    if line > 0 && rope.line_to_byte(line) == rope.len_bytes() {
        line -= 1;
    }
    let start = rope.line_to_byte(line);
    let end = if line + 1 < rope.len_lines() {
        rope.line_to_byte(line + 1)
    } else {
        rope.len_bytes()
    };
    let mut end = end;
    while end > start && matches!(rope.byte(end - 1), b'\n' | b'\r') {
        end -= 1;
    }
    if end == start {
        return start;
    }
    let mut cursor = unicode_segmentation::GraphemeCursor::new(end, rope.len_bytes(), true);
    let (mut chunk, mut chunk_start, _, _) = rope.chunk_at_byte(end - 1);
    loop {
        match cursor.prev_boundary(chunk, chunk_start) {
            Ok(Some(byte)) => return byte.max(start),
            Ok(None) => return start,
            Err(unicode_segmentation::GraphemeIncomplete::PrevChunk) => {
                (chunk, chunk_start, _, _) = rope.chunk_at_byte(chunk_start - 1);
            }
            Err(unicode_segmentation::GraphemeIncomplete::PreContext(end)) => {
                let (context, offset, _, _) = rope.chunk_at_byte(end - 1);
                cursor.provide_context(&context[..end - offset], offset);
            }
            Err(other) => unreachable!("backward grapheme traversal: {other:?}"),
        }
    }
}
