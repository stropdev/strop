use super::*;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

pub(super) fn copy(
    operation: &PreparedOperation,
    source: &LocatedObservation,
    destination: &LocatedObservation,
    source_parent: Option<&Parent>,
    destination_parent: &Parent,
    contents: Option<&ropey::Rope>,
    token: &CancelToken,
) -> Result<StepOutcome, FsFailure> {
    use rustix::fs::{Mode, OFlags};
    let mut stage = stage::Stage::create(destination_parent)?;
    let mut attempted = false;
    let mut published = false;
    let mut publication = None;
    let mut public_file = None;
    let work = (|| -> Result<(), FsFailure> {
        let mut original = if let Some(parent) = source_parent {
            Some(
                rustix::fs::openat(
                    &parent.file,
                    source.location.path.file_name().ok_or_else(|| {
                        failure(FsFailureKind::InvalidPath, "copy source has no basename")
                    })?,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map(File::from)
                .map_err(|error| io_failure(error.into()))?,
            )
        } else {
            None
        };
        let metadata = original
            .as_ref()
            .map(|file| file.metadata().map_err(io_failure))
            .transpose()?;
        if let Some(metadata) = &metadata {
            if !source
                .value
                .as_ref()
                .is_some_and(|before| before.same_metadata(&observation::metadata(metadata)))
            {
                return Err(failure(
                    FsFailureKind::Conflict,
                    "copy source descriptor differs from the prepared version",
                ));
            }
        }
        let attributes = original.as_ref().map(crate::attributes::read).transpose()?;
        let mut digest = Sha256::new();
        if let Some(contents) = contents {
            for chunk in contents.chunks() {
                if token.is_cancelled() {
                    return Err(failure(
                        FsFailureKind::Cancelled,
                        "copy cancelled before publication",
                    ));
                }
                stage.file.write_all(chunk.as_bytes()).map_err(io_failure)?;
                digest.update(chunk.as_bytes());
            }
        } else {
            let original = original
                .as_mut()
                .ok_or_else(|| failure(FsFailureKind::Conflict, "stored copy source is missing"))?;
            let mut chunk = [0_u8; 64 * 1024];
            loop {
                if token.is_cancelled() {
                    return Err(failure(
                        FsFailureKind::Cancelled,
                        "copy cancelled before publication",
                    ));
                }
                let count = original.read(&mut chunk).map_err(io_failure)?;
                if count == 0 {
                    break;
                }
                stage.file.write_all(&chunk[..count]).map_err(io_failure)?;
                digest.update(&chunk[..count]);
            }
        }
        let digest: [u8; 32] = digest.finalize().into();
        if contents.is_none()
            && source.value.as_ref().and_then(|value| value.digest) != Some(digest)
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "streamed copy bytes differ from the prepared source digest",
            ));
        }
        if let (Some(metadata), Some(attributes)) = (&metadata, &attributes) {
            crate::attributes::apply(&stage.file, metadata, attributes, contents.is_none())?;
        }
        stage.file.sync_all().map_err(io_failure)?;
        if observation::observe(
            &source.location.path,
            source
                .value
                .as_ref()
                .is_some_and(|value| value.digest.is_some()),
            token,
        )? != source.value
        {
            return Err(failure(
                FsFailureKind::Conflict,
                "copy source changed during streaming",
            ));
        }
        if let Some(parent) = source_parent {
            parent.revalidate()?;
        }
        destination_parent.revalidate()?;
        if token.is_cancelled() {
            return Err(failure(
                FsFailureKind::Cancelled,
                "copy cancelled before publication",
            ));
        }
        publication = Some(witness(&stage.file, Some(digest))?);
        let name = destination.location.path.file_name().ok_or_else(|| {
            failure(
                FsFailureKind::InvalidPath,
                "copy destination has no basename",
            )
        })?;
        attempted = true;
        #[cfg(target_os = "linux")]
        {
            rustix::fs::linkat(
                &stage.directory,
                "contents",
                &destination_parent.file,
                name,
                rustix::fs::AtFlags::empty(),
            )
            .map_err(|error| io_failure(error.into()))?;
            published = true;
            // A public-path O_PATH handle needs no read permission and keeps the
            // inode alive while the private NFS dentry is closed and unlinked.
            let file = rustix::fs::openat(
                &destination_parent.file,
                name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map(File::from)
            .map_err(|error| io_failure(error.into()))?;
            let observed = witness(&file, Some(digest))?;
            if publication.is_none_or(|owned| owned.identity != observed.identity) {
                return Err(failure(
                    FsFailureKind::Conflict,
                    "copy destination was replaced before acknowledgement",
                ));
            }
            public_file = Some(file);
        }
        #[cfg(target_os = "macos")]
        {
            rustix::fs::renameat_with(
                &stage.directory,
                "contents",
                &destination_parent.file,
                name,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(rename_error)?;
            published = true;
            public_file = Some(stage.file.try_clone().map_err(io_failure)?);
        }
        destination_parent.file.sync_all().map_err(io_failure)?;
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
                    ResourceLocation::local(destination_parent.path.join(&stage.name)).label()
                ),
                publication,
            ));
        }
    }
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
    let result = match work {
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
    drop(public_file);
    result
}
