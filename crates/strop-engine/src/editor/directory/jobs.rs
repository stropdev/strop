use super::*;
impl Editor {
    pub(crate) fn start_directory_task(
        &mut self,
        document: DocumentId,
        task: DirectoryTask,
        query: Option<String>,
    ) -> Result<(), String> {
        let mut source = self
            .docs
            .get(document)
            .and_then(|document| document.directory_metadata_ref())
            .cloned()
            .ok_or("operation requires a Directory buffer")?;
        if task == DirectoryTask::EditNames && source.draft.is_some() {
            return Err("filename draft is already active".into());
        }
        if let Some(query) = query {
            source.filter = query;
        }
        self.cancel_directory_filter(document);
        let request = self.worker_ids.allocate().map_err(|error| error.message)?;
        let key = FilterKey {
            document,
            revision: self.doc(document).buf.revision(),
            location: source.location.clone(),
            query: source.filter.clone(),
            focus: self.focus_epoch,
            task,
            draft: source.draft.as_ref().map(|draft| draft.id),
        };
        let ticket = Ticket {
            request,
            key: key.clone(),
        };
        self.directories.filters.insert(request, key);
        match self.tape.request("directory.update", &ticket) {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(error) => {
                self.directory_filter_done(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return Ok(());
            }
        }
        let client = self.remote_client();
        let container = match &source.location.filesystem {
            Filesystem::Container(id) => self.containers.attached.get(id.as_str()).cloned(),
            _ => None,
        };
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "directory-update",
            move |outcome| {
                let _ = tx.send(IoEvent::DirectoryFilter(Box::new(Completion {
                    ticket,
                    outcome,
                })));
            },
            move |token| {
                let work = || -> Result<Opened, String> {
                    let rope = if task == DirectoryTask::EditNames {
                        let (draft, rope) =
                            super::super::filesystem::draft::Draft::new(request, &source)?;
                        source.draft = Some(draft);
                        source.summary = "filename draft · :w reviews · blank new rows are separators · visible backslash escapes".into();
                        rope
                    } else {
                        if task == DirectoryTask::Reload {
                            let listed = strop_fs::list(
                                &source.location,
                                &client,
                                container.as_ref(),
                                &token,
                            )
                            .map_err(|error| error.to_string())?;
                            let filter = std::mem::take(&mut source.filter);
                            source = Directory::from_listing(listed);
                            source.filter = filter;
                        }
                        apply_filter(&mut source, &token)?;
                        ropey::Rope::from_str(&source.text())
                    };
                    let canonical = FileTarget::from_location(&source.location)
                        .map_err(|error| error.to_string())?;
                    Ok(Opened {
                        document: super::super::Document::directory(
                            strop_core::Buffer::from_snapshot(rope),
                            source,
                        ),
                        canonical,
                    })
                };
                match work() {
                    Ok(opened) => Outcome::Success(opened),
                    Err(_) if token.is_cancelled() => Outcome::Cancelled(CancelReason::Superseded),
                    Err(error) => Outcome::failed(FailureKind::Io, error),
                }
            },
        );
        self.worker_handles.insert(request, handle);
        Ok(())
    }
}
