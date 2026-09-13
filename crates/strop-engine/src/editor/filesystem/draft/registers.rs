use super::*;
impl Editor {
    pub(crate) fn capture_filename_register(
        &self,
        ranges: impl IntoIterator<Item = (strop_core::Range, bool)>,
        deleted: bool,
        separators: bool,
    ) -> Option<Arc<FileRegister>> {
        let directory = self.directory()?;
        let draft = directory.draft.as_ref()?;
        if draft.error().is_some() || draft.revision != self.buf().revision() {
            return None;
        }
        let mut rows = Vec::new();
        let mut previous_break = false;
        for (range, linewise) in ranges {
            if !linewise {
                return None;
            }
            let start = range.start.get();
            let end = range.end.get();
            let first = self.buf().line_of(start);
            let last = self.buf().line_of(end);
            if start != self.buf().line_start(first)
                || (end != self.buf().line_start(last) && end != self.buf().len_bytes())
            {
                return None;
            }
            if separators && !rows.is_empty() && previous_break {
                rows.push(None);
            }
            let end_line = last + usize::from(end > self.buf().line_start(last));
            for line in first..end_line {
                let row = draft.geometry.rows.get(line)?;
                rows.push(match &row.origin {
                    Some(Origin::Original(index)) => Some(RegisterEntry {
                        source: draft.base.get(*index)?.clone(),
                        cut: deleted,
                    }),
                    Some(Origin::Copy(source)) => Some(RegisterEntry {
                        source: source.clone(),
                        cut: false,
                    }),
                    None => None,
                });
                if rows.len() > ROW_LIMIT {
                    return None;
                }
            }
            previous_break = end > start && self.buf().byte(end - 1) == b'\n';
        }
        rows.iter()
            .any(Option::is_some)
            .then(|| Arc::new(FileRegister { rows }))
    }

    pub(crate) fn filename_delete_hint(&mut self, range: strop_core::Range, linewise: bool) {
        let revision = self.buf().revision();
        let end = self.buf().len_bytes();
        let complete_last = linewise
            && range.end.get() == end
            && end > 0
            && self.buf().byte(end - 1) != b'\n'
            && range.start.get() <= self.buf().line_start(self.buf().line_of(end));
        let document = self.current();
        if let Some(draft) = self
            .docs
            .get_mut(document)
            .and_then(|doc| doc.directory_metadata_mut())
            .and_then(|source| source.draft.as_mut())
        {
            draft.hint = complete_last.then_some(Hint {
                revision,
                at: range.start.get(),
                removed: range.len(),
                inserted: 0,
                delete_last_row: true,
                ambiguous: false,
                paste: None,
            });
        }
    }
    pub(crate) fn filename_change_hint(&mut self, range: strop_core::Range) {
        let revision = self.buf().revision();
        let document = self.current();
        if let Some(draft) = self
            .docs
            .get_mut(document)
            .and_then(|doc| doc.directory_metadata_mut())
            .and_then(|source| source.draft.as_mut())
        {
            draft.hint = Some(Hint {
                revision,
                at: range.start.get(),
                removed: range.len(),
                inserted: 0,
                delete_last_row: false,
                ambiguous: true,
                paste: None,
            });
        }
    }
    pub(crate) fn filename_paste_hint(
        &mut self,
        at: usize,
        text: &str,
        prefix: usize,
        register: Option<&Arc<FileRegister>>,
        count: usize,
    ) {
        let revision = self.buf().revision();
        let document = self.current();
        if let Some(draft) = self
            .docs
            .get_mut(document)
            .and_then(|doc| doc.directory_metadata_mut())
            .and_then(|source| source.draft.as_mut())
        {
            draft.hint = register.map(|register| Hint {
                revision,
                at,
                removed: 0,
                inserted: text.len(),
                delete_last_row: false,
                ambiguous: false,
                paste: Some((prefix, register.clone(), count)),
            });
        }
    }
    pub(crate) fn clear_filename_hint(&mut self) {
        let document = self.current();
        if let Some(draft) = self
            .docs
            .get_mut(document)
            .and_then(|doc| doc.directory_metadata_mut())
            .and_then(|source| source.draft.as_mut())
        {
            draft.hint = None;
        }
    }
}
