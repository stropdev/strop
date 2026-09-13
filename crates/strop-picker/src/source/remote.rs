//! Read-only SSH search: select native paths first, then search bounded argv batches.
use super::{
    flow::StreamSender,
    query::{parse_json_match, scoped_path, RECORD_LIMIT},
    PickerMsg, SelectionPolicy, SourceSnapshot,
};
use crate::query::{CaseMode, ContentExpr, ContentPlan, FileSelectionPlan, SearchQuery};
use std::{ffi::OsString, path::PathBuf, sync::Arc};
use strop_core::worker::{CancelReason, CancelToken, FailureKind, Outcome};
use strop_workspace::{RemoteEndpoint, ResourceLocation};

pub(super) fn run_search(
    query: Arc<SearchQuery>,
    policy: SelectionPolicy,
    root: ResourceLocation,
    snapshots: Vec<SourceSnapshot>,
    tx: StreamSender,
    token: CancelToken,
) -> Outcome<()> {
    let result = search(&query, policy, &root, snapshots, &tx, &token);
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
    tx: &StreamSender,
    token: &CancelToken,
) -> Result<(), String> {
    let strop_workspace::Filesystem::Remote(endpoint) = &root.filesystem else {
        return Err("SSH search requires an SSH scope".into());
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
    run(endpoint, root, arguments, token, tx, |chunk| {
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
    super::snapshots::emit_snapshots(root, &content, snapshots, &mut paths, tx, token)?;
    let pattern = match &content.expr {
        ContentExpr::Literal(text) | ContentExpr::Regex(text) => text,
    };
    common.extend(["--json".into(), "-e".into(), pattern.into()]);
    common.push(
        match content.case {
            CaseMode::Smart => "--smart-case",
            CaseMode::Sensitive => "--case-sensitive",
            CaseMode::Ignore => "--ignore-case",
        }
        .into(),
    );
    if matches!(content.expr, ContentExpr::Literal(_)) {
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
        run(endpoint, root, args, token, tx, |chunk| {
            records.feed(chunk, |record| {
                let items = parse_json_match(record, root)?;
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
    root: &ResourceLocation,
    args: Vec<OsString>,
    token: &CancelToken,
    tx: &StreamSender,
    consume: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let command = strop_remote::RemoteCommand::new("rg", args, &root.path)
        .map_err(|error| error.to_string())?;
    let output =
        strop_remote::stream(endpoint, &command, token, consume).map_err(|error| match error {
            strop_remote::RemoteStreamError::Remote(error) => {
                format!("SSH search on {endpoint}: {error}")
            }
            strop_remote::RemoteStreamError::Consumer(error) => error,
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
                let _ = tx.control(PickerMsg::Warning(format!("{endpoint}: {}", stderr.trim())));
            }
            Ok(())
        }
        Some(127) => Err(format!(
            "SSH Search here requires rg installed on {endpoint}: {}",
            stderr.trim()
        )),
        code => Err(format!(
            "SSH rg on {endpoint} exited {code:?}: {}",
            stderr.trim()
        )),
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
