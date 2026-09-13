//! Fixed protected filesystem operations, invoked only by owned workers.
//! Directory authority does not transfer remote file-save permits.
use ropey::Rope;
#[cfg(all(test, target_os = "linux"))]
mod tests;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use strop_core::worker::CancelToken;
use strop_workspace::operation::*;
use strop_workspace::{Filesystem, Observation, RemoteEndpoint, ResourceLocation};

const HELPER: &str = concat!(
    include_str!("../protected.py"),
    "\n",
    include_str!("observe.py"),
    "\n",
    include_str!("cleanup.py"),
    "\n",
    include_str!("mutate.py"),
    "\n",
    include_str!("main.py")
);
const HEADER_LIMIT: usize = 64 * 1024;
const REPLY_LIMIT: usize = 64 * 1024;
const BODY_LIMIT: usize = 256 * 1024 * 1024;

fn failure(kind: FsFailureKind, detail: impl Into<String>) -> FsFailure {
    FsFailure::new(kind, detail)
}
fn endpoint(intent: &OperationIntent) -> Result<&RemoteEndpoint, FsFailure> {
    let location = intent.location().ok_or_else(|| {
        failure(
            FsFailureKind::InvalidPath,
            "filesystem operation has no resource",
        )
    })?;
    let Filesystem::Remote(endpoint) = &location.filesystem else {
        return Err(failure(
            FsFailureKind::Unsupported,
            "protected SSH operations require a remote namespace",
        ));
    };
    if intent
        .source
        .iter()
        .chain(intent.destination.iter())
        .any(|other| other.filesystem != location.filesystem)
    {
        return Err(failure(
            FsFailureKind::Unsupported,
            "cross-namespace transfer is unsupported",
        ));
    }
    Ok(endpoint)
}
fn path(location: Option<&ResourceLocation>) -> Option<&[u8]> {
    location.map(|location| strop_workspace::addr::uri::path_bytes(&location.path))
}

#[derive(Deserialize)]
struct NativeObservation {
    path: Vec<u8>,
    value: Option<Observation>,
}
impl NativeObservation {
    fn locate(self, endpoint: &RemoteEndpoint) -> Result<LocatedObservation, FsFailure> {
        let path = strop_workspace::addr::uri::bytes_to_path(self.path)
            .map_err(|error| failure(FsFailureKind::Protocol, error.to_string()))?;
        let file = strop_workspace::RemoteFile::from_path(endpoint.clone(), path)
            .map_err(|error| failure(FsFailureKind::Protocol, error.to_string()))?;
        Ok(LocatedObservation {
            location: ResourceLocation::remote(endpoint.clone(), file.path().to_owned()),
            value: self.value,
        })
    }
}
#[derive(Deserialize)]
struct PreparedReply {
    source: Option<NativeObservation>,
    destination: Option<NativeObservation>,
    parents: Vec<NativeObservation>,
    capability: OperationCapability,
}
#[derive(Deserialize)]
struct Reply<T> {
    version: u8,
    value: Option<T>,
    error: Option<HelperError>,
}
#[derive(Deserialize)]
struct HelperError {
    kind: String,
    detail: String,
    unconfirmed: bool,
    observed_destination: Option<Observation>,
    publication: Option<PublicationWitness>,
}
impl HelperError {
    fn failure(self) -> FsFailure {
        let kind = match self.kind.as_str() {
            "unsupported" => FsFailureKind::Unsupported,
            "permission" => FsFailureKind::Permission,
            "conflict" => FsFailureKind::Conflict,
            "busy" => FsFailureKind::Busy,
            "invalid_path" => FsFailureKind::InvalidPath,
            "incomplete" => FsFailureKind::Incomplete,
            "cancelled" => FsFailureKind::Cancelled,
            "io" => FsFailureKind::Io,
            _ => FsFailureKind::Protocol,
        };
        failure(kind, self.detail)
    }
}

enum InvokeError {
    Before(FsFailure),
    Unconfirmed {
        detail: String,
        observed_destination: Option<Box<Observation>>,
        publication: Option<PublicationWitness>,
    },
}
fn invoke<T: DeserializeOwned>(
    endpoint: &RemoteEndpoint,
    request: Value,
    contents: Option<&Rope>,
    mutation: bool,
    token: &CancelToken,
) -> Result<T, InvokeError> {
    if token.is_cancelled() {
        return Err(InvokeError::Before(failure(
            FsFailureKind::Cancelled,
            "cancelled before helper launch",
        )));
    }
    let mut header = serde_json::to_vec(&request).map_err(|error| {
        InvokeError::Before(failure(FsFailureKind::Protocol, error.to_string()))
    })?;
    header.push(b'\n');
    if header.len() > HEADER_LIMIT {
        return Err(InvokeError::Before(failure(
            FsFailureKind::Unsupported,
            "filesystem request header exceeds its bound",
        )));
    }
    let command = crate::RemoteCommand::python(HELPER, Vec::new(), std::path::Path::new("/"))
        .map_err(|error| {
            InvokeError::Before(failure(FsFailureKind::Unsupported, error.to_string()))
        })?;
    let mut chunks = vec![header.as_slice()];
    if let Some(contents) = contents {
        chunks.extend(contents.chunks().map(str::as_bytes));
    }
    let lost = |detail: String| {
        if mutation {
            InvokeError::Unconfirmed {
                detail,
                observed_destination: None,
                publication: None,
            }
        } else {
            InvokeError::Before(failure(FsFailureKind::Io, detail))
        }
    };
    let output = crate::run_with_input(endpoint, &command, token, &chunks)
        .map_err(|error| lost(error.to_string()))?;
    if output.stdout_dropped != 0 || output.stdout.len() > REPLY_LIMIT {
        return Err(lost("filesystem response exceeded its bound".into()));
    }
    let reply: Reply<T> = serde_json::from_slice(&output.stdout).map_err(|error| {
        lost(format!(
            "{error}; {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    })?;
    if reply.version != 1 {
        return Err(lost("filesystem protocol version differs".into()));
    }
    if let Some(error) = reply.error {
        return Err(if error.unconfirmed {
            InvokeError::Unconfirmed {
                detail: error.detail,
                observed_destination: error.observed_destination.map(Box::new),
                publication: error.publication,
            }
        } else {
            InvokeError::Before(error.failure())
        });
    }
    if !output.status.success() || output.stdin_error.is_some() {
        return Err(lost(format!(
            "filesystem helper transport did not complete: {:?}; {:?}",
            output.status, output.stdin_error
        )));
    }
    reply
        .value
        .ok_or_else(|| lost("filesystem response omitted its result".into()))
}
fn read_error(error: InvokeError) -> FsFailure {
    match error {
        InvokeError::Before(error) => error,
        InvokeError::Unconfirmed { detail, .. } => failure(FsFailureKind::Protocol, detail),
    }
}

pub fn prepare(
    intent: &OperationIntent,
    allow_occupied: bool,
    token: &CancelToken,
) -> Result<PreparedOperation, FsFailure> {
    let endpoint = endpoint(intent)?;
    let buffer_copy =
        intent.kind == OperationKind::Copy && intent.copy_version == CopyVersion::Buffer;
    let reply: PreparedReply = invoke(endpoint, json!({"version":1,"action":"prepare","kind":intent.kind,
        "source":path(intent.source.as_ref()),"destination":path(intent.destination.as_ref()),
        "buffer_copy":buffer_copy,"allow_occupied":allow_occupied,"expected_content":intent.expected_content}), None, false, token).map_err(read_error)?;
    if reply.parents.len() > 2 {
        return Err(failure(
            FsFailureKind::Protocol,
            "helper returned too many parent observations",
        ));
    }
    Ok(PreparedOperation {
        intent: intent.clone(),
        source: reply
            .source
            .map(|value| value.locate(endpoint))
            .transpose()?,
        destination: reply
            .destination
            .map(|value| value.locate(endpoint))
            .transpose()?,
        parents: reply
            .parents
            .into_iter()
            .map(|value| value.locate(endpoint))
            .collect::<Result<_, _>>()?,
        dependencies: Vec::new(),
        capability: reply.capability,
    })
}

fn request(operation: &PreparedOperation, action: &str) -> Value {
    json!({"version":1,"action":action,"kind":operation.intent.kind,
        "source":path(operation.source.as_ref().map(|value| &value.location)),
        "destination":path(operation.destination.as_ref().map(|value| &value.location)),
        "logical_source":path(operation.intent.source.as_ref()),"logical_destination":path(operation.intent.destination.as_ref()),
        "before_source":operation.source.as_ref().and_then(|value| value.value.as_ref()),
        "before_destination":operation.destination.as_ref().and_then(|value| value.value.as_ref()),
        "parents":operation.parents.iter().map(|parent| json!({"path":path(Some(&parent.location)),"value":parent.value})).collect::<Vec<_>>(),
        "capability":operation.capability,"buffer_copy":operation.intent.copy_version == CopyVersion::Buffer})
}

pub fn execute(
    operation: &PreparedOperation,
    contents: Option<&Rope>,
    receipts: &[StepReceipt],
    token: &CancelToken,
) -> StepOutcome {
    let endpoint = match endpoint(&operation.intent) {
        Ok(endpoint) => endpoint,
        Err(error) => return StepOutcome::Refused(error),
    };
    let mut request = request(operation, "apply");
    if operation.intent.kind == OperationKind::Copy
        && operation.intent.copy_version == CopyVersion::Buffer
    {
        let Some(contents) = contents else {
            return StepOutcome::Refused(failure(
                FsFailureKind::Conflict,
                "buffer copy snapshot is missing",
            ));
        };
        if contents.len_bytes() > BODY_LIMIT {
            return StepOutcome::Refused(failure(
                FsFailureKind::Unsupported,
                "buffer copy exceeds the 256 MiB upload bound",
            ));
        }
        let mut digest = Sha256::new();
        for chunk in contents.chunks() {
            digest.update(chunk.as_bytes());
        }
        let digest: [u8; 32] = digest.finalize().into();
        request["length"] = json!(contents.len_bytes());
        request["digest"] = json!(digest);
    }
    request["parents"] = Value::Array(
        operation
            .parents
            .iter()
            .map(|parent| {
                let value = parent.value.as_ref().or_else(|| {
                    operation.dependencies.iter().find_map(|step| {
                        receipts.iter().find_map(|receipt| {
                            if receipt.step != *step
                                || receipt
                                    .operation
                                    .destination
                                    .as_ref()
                                    .map(|value| &value.location)
                                    != Some(&parent.location)
                            {
                                return None;
                            }
                            match &receipt.outcome {
                                StepOutcome::Committed {
                                    destination_after, ..
                                } => destination_after.as_ref(),
                                _ => None,
                            }
                        })
                    })
                });
                json!({"path":path(Some(&parent.location)),"value":value})
            })
            .collect(),
    );
    let vacated = operation.destination.as_ref().is_some_and(|destination| {
        destination.value.as_ref().is_some_and(|expected| {
            operation.dependencies.iter().any(|step| {
                receipts.iter().any(|receipt| {
                    receipt.step == *step
                        && receipt.outcome.is_committed()
                        && receipt.operation.source.as_ref().is_some_and(|source| {
                            source.location == destination.location
                                && source
                                    .value
                                    .as_ref()
                                    .is_some_and(|value| value.same_object(expected))
                        })
                })
            })
        })
    });
    request["vacated"] = json!(vacated);
    let contents = contents.filter(|_| {
        operation.intent.kind == OperationKind::Copy
            && operation.intent.copy_version == CopyVersion::Buffer
    });
    match invoke(endpoint, request, contents, true, token) {
        Ok(outcome) => outcome,
        Err(InvokeError::Before(error)) if error.kind == FsFailureKind::Cancelled => {
            StepOutcome::Cancelled {
                detail: error.detail,
            }
        }
        Err(InvokeError::Before(error)) => StepOutcome::Refused(error),
        Err(InvokeError::Unconfirmed {
            detail,
            observed_destination,
            publication,
        }) => StepOutcome::Unconfirmed {
            detail,
            observed_destination: observed_destination.map(|value| *value),
            recovery: None,
            publication,
        },
    }
}

pub fn verify(receipt: &StepReceipt, token: &CancelToken) -> Result<VerifiedOutcome, FsFailure> {
    let endpoint = endpoint(&receipt.operation.intent)?;
    let mut request = request(&receipt.operation, "verify");
    request["observed_destination"] = json!(match &receipt.outcome {
        StepOutcome::Committed {
            destination_after, ..
        } => destination_after.as_ref(),
        StepOutcome::Unconfirmed {
            observed_destination,
            ..
        } => observed_destination.as_ref(),
        _ => None,
    });
    request["publication"] = json!(match &receipt.outcome {
        StepOutcome::Committed { publication, .. }
        | StepOutcome::Unconfirmed { publication, .. } => publication.as_ref(),
        _ => None,
    });
    invoke(endpoint, request, None, false, token).map_err(read_error)
}
