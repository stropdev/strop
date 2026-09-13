use super::*;

impl Editor {
    pub(crate) fn prepare_filesystem(
        &mut self,
        intents: Vec<OperationIntent>,
        recovery: Option<recovery::RecoveryGuard>,
    ) -> Result<(), String> {
        if self.filesystem.pending() {
            return Err("another filesystem request is still running".into());
        }
        if intents.is_empty() {
            return Err("no filesystem entries selected".into());
        }
        if intents.len() > strop_fs::batch::STEP_LIMIT {
            return Err("filesystem review is limited to 512 steps".into());
        }
        if recovery.is_some() && intents.len() != 1 {
            return Err("checked recovery names exactly one receipt step".into());
        }
        self.filesystem_admission(&intents, None)?;
        if self.filesystem.pending.is_some() {
            return Err("apply or cancel the current filesystem review first".into());
        }
        let mut copies = HashMap::new();
        for intent in &intents {
            if intent.kind == OperationKind::Copy && intent.copy_version == CopyVersion::Buffer {
                let location = intent
                    .source
                    .as_ref()
                    .ok_or("buffer copy requires a source")?;
                let target = crate::files::FileTarget::from_location(location)
                    .map_err(|error| error.to_string())?;
                let source = self
                    .docs
                    .iter()
                    .find_map(|(_, document)| document.matches_target(&target).then_some(document))
                    .ok_or("copy current buffer requires an open source document")?;
                copies.insert(location.clone(), source.buf.snapshot());
            }
        }
        let intents = std::sync::Arc::new(intents);
        let key = FsKey {
            origin: self.current(),
            revision: self.buf().revision(),
            focus: self.focus_epoch,
            open_created: intents.len() == 1
                && matches!(
                    intents[0].kind,
                    OperationKind::CreateFile | OperationKind::CreateDirectory
                ),
            intents: intents.clone(),
            recovery: recovery.clone(),
            draft: None,
        };
        let environment = self.filesystem.environment.clone();
        self.start_filesystem_preparation(key, copies, move |token| match strop_fs::batch::prepare(
            &intents,
            &environment,
            &token,
        ) {
            Ok(batch)
                if recovery
                    .as_ref()
                    .is_some_and(|guard| batch.refused.is_empty() && !guard.accepts(&batch)) =>
            {
                Outcome::failed(
                    FailureKind::InvalidInput,
                    "recovery object changed since its receipt; no mutation admitted",
                )
            }
            Ok(batch) => Outcome::Success(PreparedFilesystem {
                batch,
                draft_targets: Vec::new(),
            }),
            Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
        })
    }

    pub(super) fn start_filesystem_preparation(
        &mut self,
        key: FsKey,
        copies: HashMap<ResourceLocation, ropey::Rope>,
        work: impl FnOnce(worker::CancelToken) -> Outcome<PreparedFilesystem> + Send + 'static,
    ) -> Result<(), String> {
        if self.filesystem.pending() || self.filesystem.pending.is_some() {
            return Err("finish the current filesystem request or review first".into());
        }
        let request = self.worker_ids.allocate().map_err(|error| error.message)?;
        let ticket = Ticket { request, key };
        self.filesystem.preparing = Some(ticket.clone());
        self.filesystem.preparing_copies = copies;
        self.message = "preparing filesystem review; nothing applied".into();
        let launch = self.tape.request("filesystem.prepare", &ticket);
        match launch {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(error) => {
                self.handle_filesystem(FsEvent::Prepared(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                })));
                return Ok(());
            }
        }
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "strop-fs-prepare",
            move |outcome| {
                let _ = tx.send(IoEvent::Filesystem(Box::new(FsEvent::Prepared(Box::new(
                    Completion { ticket, outcome },
                )))));
            },
            work,
        );
        self.worker_handles.insert(request, handle);
        Ok(())
    }
}
