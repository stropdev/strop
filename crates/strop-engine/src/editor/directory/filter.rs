use super::*;
use strop_picker::query::{ContentPlan, FileSelectionPlan, SearchQuery};

/// Worker-only semantic compilation/projection; opening and filtering use one policy.
pub(crate) fn apply_filter(
    directory: &mut Directory,
    token: &strop_core::worker::CancelToken,
) -> Result<(), String> {
    let query = SearchQuery::parse(&directory.filter);
    let selection = FileSelectionPlan::compile(&query).map_err(|error| error.message)?;
    if selection.ignored == Some(false) {
        return Err("Directory snapshots include ignored entries and do not evaluate ignore rules; use Search or an explicit path/glob filter".into());
    }
    let content = ContentPlan::compile(&query).map_err(|error| error.message)?;
    directory.summary = format!(
        "folder · {} · hidden {} · no ignores",
        if matches!(
            query.content,
            Some(strop_picker::query::ContentExpr::Regex(_))
        ) {
            "regex"
        } else {
            "literal"
        },
        if selection.hidden == Some(false) {
            "off"
        } else {
            "on"
        }
    );
    let mut visible = Vec::new();
    for (index, entry) in directory.entries.iter().enumerate() {
        if token.is_cancelled() {
            return Err("directory filter cancelled".into());
        }
        if selection.hidden == Some(false)
            && entry
                .name
                .as_path()
                .as_os_str()
                .as_encoded_bytes()
                .starts_with(b".")
        {
            continue;
        }
        let name = entry
            .name
            .as_path()
            .to_str()
            .map(std::borrow::Cow::Borrowed)
            .unwrap_or_else(|| std::borrow::Cow::Owned(entry.name.display()));
        if selection.allows(&name)
            && content
                .as_ref()
                .is_none_or(|plan| plan.regex.is_match(&name))
        {
            visible.push(index);
        }
    }
    directory.visible = visible.into();
    Ok(())
}

impl Editor {
    pub(crate) fn filter_directory(&mut self, query: String) -> Result<(), String> {
        let source = self
            .directory()
            .ok_or(":filter requires a Directory buffer")?;
        if source.draft.is_some()
            || !self.buf().readonly
            || source.view_revision != self.buf().revision()
        {
            return Err("finish the filename draft or refresh before filtering".into());
        }
        self.start_directory_task(self.current(), DirectoryTask::Filter, Some(query))
    }
    pub(crate) fn cancel_directory_filter(&mut self, document: DocumentId) {
        let requests: Vec<_> = self
            .directories
            .filters
            .iter()
            .filter_map(|(&request, key)| (key.document == document).then_some(request))
            .collect();
        for request in requests {
            self.directories.filters.remove(&request);
            if let Some(handle) = self.worker_handles.remove(&request) {
                handle.cancel(CancelReason::Superseded);
            }
        }
    }
    pub(crate) fn directory_filter_done(&mut self, completion: Completion<FilterKey, Opened>) {
        let request = completion.ticket.request;
        if self.directories.filters.get(&request) != Some(&completion.ticket.key) {
            return;
        }
        self.directories.filters.remove(&request);
        self.worker_handles.remove(&request);
        let key = completion.ticket.key;
        if self.finishing
            || self.docs.get(key.document).is_none_or(|doc| {
                let source = doc.directory_metadata_ref();
                let draft_reload = key.task == DirectoryTask::Reload
                    && key.draft.is_some()
                    && source
                        .and_then(|source| source.draft.as_ref())
                        .is_some_and(|draft| Some(draft.id) == key.draft);
                (!draft_reload && doc.buf.revision() != key.revision)
                    || source.is_none_or(|source| source.location != key.location)
            })
        {
            return;
        }
        let focused = self.current() == key.document && self.focus_epoch == key.focus;
        match completion.outcome {
            Outcome::Success(mut opened) => {
                let retained = self
                    .doc(key.document)
                    .directory_metadata_ref()
                    .and_then(|source| source.draft.as_ref())
                    .is_some_and(|draft| {
                        !matches!(
                            draft.phase,
                            super::super::filesystem::draft::Phase::Reloading
                        )
                    });
                if key.task == DirectoryTask::Reload && !retained {
                    if let Some(doc) = self.docs.get_mut(key.document) {
                        doc.buf.dirty = false;
                    }
                }
                let marks = self
                    .doc(key.document)
                    .directory_metadata_ref()
                    .map(|source| source.marked.clone());
                if let (Some(source), Some(marks)) =
                    (opened.document.directory_metadata_mut(), marks)
                {
                    source.marked = marks;
                }
                match self.publish_source_snapshot(key.document, opened.document, false) {
                    Err(error) if focused => self.message = error.to_string(),
                    Ok(()) if retained && focused => {
                        self.message =
                            "filename draft retained; refreshed source observations are available"
                                .into();
                    }
                    _ => {}
                }
            }
            Outcome::Failed { failure, .. } => {
                if let Some(source) = self.doc_mut(key.document).directory_metadata_mut() {
                    source.stale = Some(failure.message.clone());
                }
                if focused {
                    self.message = failure.message;
                }
            }
            Outcome::Cancelled(_) => {}
        }
    }
}
