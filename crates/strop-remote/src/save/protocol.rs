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
pub(crate) const HELPER: &str = concat!(
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
    classify(&operation, output)
}

/// The pure reply boundary: bound checks, decoding, version and exit-status
/// classification over one completed helper exchange. No I/O lives here, so
/// the malformed/ambiguous-reply campaigns exercise exactly this function.
fn classify(
    operation: &Operation<'_>,
    output: crate::exec::CommandOutput,
) -> Result<Reply, RemoteSaveError> {
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

/// VF08 reply-boundary campaign: the pure classifier is exercised with
/// malformed, ambiguous and dishonest helper outputs. A bad reply must
/// never decode into a permit, a clean stamp or another binding's outcome.
#[cfg(test)]
mod reply_tests {
    use super::*;

    fn output(code: u32, stdout: String) -> crate::exec::CommandOutput {
        crate::exec::CommandOutput {
            status: crate::exec::RemoteExitStatus::Exited(code),
            stdout: stdout.into_bytes(),
            stderr: Vec::new(),
            stdout_dropped: 0,
            stderr_dropped: 0,
            stdin_error: None,
        }
    }
    fn stamp() -> Stamp {
        Stamp {
            device: 1,
            inode: 2,
            size: 3,
            mtime_ns: 4,
            ctime_ns: 5,
            mode: 0o644,
            uid: 1000,
            gid: 1000,
            content: ContentDigest([7u8; 32]),
            attributes: ContentDigest([8u8; 32]),
        }
    }

    fn stamp_json() -> String {
        serde_json::to_string(&stamp()).unwrap()
    }

    /// Run a check against both operation shapes: an Edit (whose protocol
    /// failures are clean refusals) and a Verify (whose failures must be
    /// Unconfirmed, never a clean "nothing happened").
    fn with_operations(body: impl Fn(&Operation, &Operation)) {
        let digest = ContentDigest([7u8; 32]);
        let before = stamp();
        body(
            &Operation::Edit {
                length: 3,
                digest: &digest,
            },
            &Operation::Verify {
                before: &before,
                length: 3,
                digest: &digest,
            },
        );
    }

    /// A reply the helper would emit for a successful write.
    fn written() -> String {
        format!(
            "{{\"version\":1,\"result\":{{\"status\":\"written\",\"stamp\":{}}}}}\n",
            stamp_json()
        )
    }

    #[test]
    fn malformed_and_trailing_replies_are_protocol_failures() {
        for body in [
            "not json at all".to_string(),
            written().replace('\n', "junk"),
            format!("{}}}", written().trim_end()),
        ] {
            with_operations(|edit, save| {
                // For a save/verify the safe direction is Unconfirmed, never
                // a clean refusal that could read as "nothing happened".
                assert!(
                    matches!(
                        classify(save, output(0, body.clone())),
                        Err(RemoteSaveError::Unconfirmed { .. })
                    ),
                    "save: {body}"
                );
                assert!(
                    matches!(
                        classify(edit, output(0, body.clone())),
                        Err(RemoteSaveError::Refused {
                            kind: RefusalKind::Protocol,
                            ..
                        })
                    ),
                    "edit: {body}"
                );
            });
        }
    }

    #[test]
    fn unknown_status_and_unknown_fields_are_rejected() {
        let stamp = stamp_json();
        for body in [
            format!("{{\"version\":1,\"result\":{{\"status\":\"done\",\"stamp\":{stamp}}}}}\n"),
            format!(
                "{{\"version\":1,\"result\":{{\"status\":\"written\",\"stamp\":{stamp},\"extra\":true}}}}\n"
            ),
            format!(
                "{{\"version\":1,\"result\":{{\"status\":\"written\",\"stamp\":{stamp}}},\"trace\":[]}}\n"
            ),
        ] {
            with_operations(|_, save| {
                assert!(
                    matches!(
                        classify(save, output(0, body.clone())),
                        Err(RemoteSaveError::Unconfirmed { .. })
                    ),
                    "{body}"
                );
            });
        }
    }

    #[test]
    fn absent_null_and_duplicate_fields_are_not_defaults() {
        for body in [
            "{\"version\":1,\"result\":{\"status\":\"refused\",\"kind\":\"busy\",\"detail\":\"held\"}}\n",
            "{\"version\":1,\"result\":{\"status\":\"refused\",\"kind\":\"busy\",\"detail\":\"held\",\"unconfirmed\":null}}\n",
            "{\"version\":1,\"version\":1,\"result\":{\"status\":\"unchanged\",\"stamp\":null}}\n",
        ] {
            with_operations(|_, save| {
                assert!(
                    classify(save, output(0, body.to_string())).is_err(),
                    "{body}"
                );
            });
        }
    }

    #[test]
    fn a_version_mismatch_is_a_protocol_failure() {
        for version in [0u8, 2, 255] {
            let body = format!(
                "{{\"version\":{version},\"result\":{{\"status\":\"written\",\"stamp\":{}}}}}\n",
                stamp_json()
            );
            with_operations(|_, save| {
                assert!(matches!(
                    classify(save, output(0, body.clone())),
                    Err(RemoteSaveError::Unconfirmed { .. })
                ));
            });
        }
    }

    #[test]
    fn a_failed_exit_cannot_report_written() {
        // A syntactically perfect "written" reply from a helper that exited
        // nonzero is a protocol failure, never a clean stamp.
        with_operations(|edit, save| {
            assert!(matches!(
                classify(save, output(1, written())),
                Err(RemoteSaveError::Unconfirmed { .. })
            ));
            assert!(matches!(
                classify(edit, output(1, written())),
                Err(RemoteSaveError::Refused {
                    kind: RefusalKind::Protocol,
                    ..
                })
            ));
        });
    }

    #[test]
    fn dropped_or_oversize_replies_are_bounded_failures() {
        with_operations(|_, save| {
            let mut truncated = output(0, written());
            truncated.stdout_dropped = 7;
            assert!(matches!(
                classify(save, truncated),
                Err(RemoteSaveError::Unconfirmed { .. })
            ));
            let oversized = output(0, "x".repeat(REPLY_LIMIT + 1));
            assert!(matches!(
                classify(save, oversized),
                Err(RemoteSaveError::Unconfirmed { .. })
            ));
        });
    }

    #[test]
    fn refusals_carry_kind_and_the_unconfirmed_flag() {
        with_operations(|_, save| {
            let refused = "{\"version\":1,\"result\":{\"status\":\"refused\",\"kind\":\"conflict\",\"detail\":\"changed\",\"unconfirmed\":false}}\n";
            assert!(matches!(
                classify(save, output(1, refused.to_string())),
                Err(RemoteSaveError::Refused {
                    kind: RefusalKind::Conflict,
                    ..
                })
            ));
            let uncertain = "{\"version\":1,\"result\":{\"status\":\"refused\",\"kind\":\"cancelled\",\"detail\":\"commit raced\",\"unconfirmed\":true}}\n";
            assert!(matches!(
                classify(save, output(1, uncertain.to_string())),
                Err(RemoteSaveError::Unconfirmed { .. })
            ));
            // An unknown refusal kind cannot be invented either.
            let invented = "{\"version\":1,\"result\":{\"status\":\"refused\",\"kind\":\"success\",\"detail\":\"x\",\"unconfirmed\":false}}\n";
            assert!(classify(save, output(1, invented.to_string())).is_err());
        });
    }

    #[test]
    fn written_and_unchanged_decode_with_their_stamps() {
        with_operations(|_, save| {
            match classify(save, output(0, written())) {
                Ok(Reply::Written { stamp: replied }) => assert_eq!(replied, stamp()),
                _ => panic!("a valid written reply decodes"),
            }
            let unchanged = format!(
                "{{\"version\":1,\"result\":{{\"status\":\"unchanged\",\"stamp\":{}}}}}\n",
                stamp_json()
            );
            match classify(save, output(0, unchanged)) {
                Ok(Reply::Unchanged { .. }) => {}
                _ => panic!("a valid unchanged reply decodes"),
            }
        });
    }
}
