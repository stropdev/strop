//! Directory operands share the existing completion ticket and prompt reducer.
use super::*;
use strop_workspace::{Filesystem, ResourceLocation};
const CANDIDATE_LIMIT: usize = 1024;

pub(super) fn classify(
    editor: &Editor,
    operand: &str,
    context: ResourceLocation,
) -> Result<RemoteCompletionQuery, String> {
    let (directory, segment) = if operand.is_empty() || operand.ends_with('/') {
        (operand, "")
    } else {
        operand
            .rfind('/')
            .map_or(("", operand), |at| (&operand[..=at], &operand[at + 1..]))
    };
    let target = Editor::directory_operand_from(context, directory)?;
    let location = target
        .resource_location()
        .ok_or("completion requires a filesystem namespace")?;
    let bytes = if operand.starts_with("file://") || operand.starts_with("container:") {
        lenient_percent_decode(segment)
    } else {
        segment.as_bytes().to_vec()
    };
    if let Filesystem::Remote(endpoint) = &location.filesystem {
        let file = RemoteFile::from_path(
            endpoint.clone(),
            location.path.join(
                strop_workspace::addr::uri::bytes_to_path(bytes)
                    .map_err(|error| error.to_string())?,
            ),
        )
        .map_err(|error| error.to_string())?;
        let mut uri = file.to_string();
        if segment.is_empty() && !uri.ends_with('/') {
            uri.push('/');
        }
        return classify_remote_operand(
            uri.strip_prefix("ssh://")
                .ok_or("invalid remote completion identity")?,
        );
    }
    let container = match &location.filesystem {
        Filesystem::Container(id) => editor.containers.attached.get(id.as_str()).cloned(),
        _ => None,
    };
    Ok(RemoteCompletionQuery::Directory {
        location,
        segment: bytes,
        container,
    })
}

/// All namespace authorities captured at one prompt moment. A
/// container lease is paired with its inspected incarnation; missing
/// authority stays on the established read-only tar/SFTP path.
pub(super) struct Sources<'a> {
    pub client: &'a RemoteClient,
    pub worker: &'a strop_worker_client::Worker,
    pub remote: &'a crate::editor::remote::workers::RemoteWorkers,
    pub container: Option<&'a strop_containers::ContainerIdentity>,
    pub container_worker: Option<&'a crate::editor::containers::BoundWorker>,
}

pub(super) fn run(
    location: ResourceLocation,
    segment: &[u8],
    sources: Sources<'_>,
    cancel: &worker::CancelToken,
) -> Outcome<RemoteCompletionResult> {
    // Completion never authenticates: a bound lease is reused, or the
    // read-only SFTP/tar path serves without provisioning.
    let listed = match crate::editor::namespace::list(
        sources.worker,
        sources.remote,
        &location,
        sources.client,
        sources.container,
        sources.container_worker,
        cancel,
    ) {
        Ok(listed) => listed,
        Err(error) if error.kind == strop_workspace::operation::FsFailureKind::Cancelled => {
            return Outcome::Cancelled(CancelReason::OwnerClosed)
        }
        Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
    };
    let mut items = Vec::new();
    let mut limited = !listed.directory.snapshot.state.is_complete();
    for entry in listed.directory.snapshot.entries.iter() {
        if cancel.is_cancelled() {
            return Outcome::Cancelled(CancelReason::OwnerClosed);
        }
        if !strop_workspace::addr::uri::path_bytes(entry.name.as_path()).starts_with(segment) {
            continue;
        }
        if items.len() == CANDIDATE_LIMIT {
            limited = true;
            break;
        }
        let directory = entry.observation.kind == strop_workspace::EntryKind::Directory;
        let mut uri = match listed.directory.snapshot.location_of(entry).uri() {
            Ok(uri) => uri,
            Err(error) => return Outcome::failed(FailureKind::Protocol, error.to_string()),
        };
        if directory && !uri.ends_with('/') {
            uri.push('/');
        }
        items.push(RemoteCandidate { uri, directory });
    }
    Outcome::Success(RemoteCompletionResult::Candidates {
        items,
        source: CandidateSource::Directory,
        notes: if limited {
            vec!["limited directory snapshot; refine the path prefix".into()]
        } else {
            Vec::new()
        },
        listed_directory: None,
    })
}
