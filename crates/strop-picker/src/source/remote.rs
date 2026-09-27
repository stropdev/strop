//! Read-only SSH search: select native paths first, then search bounded argv batches.
use super::{
    flow::StreamSender,
    query::{parse_json_match_with, scoped_path, RECORD_LIMIT},
    PickerMsg, SelectionPolicy, SourceSnapshot,
};
use crate::query::{CaseMode, ContentPlan, Evidence, FileSelectionPlan, SearchQuery};
use std::io::Read;
use std::{ffi::OsString, path::PathBuf, sync::Arc};
use strop_core::worker::{CancelReason, CancelToken, FailureKind, Outcome};
use strop_remote::worker_transport::RemoteWorker;
use strop_workspace::{RemoteEndpoint, ResourceLocation};

pub(super) fn run_search(
    query: Arc<SearchQuery>,
    policy: SelectionPolicy,
    root: ResourceLocation,
    snapshots: Vec<SourceSnapshot>,
    lease: Option<RemoteWorker>,
    tx: StreamSender,
    token: CancelToken,
) -> Outcome<()> {
    let Some(worker) = lease else {
        return Outcome::failed(
            FailureKind::Unavailable,
            "SSH search requires an admitted worker; SFTP-only hosts remain read-only",
        );
    };
    let strop_workspace::Filesystem::Remote(endpoint) = &root.filesystem else {
        return Outcome::failed(FailureKind::Protocol, "SSH search requires an SSH scope");
    };
    if worker.endpoint() != endpoint {
        return Outcome::failed(
            FailureKind::Protocol,
            format!("SSH search scope {endpoint} does not match the admitted worker"),
        );
    }
    let result = search(&query, policy, &root, snapshots, &worker, &tx, &token);
    if token.is_cancelled() {
        return Outcome::Cancelled(CancelReason::Superseded);
    }
    match result {
        Ok(()) => Outcome::Success(()),
        Err(error) => Outcome::failed(FailureKind::Io, error),
    }
}

fn search(
    query: &SearchQuery,
    policy: SelectionPolicy,
    root: &ResourceLocation,
    snapshots: Vec<SourceSnapshot>,
    worker: &RemoteWorker,
    tx: &StreamSender,
    token: &CancelToken,
) -> Result<(), String> {
    let endpoint = worker.endpoint();
    let selection = FileSelectionPlan::compile(query).map_err(|error| error.message)?;
    let content = ContentPlan::compile(query)
        .map_err(|error| error.message)?
        .ok_or("content search needs an expression")?;
    let effective = policy.effective(&selection);
    let mut common: Vec<OsString> = vec!["--no-config".into()];
    if effective.hidden {
        common.push("--hidden".into());
    }
    if !effective.respect_ignore {
        common.push("--no-ignore".into());
    }
    common.extend(super::RG_PROTECTED_ARGS.iter().map(OsString::from));
    let mut arguments = common.clone();
    arguments.extend(["--files".into(), "--null".into(), "--".into(), ".".into()]);
    let mut records = Records::new(0);
    let mut paths = Vec::new();
    let mut bytes = 0usize;
    run(endpoint, worker, root, arguments, token, tx, |chunk| {
        records.feed(chunk, |record| {
            let location = scoped_path(root, record.to_vec())?;
            let relative = location
                .path
                .strip_prefix(&root.path)
                .map_err(|_| "SSH file escaped scope")?;
            if !selection.allows(&super::display_path(relative)) {
                return Ok(());
            }
            bytes = bytes.saturating_add(relative.as_os_str().as_encoded_bytes().len());
            if paths.len() >= super::selection::PATH_LIMIT || bytes > super::selection::PATH_BYTES {
                return Err(
                    "SSH search file selection exceeds 100000 paths or 16 MiB; narrow the scope"
                        .into(),
                );
            }
            paths.push(relative.to_path_buf());
            Ok(())
        })
    })?;
    records.finish()?;
    // Remote project discovery arrives with 0058's unified worker;
    // until then `repo:` atoms admit (overfetch), never drop.
    // Remote symbol evidence arrives with 0058's worker; until then
    super::snapshots::emit_snapshots(
        root,
        &content,
        snapshots,
        &mut paths,
        tx,
        token,
        super::snapshots::Sources {
            catalog: None,
            symbols: None,
        },
    )?;
    let pattern: std::ffi::OsString = content.provider_pattern().into();
    common.extend(["--json".into(), "-e".into(), pattern]);
    common.push(
        match content.case {
            CaseMode::Smart => "--smart-case",
            CaseMode::Sensitive => "--case-sensitive",
            CaseMode::Ignore => "--ignore-case",
        }
        .into(),
    );
    if content.is_literal() {
        common.push("--fixed-strings".into());
    }
    common.push("--".into());
    let base = common
        .iter()
        .map(|arg| arg.as_encoded_bytes().len() + 1)
        .sum::<usize>();
    let mut paths = paths.into_iter().peekable();
    while paths.peek().is_some() {
        if token.is_cancelled() {
            return Err("SSH search cancelled".into());
        }
        let mut args = common.clone();
        let mut bytes = base;
        let mut count = 0;
        while let Some(path) = paths.peek() {
            let size = path.as_os_str().as_encoded_bytes().len() + 3;
            if count > 0 && (count == 1024 || bytes + size > 64 * 1024) {
                break;
            }
            if bytes + size > 64 * 1024 {
                return Err("SSH search path or expression exceeds the argv bound".into());
            }
            bytes += size;
            count += 1;
            if let Some(path) = paths.next() {
                args.push(PathBuf::from(".").join(path).into_os_string());
            }
        }
        let mut records = Records::new(b'\n');
        let admits = {
            let content = content.clone();
            move |path: &str, line: Option<usize>, text: &str| {
                content.admits(
                    Evidence {
                        path: Some(path),
                        line,
                        ..Evidence::default()
                    },
                    text,
                )
            }
        };
        run(endpoint, worker, root, args, token, tx, |chunk| {
            records.feed(chunk, |record| {
                let items = parse_json_match_with(record, root, Some(&admits))?;
                tx.batch(items, token)
                    .map_err(|error| error.message().to_string())
            })
        })?;
        records.finish()?;
    }
    Ok(())
}

fn run(
    endpoint: &RemoteEndpoint,
    worker: &RemoteWorker,
    root: &ResourceLocation,
    args: Vec<OsString>,
    token: &CancelToken,
    tx: &StreamSender,
    mut consume: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let spec = strop_worker_protocol::ExecSpec {
        program: b"rg".to_vec(),
        argv: args
            .iter()
            .map(|arg| arg.as_encoded_bytes().to_vec())
            .collect(),
        cwd: strop_workspace::addr::uri::path_bytes(&root.path).to_vec(),
        env: Vec::new(),
        service: false,
        pty: None,
    };
    let handle = worker
        .worker()
        .exec(token, spec)
        .map_err(|error| format!("SSH search on {endpoint}: {error}"))?;
    let (exec, stdin, mut stdout, stderr, exit) = handle.into_parts();
    let control = stdin.control(worker.worker().clone(), exec);
    // Both pipes drain concurrently: stderr cannot stall a large rg
    // stdout stream, and noisy diagnostics stay bounded.
    let stderr_pump = std::thread::spawn(move || capture_stderr(stderr));
    let mut buffer = [0u8; 32 * 1024];
    let consumed: Result<(), String> = (|| loop {
        let count = stdout
            .read(&mut buffer)
            .map_err(|error| format!("SSH rg output on {endpoint}: {error}"))?;
        if count == 0 {
            return Ok(());
        }
        consume(&buffer[..count])?;
    })();
    if consumed.is_err() {
        let (cancel, _owner) = CancelToken::standalone();
        let _ = control.cancel(&cancel);
    }
    drop(stdout);
    let stderr_result = stderr_pump
        .join()
        .map_err(|_| "SSH rg stderr reader panicked".to_owned())?;
    let status = exit.wait();
    consumed?;
    let (stderr_bytes, dropped) =
        stderr_result.map_err(|error| format!("SSH rg stderr on {endpoint}: {error}"))?;
    let mut stderr = String::from_utf8_lossy(&stderr_bytes);
    if dropped > 0 {
        stderr
            .to_mut()
            .push_str(&format!("\nrg stderr truncated ({dropped} bytes omitted)"));
    }
    match status {
        strop_worker_protocol::ExitStatus::Exit(0 | 1) => {
            if !stderr.trim().is_empty() {
                let _ = tx.control(PickerMsg::Warning(format!("{endpoint}: {}", stderr.trim())));
            }
            Ok(())
        }
        strop_worker_protocol::ExitStatus::Exit(127) => Err(format!(
            "SSH Search here requires rg installed on {endpoint}: {}",
            stderr.trim()
        )),
        strop_worker_protocol::ExitStatus::Exit(code) => Err(format!(
            "SSH rg on {endpoint} exited {code}: {}",
            stderr.trim()
        )),
        strop_worker_protocol::ExitStatus::Signal(signal) => {
            Err(format!("SSH rg on {endpoint} ended by signal {signal}"))
        }
        strop_worker_protocol::ExitStatus::Lost => {
            Err(format!("SSH rg on {endpoint}: worker exit not attested"))
        }
    }
}

fn capture_stderr(mut stderr: impl Read) -> std::io::Result<(Vec<u8>, u64)> {
    const LIMIT: usize = 64 * 1024;
    let mut kept = Vec::new();
    let mut dropped = 0u64;
    let mut buffer = [0u8; 4096];
    loop {
        let count = stderr.read(&mut buffer)?;
        if count == 0 {
            return Ok((kept, dropped));
        }
        let take = count.min(LIMIT - kept.len());
        kept.extend_from_slice(&buffer[..take]);
        dropped += (count - take) as u64;
    }
}

struct Records {
    pending: Vec<u8>,
    delimiter: u8,
}
impl Records {
    fn new(delimiter: u8) -> Self {
        Self {
            pending: Vec::new(),
            delimiter,
        }
    }
    fn feed(
        &mut self,
        chunk: &[u8],
        mut emit: impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        for part in chunk.split_inclusive(|byte| *byte == self.delimiter) {
            let complete = part.last() == Some(&self.delimiter);
            let data = if complete {
                &part[..part.len() - 1]
            } else {
                part
            };
            if self.pending.len().saturating_add(data.len()) > RECORD_LIMIT {
                return Err("SSH rg record exceeds the 1 MiB bound".into());
            }
            self.pending.extend_from_slice(data);
            if complete {
                emit(&self.pending)?;
                self.pending.clear();
            }
        }
        Ok(())
    }
    fn finish(&self) -> Result<(), String> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err("SSH rg output ended in an incomplete record".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SearchWorker;
    #[test]
    fn record_framing_bounds_split_native_names_and_rejects_truncation() {
        let mut records = Records::new(0);
        let mut found = Vec::new();
        records
            .feed(b"a\xff", |record| {
                found.push(record.to_vec());
                Ok(())
            })
            .unwrap();
        records
            .feed(b"b\0second\0", |record| {
                found.push(record.to_vec());
                Ok(())
            })
            .unwrap();
        records.finish().unwrap();
        assert_eq!(found, [b"a\xffb".to_vec(), b"second".to_vec()]);
        records.feed(b"unterminated", |_| Ok(())).unwrap();
        assert!(records.finish().is_err());
        assert!(records.feed(&vec![b'x'; RECORD_LIMIT], |_| Ok(())).is_err());
    }

    /// The same real `rg` and worker codec as an admitted SSH endpoint,
    /// over local pipes: native filename bytes survive both selection
    /// and the returned match identity. No Python supervisor or mock
    /// response can supply the hit.
    #[cfg(unix)]
    #[test]
    fn worker_remote_search_returns_native_path_hits() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        use strop_worker_client::{Transport, Worker};
        let directory = tempfile::tempdir().unwrap();
        let name = OsString::from_vec(b"name_\xff.txt".to_vec());
        let file = directory.path().join(&name);
        std::fs::write(&file, b"needle\n").unwrap();
        let endpoint = RemoteEndpoint::parse("ssh://fixture.example").unwrap();
        let worker = Worker::connect_with(|| {
            let (client_read, worker_write) = std::io::pipe()?;
            let (worker_read, client_write) = std::io::pipe()?;
            std::thread::spawn(move || {
                strop_worker::serve::run(worker_read, worker_write).unwrap();
            });
            Ok(Transport {
                reader: Box::new(client_read),
                writer: Box::new(client_write),
                child: None,
                stderr: None,
            })
        });
        let lease = RemoteWorker::for_test(&endpoint, worker);
        let (tx, rx) = std::sync::mpsc::channel();
        let source = super::super::SourceWorker::new().unwrap();
        let _handle = source.search(
            Arc::new(SearchQuery::parse("needle")),
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
            ResourceLocation::remote(endpoint.clone(), directory.path().to_path_buf()),
            Vec::new(),
            Some(SearchWorker::Admitted(lease.clone())),
            tx,
        );
        let mut found = false;
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap() {
                PickerMsg::Items(batch) => {
                    for item in batch.into_items() {
                        if let crate::Payload::Grep { location, .. } = item.payload {
                            assert_eq!(
                                location.filesystem,
                                strop_workspace::Filesystem::Remote(endpoint.clone())
                            );
                            assert_eq!(
                                location.path.as_os_str().as_bytes(),
                                file.as_os_str().as_bytes()
                            );
                            found = true;
                        }
                    }
                }
                PickerMsg::Finished(outcome) => {
                    assert_eq!(outcome, Outcome::Success(()));
                    break;
                }
                PickerMsg::Warning(_) => {}
                other => panic!("unexpected search event {other:?}"),
            }
        }
        assert!(found, "worker search did not deliver the native-byte hit");
        lease.worker().shutdown().unwrap();
    }

    /// A read-only SFTP account has no worker execution capability;
    /// passing another endpoint's lease must not run rg in the wrong
    /// namespace. Both settle as typed failures, not empty successes.
    #[test]
    fn remote_search_refuses_absent_or_foreign_worker_authority() {
        use strop_worker_client::Worker;
        let endpoint = RemoteEndpoint::parse("ssh://one.example").unwrap();
        let foreign = RemoteWorker::for_test(
            &RemoteEndpoint::parse("ssh://other.example").unwrap(),
            Worker::connect_with(|| panic!("wrong worker must never be contacted")),
        );
        let source = super::super::SourceWorker::new().unwrap();
        for (lease, kind) in [
            (None, FailureKind::Unavailable),
            (Some(foreign), FailureKind::Protocol),
        ] {
            let (tx, rx) = std::sync::mpsc::channel();
            let _handle = source.search(
                Arc::new(SearchQuery::parse("needle")),
                SelectionPolicy {
                    hidden: true,
                    respect_ignore: true,
                },
                ResourceLocation::remote(endpoint.clone(), "/workspace".into()),
                Vec::new(),
                lease.map(SearchWorker::Admitted),
                tx,
            );
            match rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap() {
                PickerMsg::Finished(Outcome::Failed { failure, .. }) => {
                    assert_eq!(failure.kind, kind);
                }
                other => panic!("expected a typed worker authority refusal, got {other:?}"),
            }
        }
    }
    #[test]
    fn explicit_search_reports_worker_deployment_refusal_without_running_rg() {
        let endpoint = RemoteEndpoint::parse("ssh://restricted.example").unwrap();
        let source = super::super::SourceWorker::new().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let _handle = source.search(
            Arc::new(SearchQuery::parse("needle")),
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
            ResourceLocation::remote(endpoint, "/workspace".into()),
            Vec::new(),
            Some(SearchWorker::Admit(Box::new(|_| {
                Err(strop_core::worker::Failure::new(
                    FailureKind::Unavailable,
                    "restricted SSH account cannot run a worker",
                ))
            }))),
            tx,
        );
        match rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap() {
            PickerMsg::Finished(Outcome::Failed { failure, .. }) => {
                assert_eq!(failure.kind, FailureKind::Unavailable);
                assert!(failure.message.contains("restricted SSH account"));
            }
            other => panic!("expected a visible deployment refusal, got {other:?}"),
        }
    }
}
