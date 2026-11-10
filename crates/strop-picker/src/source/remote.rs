//! Search on an SSH host: `rg` runs there as a channel on the endpoint's
//! master (plan 0090 §5). Select native paths first, then search bounded
//! argv batches.
use super::{
    flow::StreamSender,
    query::{parse_json_match_with, scoped_path, RECORD_LIMIT},
    PickerMsg, SelectionPolicy, SourceSnapshot,
};
use crate::query::{CaseMode, ContentPlan, Evidence, FileSelectionPlan, SearchQuery};
use std::ffi::OsStr;
use std::time::Duration;
use std::{ffi::OsString, path::PathBuf, sync::Arc};
use strop_core::process::{stream_with, StreamError, StreamPolicy};
use strop_core::worker::{CancelReason, CancelToken, FailureKind, Outcome};
use strop_remote::exec::ExecHost;
use strop_workspace::ResourceLocation;

/// One `rg` run's wall-clock budget.
const DEADLINE: Duration = Duration::from_secs(120);

fn case_flag(case: CaseMode) -> &'static str {
    match case {
        CaseMode::Smart => "--smart-case",
        CaseMode::Sensitive => "--case-sensitive",
        CaseMode::Ignore => "--ignore-case",
    }
}

pub(super) fn run_search(
    query: Arc<SearchQuery>,
    policy: SelectionPolicy,
    root: ResourceLocation,
    snapshots: Vec<SourceSnapshot>,
    host: &ExecHost,
    tx: &StreamSender,
    token: CancelToken,
) -> Outcome<()> {
    let strop_workspace::Filesystem::Remote(endpoint) = &root.filesystem else {
        return Outcome::failed(FailureKind::Protocol, "remote search requires an SSH scope");
    };
    if host.endpoint() != endpoint {
        return Outcome::failed(
            FailureKind::Protocol,
            format!(
                "search scope {endpoint} does not match the connection to {}",
                host.endpoint()
            ),
        );
    }
    let result = search(&query, policy, &root, snapshots, host, tx, &token);
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
    host: &ExecHost,
    tx: &StreamSender,
    token: &CancelToken,
) -> Result<(), String> {
    let backend = match &root.filesystem {
        strop_workspace::Filesystem::Remote(endpoint) => endpoint.to_string(),
        strop_workspace::Filesystem::Local => root.label(),
    };
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
    run(&backend, host, root, arguments, token, tx, |chunk| {
        records.feed(chunk, |record| {
            let location = scoped_path(root, record.to_vec())?;
            let relative = location
                .path
                .strip_prefix(&root.path)
                .map_err(|_| "rg listed a file outside the scope")?;
            if !selection.allows(&super::display_path(relative)) {
                return Ok(());
            }
            bytes = bytes.saturating_add(relative.as_os_str().as_encoded_bytes().len());
            if paths.len() >= super::selection::PATH_LIMIT || bytes > super::selection::PATH_BYTES {
                return Err(
                    "remote search file selection exceeds 100000 paths or 16 MiB; narrow the scope"
                        .into(),
                );
            }
            paths.push(relative.to_path_buf());
            Ok(())
        })
    })?;
    records.finish()?;
    // Remote project discovery and symbol evidence are not built:
    // `repo:` atoms admit (overfetch), never drop.
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
    common.push(case_flag(content.provider_case()).into());
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
            return Err("remote search cancelled".into());
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
                return Err("remote search path or expression exceeds the argv bound".into());
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
        run(&backend, host, root, args, token, tx, |chunk| {
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
    backend: &str,
    host: &ExecHost,
    root: &ResourceLocation,
    args: Vec<OsString>,
    token: &CancelToken,
    tx: &StreamSender,
    consume: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut argv: Vec<&OsStr> = vec![OsStr::new("rg")];
    argv.extend(args.iter().map(OsString::as_os_str));
    let mut command = host.command(&argv, Some(&root.path));
    let policy = StreamPolicy {
        stderr_limit: 64 * 1024,
        stderr_tail: 0,
        deadline: DEADLINE,
        hold_stdin: false,
    };
    let output =
        stream_with(&mut command, token, &policy, consume).map_err(|error| match error {
            StreamError::Consumer(message) => message,
            StreamError::Spawn(message) => format!("search on {backend}: {message}"),
            StreamError::Cancelled => "remote search cancelled".into(),
            StreamError::TimedOut(after) => {
                format!("rg on {backend} timed out after {} s", after.as_secs())
            }
            StreamError::Failure(failure) => format!("rg on {backend}: {}", failure.message),
        })?;
    let mut stderr = String::from_utf8_lossy(&output.stderr);
    if output.stderr_dropped > 0 {
        stderr.to_mut().push_str(&format!(
            "\nrg stderr truncated ({} bytes omitted)",
            output.stderr_dropped
        ));
    }
    match output.status.code() {
        Some(0 | 1) => {
            if !stderr.trim().is_empty() {
                let _ = tx.control(PickerMsg::Warning(format!("{backend}: {}", stderr.trim())));
            }
            Ok(())
        }
        Some(127) => Err(format!(
            "Search here requires rg installed on {backend}: {}",
            stderr.trim()
        )),
        Some(255) => Err(format!("ssh to {backend} failed: {}", stderr.trim())),
        Some(code) => Err(format!("rg on {backend} exited {code}: {}", stderr.trim())),
        None => Err(format!("rg on {backend} ended by a signal")),
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
                return Err("rg record exceeds the 1 MiB bound".into());
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
            Err("rg output ended in an incomplete record".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_case_selects_the_remote_rg_flag() {
        for (input, expected) in [
            ("needle", "--smart-case"),
            ("NEEDLE", "--smart-case"),
            ("case:ignore NEEDLE", "--ignore-case"),
            ("case:insensitive NEEDLE", "--ignore-case"),
            ("case:sensitive NEEDLE", "--case-sensitive"),
            ("case:smart NEEDLE", "--smart-case"),
            ("case:smart text:foo OR text:Bar", "--ignore-case"),
        ] {
            let query = SearchQuery::parse(input);
            let plan = ContentPlan::compile(&query).unwrap().unwrap();
            assert_eq!(case_flag(plan.provider_case()), expected, "{input}");
        }
    }

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
}
