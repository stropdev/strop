//! One admitted prompt moment's completion job: Directory, hosts or
//! already-owned SSH connection/cache. No I/O crosses the input path.

use super::*;

pub(super) fn run_completion(
    job: CompletionJob,
    cancel: worker::CancelToken,
) -> Outcome<RemoteCompletionResult> {
    let CompletionJob {
        query,
        directory,
        fallback,
        sources,
        history,
        client,
        worker,
        remote,
        container_worker,
    } = job;
    if cancel.is_cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    match query {
        RemoteCompletionQuery::Directory {
            location,
            segment,
            container,
        } => directory::run(
            location,
            &segment,
            directory::Sources {
                client: &client,
                worker: &worker,
                remote: &remote,
                container: container.as_ref(),
                container_worker: container_worker.as_deref(),
            },
            &cancel,
        ),
        RemoteCompletionQuery::Hosts { partial } => {
            let enumeration = strop_remote::enumerate_hosts(&sources, &history);
            let items = enumeration
                .complete(&partial)
                .into_iter()
                .map(|token| RemoteCandidate {
                    uri: format!("ssh://{token}"),
                    directory: false,
                })
                .collect();
            Outcome::Success(RemoteCompletionResult::Candidates {
                items,
                source: CandidateSource::Config,
                notes: enumeration.notes().to_vec(),
                listed_directory: None,
            })
        }
        RemoteCompletionQuery::Path { segment, .. } => {
            let Some(dir) = directory else {
                return Outcome::failed(
                    FailureKind::Protocol,
                    "path completion without a listing target",
                );
            };
            let prefix = lenient_percent_decode(&segment);
            match client.list_connected(&dir, &cancel) {
                Ok(entries) => {
                    let mut items: Vec<RemoteCandidate> = entries
                        .into_iter()
                        .filter_map(|entry| {
                            // `.`/`..` have no file_name; browsing owns
                            // parent navigation, completion owns names.
                            let name = entry.file.path().file_name()?;
                            if !name.as_encoded_bytes().starts_with(&prefix) {
                                return None;
                            }
                            let directory = matches!(entry.kind, RemoteEntryKind::Directory);
                            let mut uri = entry.file.to_string();
                            if directory && !uri.ends_with('/') {
                                uri.push('/');
                            }
                            Some(RemoteCandidate { uri, directory })
                        })
                        .collect();
                    items.sort_by(|a, b| {
                        b.directory
                            .cmp(&a.directory)
                            .then_with(|| a.uri.cmp(&b.uri))
                    });
                    let listed = dir.to_string();
                    Outcome::Success(RemoteCompletionResult::Candidates {
                        items,
                        source: CandidateSource::Connection,
                        notes: Vec::new(),
                        listed_directory: Some(listed),
                    })
                }
                Err(_not_connected) => {
                    if cancel.is_cancelled() {
                        return Outcome::Cancelled(CancelReason::OwnerClosed);
                    }
                    if let Some(cached) = fallback {
                        return Outcome::Success(RemoteCompletionResult::Candidates {
                            items: cached,
                            source: CandidateSource::Cache,
                            notes: Vec::new(),
                            listed_directory: None,
                        });
                    }
                    Outcome::Success(RemoteCompletionResult::ConnectRequired {
                        endpoint: endpoint_display(&dir),
                    })
                }
            }
        }
    }
}
