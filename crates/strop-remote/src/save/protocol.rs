//! Versioned helper messages and a borrowed-chunk upload through the owned lease.
use super::{ContentDigest, RefusalKind, RemoteSaveError, Stamp};
use crate::RemoteCommand;
use ropey::Rope;
use serde::{Deserialize, Serialize};
use strop_core::worker::CancelToken;
use strop_workspace::RemoteFile;

const VERSION: u8 = 1;
const HEADER_LIMIT: usize = 16 * 1024;
const REPLY_LIMIT: usize = 64 * 1024;
pub(super) const HELPER: &str = concat!(
    include_str!("../protected.py"),
    "\n",
    include_str!("helper.py")
);

#[derive(Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(super) enum Operation<'a> {
    Edit {
        length: u64,
        digest: &'a ContentDigest,
    },
    Save {
        before: &'a Stamp,
        length: u64,
        digest: &'a ContentDigest,
        #[serde(skip)]
        contents: &'a Rope,
    },
    Verify {
        before: &'a Stamp,
        length: u64,
        digest: &'a ContentDigest,
    },
}
impl Operation<'_> {
    fn contents(&self) -> Option<&Rope> {
        match self {
            Self::Save { contents, .. } => Some(contents),
            _ => None,
        }
    }
    fn error(&self, kind: RefusalKind, detail: impl Into<String>) -> RemoteSaveError {
        if matches!(self, Self::Edit { .. }) {
            RemoteSaveError::refused(kind, detail)
        } else {
            RemoteSaveError::Unconfirmed {
                detail: detail.into(),
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Reply {
    Ready {
        stamp: Stamp,
    },
    Written {
        stamp: Stamp,
    },
    Unchanged {
        stamp: Stamp,
    },
    Refused {
        kind: RefusalKind,
        detail: String,
        unconfirmed: bool,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u8,
    result: Reply,
}

pub(super) fn invoke(
    file: &RemoteFile,
    operation: Operation<'_>,
    token: &CancelToken,
) -> Result<Reply, RemoteSaveError> {
    if token.is_cancelled() {
        return Err(RemoteSaveError::refused(
            RefusalKind::Cancelled,
            "cancelled before helper launch",
        ));
    }
    #[derive(Serialize)]
    struct Request<'a, 'b> {
        version: u8,
        #[serde(flatten)]
        operation: &'a Operation<'b>,
    }
    let mut header = serde_json::to_vec(&Request {
        version: VERSION,
        operation: &operation,
    })
    .map_err(|error| RemoteSaveError::refused(RefusalKind::Protocol, error.to_string()))?;
    header.push(b'\n');
    if header.len() > HEADER_LIMIT {
        return Err(RemoteSaveError::refused(
            RefusalKind::Protocol,
            "request header exceeds its bound",
        ));
    }
    let command = RemoteCommand::python(
        HELPER,
        vec![file.path().as_os_str().to_owned()],
        std::path::Path::new("/"),
    )
    .map_err(|error| RemoteSaveError::refused(RefusalKind::Unsupported, error.to_string()))?;
    let mut chunks = vec![header.as_slice()];
    if let Some(contents) = operation.contents() {
        chunks.extend(contents.chunks().map(str::as_bytes));
    }
    let output = crate::exec::run_with_input(file.endpoint(), &command, token, &chunks)
        .map_err(|error| operation.error(RefusalKind::Io, error.to_string()))?;
    if output.stdout_dropped != 0 || output.stdout.len() > REPLY_LIMIT {
        return Err(operation.error(RefusalKind::Protocol, "save response exceeded its bound"));
    }
    let response: Response = serde_json::from_slice(&output.stdout).map_err(|error| {
        operation.error(
            RefusalKind::Protocol,
            format!("{error}; {}", String::from_utf8_lossy(&output.stderr)),
        )
    })?;
    if response.version != VERSION {
        return Err(operation.error(RefusalKind::Protocol, "save protocol version differs"));
    }
    if let Reply::Refused {
        kind,
        mut detail,
        unconfirmed,
    } = response.result
    {
        if let Some(error) = output.stdin_error {
            detail.push_str(&format!("; input delivery: {error}"));
        }
        return Err(if unconfirmed {
            RemoteSaveError::Unconfirmed { detail }
        } else {
            RemoteSaveError::refused(kind, detail)
        });
    }
    if !output.status.success() {
        return Err(operation.error(
            RefusalKind::Protocol,
            "helper exited unsuccessfully without a refusal",
        ));
    }
    if let Some(error) = output.stdin_error {
        return Err(operation.error(RefusalKind::Io, format!("input delivery failed: {error}")));
    }
    Ok(response.result)
}
