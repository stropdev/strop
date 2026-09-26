//! Protected document save (0058 WK09): frozen content atomically
//! replaces (or creates) a checked destination, preserving metadata.
//! The stage is private and descriptor-pinned; syncs bracket publication
//! and uncertain outcomes retain the attempt's verification evidence.

use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;

pub(super) fn store(
    operation: &PreparedOperation,
    contents: Option<&ropey::Rope>,
    destination: &LocatedObservation,
    parent: &Parent,
    token: &CancelToken,
) -> Result<StepOutcome, FsFailure> {
    use rustix::fs::{Mode, OFlags};
    let policy = operation.intent.store.ok_or_else(|| {
        failure(
            FsFailureKind::Protocol,
            "store step carries no conditional policy",
        )
    })?;
    let intended = operation.intent.expected_content.ok_or_else(|| {
        failure(
            FsFailureKind::Protocol,
            "store step carries no intended content digest",
        )
    })?;
    let contents = contents.ok_or_else(|| {
        failure(
            FsFailureKind::Protocol,
            "store requires its frozen content stream",
        )
    })?;
    let name = destination.location.path.file_name().ok_or_else(|| {
        failure(
            FsFailureKind::InvalidPath,
            "store destination has no basename",
        )
    })?;
    // The current destination, pinned by descriptor when present: the
    // baseline check and the preserved metadata come from this open,
    // never from a second path walk.
    let mut current = match rustix::fs::openat(
        &parent.file,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => Some(File::from(file)),
        Err(error) if error == rustix::io::Errno::NOENT => None,
        Err(error) => return Err(io_failure(error.into())),
    };
    let before = current
        .as_ref()
        .map(|file| file.metadata().map_err(io_failure))
        .transpose()?;
    let observed = before.as_ref().map(observation::metadata);
    if let Some(expected) = operation
        .destination
        .as_ref()
        .and_then(|destination| destination.value.as_ref())
    {
        if observed
            .as_ref()
            .is_none_or(|actual| !expected.same_metadata(actual))
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "stored destination changed after preparation",
            ));
        }
    }
    if policy.baseline_object.is_some()
        && observed.as_ref().and_then(|value| value.identity) != policy.baseline_object
    {
        return Err(failure(
            FsFailureKind::Conflict,
            "stored file identity changed since edit admission",
        ));
    }
    if let Some(observed) = &observed {
        if observed.kind != EntryKind::File {
            return Err(failure(
                FsFailureKind::Unsupported,
                "store replaces only regular files",
            ));
        }
        if observed.links != Some(1) {
            return Err(failure(
                FsFailureKind::Unsupported,
                "hard-linked file mutation requires explicit alias handling",
            ));
        }
    }
    // The conflict contract at effect time (the local `:w` semantics): the
    // baseline is the editor's frozen evidence, never re-observed.
    if !policy.force {
        if policy.expect_absent {
            if current.is_some() {
                return Err(failure(
                    FsFailureKind::Conflict,
                    "file exists — :w! to overwrite",
                ));
            }
        } else if observed.as_ref().and_then(|value| value.modified) != policy.baseline {
            return Err(failure(
                FsFailureKind::Conflict,
                "file changed on disk — :w! to force",
            ));
        }
    }
    if let Some(displayed) = policy.displayed {
        let file = current
            .as_mut()
            .ok_or_else(|| failure(FsFailureKind::Conflict, "stored baseline file disappeared"))?;
        let actual = observation::digest_descriptor(file, token)?;
        if actual != displayed
            || observed.as_ref()
                != Some(&observation::metadata(
                    &file.metadata().map_err(io_failure)?,
                ))
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "stored baseline content or metadata changed before save",
            ));
        }
    }
    let attributes = current.as_ref().map(crate::attributes::read).transpose()?;
    let attributes_digest = attributes
        .as_ref()
        .map(|attributes| crate::attributes::fingerprint(attributes));
    if let Some(expected) = operation
        .destination
        .as_ref()
        .and_then(|destination| destination.value.as_ref())
        .and_then(|value| value.attributes)
    {
        if attributes_digest != Some(expected) {
            return Err(failure(
                FsFailureKind::Conflict,
                "destination extended attributes changed before save",
            ));
        }
    }
    if policy.baseline_attributes.is_some() && attributes_digest != policy.baseline_attributes {
        return Err(failure(
            FsFailureKind::Conflict,
            "stored file attributes changed since edit admission",
        ));
    }
    let mut stage = stage::Stage::create(parent)?;
    let mut attempted = false;
    let mut published = false;
    let mut publication = None;
    let mut public_file = None;
    let work = (|| -> Result<(), FsFailure> {
        let mut digest = Sha256::new();
        for chunk in contents.chunks() {
            if token.is_cancelled() {
                return Err(failure(
                    FsFailureKind::Cancelled,
                    "save cancelled before publication",
                ));
            }
            stage.file.write_all(chunk.as_bytes()).map_err(io_failure)?;
            digest.update(chunk.as_bytes());
        }
        let digest: [u8; 32] = digest.finalize().into();
        if digest != intended {
            return Err(failure(
                FsFailureKind::Protocol,
                "stored content digest differs from the admitted digest",
            ));
        }
        #[cfg(test)]
        test_support::check(test_support::Fault::Metadata, &destination.location.path)?;
        if let (Some(metadata), Some(attributes)) = (&before, &attributes) {
            crate::attributes::restore(&stage.file, metadata, attributes)?;
        }
        stage.file.sync_all().map_err(io_failure)?;
        stage.directory.sync_all().map_err(io_failure)?;
        #[cfg(test)]
        test_support::check(
            test_support::Fault::ExternalWrite,
            &destination.location.path,
        )?;
        // The pinned destination must still be the checked object. An
        // absent baseline is guarded atomically by the publication itself
        // (no-replace create) or by the overwrite's local `:w` parity.
        if !policy.force && observed.is_some() {
            let now = current
                .as_ref()
                .map(|file| file.metadata().map_err(io_failure))
                .transpose()?;
            if now.as_ref().map(observation::metadata) != observed {
                return Err(failure(
                    FsFailureKind::Conflict,
                    "destination changed during stage preparation",
                ));
            }
        }
        parent.revalidate()?;
        if token.is_cancelled() {
            return Err(failure(
                FsFailureKind::Cancelled,
                "save cancelled before publication",
            ));
        }
        publication = Some(witness(&stage.file, Some(digest))?);
        // Set before the syscall: a failure after publication must never
        // be misclassified as a proven pre-commit refusal.
        attempted = true;
        #[cfg(test)]
        test_support::check(
            test_support::Fault::BeforeRename,
            &destination.location.path,
        )?;
        let replace = policy.force || !policy.expect_absent;
        #[cfg(target_os = "linux")]
        {
            if replace {
                rustix::fs::renameat(&stage.directory, "contents", &parent.file, name)
                    .map_err(|error| io_failure(error.into()))?;
            } else {
                rustix::fs::linkat(
                    &stage.directory,
                    "contents",
                    &parent.file,
                    name,
                    rustix::fs::AtFlags::empty(),
                )
                .map_err(|error| io_failure(error.into()))?;
            }
            published = true;
            // A public-path O_PATH handle needs no read permission and
            // pins the published object while the private stage closes.
            let file = rustix::fs::openat(
                &parent.file,
                name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map(File::from)
            .map_err(|error| io_failure(error.into()))?;
            let observed_publication = witness(&file, Some(digest))?;
            if publication.is_none_or(|owned| owned.identity != observed_publication.identity) {
                return Err(failure(
                    FsFailureKind::Conflict,
                    "store destination was replaced before acknowledgement",
                ));
            }
            publication = Some(observed_publication);
            public_file = Some(file);
        }
        #[cfg(target_os = "macos")]
        {
            if replace {
                rustix::fs::renameat(&stage.directory, "contents", &parent.file, name)
                    .map_err(|error| io_failure(error.into()))?;
            } else {
                rustix::fs::renameat_with(
                    &stage.directory,
                    "contents",
                    &parent.file,
                    name,
                    rustix::fs::RenameFlags::NOREPLACE,
                )
                .map_err(rename_error)?;
            }
            published = true;
            public_file = Some(stage.file.try_clone().map_err(io_failure)?);
        }
        #[cfg(test)]
        test_support::check(
            test_support::Fault::BeforeDirectorySync,
            &destination.location.path,
        )?;
        parent.file.sync_all().map_err(io_failure)?;
        Ok(())
    })();
    if let Err(error) = &work {
        if public_file.is_none() && (published || (attempted && error.kind == FsFailureKind::Io)) {
            if let Some(owned) = publication {
                publication = witness(&stage.file, owned.content).ok();
            }
            return Ok(uncertain(
                operation,
                format!(
                    "{error}; private bookkeeping retained at {}",
                    ResourceLocation::local(parent.path.join(&stage.name)).label()
                ),
                publication,
            ));
        }
    }
    let attributes_checked = if published {
        match attributes_digest {
            Some(expected) => match crate::attributes::digest(&stage.file) {
                Ok(actual) if actual == expected => Ok(()),
                Ok(_) => Err(failure(
                    FsFailureKind::Conflict,
                    "published extended attributes changed before acknowledgment",
                )),
                Err(error) => Err(error),
            },
            None => Ok(()),
        }
    } else {
        Ok(())
    };
    let cleanup = stage.cleanup();
    if let Some(file) = &public_file {
        publication = match witness(file, publication.and_then(|value| value.content)) {
            Ok(value) => Some(value),
            Err(error) => {
                return Ok(uncertain(
                    operation,
                    format!("{error}; cleanup: {cleanup:?}"),
                    None,
                ))
            }
        };
    }
    if let Err(error) = attributes_checked {
        return Ok(uncertain(operation, error.to_string(), publication));
    }
    let mut result = match work {
        Ok(()) => committed(
            operation,
            cleanup
                .err()
                .map(|error| error.to_string())
                .into_iter()
                .collect(),
            publication,
        ),
        Err(error) if published => Ok(uncertain(
            operation,
            format!("{error}; cleanup: {cleanup:?}"),
            publication,
        )),
        Err(error) => Err(failure(
            error.kind,
            format!(
                "{}{}",
                error.detail,
                cleanup
                    .err()
                    .map(|error| format!("; {error}"))
                    .unwrap_or_default()
            ),
        )),
    };
    if let Ok(StepOutcome::Committed {
        destination_after: Some(after),
        ..
    }) = &mut result
    {
        after.attributes = attributes_digest;
    }
    drop(public_file);
    result
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{failure, FsFailure, FsFailureKind};
    use std::cell::Cell;
    use std::path::Path;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Fault {
        Metadata,
        ExternalWrite,
        BeforeRename,
        BeforeDirectorySync,
    }

    thread_local! {
        static ACTIVE: Cell<Option<Fault>> = const { Cell::new(None) };
    }

    struct Restore(Option<Fault>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|active| active.set(self.0));
        }
    }

    /// Scoped to this test thread, including panic paths; no production
    /// build includes the injector or a fake syscall implementation.
    pub fn with_fault<T>(point: Fault, work: impl FnOnce() -> T) -> T {
        let old = ACTIVE.with(|active| active.replace(Some(point)));
        let _restore = Restore(old);
        work()
    }

    pub(super) fn check(point: Fault, path: &Path) -> Result<(), FsFailure> {
        let active = ACTIVE.with(|active| {
            if active.get() == Some(point) {
                active.set(None);
                true
            } else {
                false
            }
        });
        if !active {
            return Ok(());
        }
        if point == Fault::ExternalWrite {
            std::fs::write(path, b"third-party\n").map_err(super::super::io_failure)
        } else {
            Err(failure(
                FsFailureKind::Io,
                "injected native Store syscall failure",
            ))
        }
    }
}
