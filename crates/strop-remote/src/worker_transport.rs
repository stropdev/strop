//! The SSH worker transport (0058 WK07): exec the deployed worker
//! object directly into `--worker-stdio` over the authenticated ssh
//! channel — the same protocol, framing and handshake as the local
//! worker, with system OpenSSH owning authentication, host-key policy,
//! aliases and ProxyJump exactly as the pooled SFTP connections do.
//!
//! - **Direct launch, never a supervisor.** The remote command line is
//!   the quoted object path plus the fixed `--worker-stdio` flag. No
//!   Python, no shell supervisor, no PATH lookup: the remote login shell
//!   parses one constant-shaped line whose single dynamic word crosses
//!   the tested [`crate::bootstrap::quote`] boundary.
//! - **Shell text can never impersonate a worker.** Readiness is the
//!   framed protocol handshake; shell or interpreter diagnostics fail
//!   the decode and surface as typed handshake failures with the
//!   bounded ssh/worker stderr attached.
//! - **Reconnect re-handshakes fresh.** Every connection attempt spawns
//!   a fresh ssh exec and a fresh worker incarnation; the client lease
//!   ([`strop_worker_client::Worker`]) respawns through the factory and
//!   stale prepared authority dies with the old incarnation. Old
//!   handles are never revived.
use std::io::Read;
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::Duration;

use strop_worker_client::{ClientError, StderrCapture, Transport, Worker};
use strop_worker_protocol::codec::{self, Incoming};
use strop_worker_protocol::frame::{self, FrameDecoder};
use strop_worker_protocol::{
    ClientMessage, EndpointInfo, Session, WorkerMessage, PROTOCOL_VERSION,
};
use strop_workspace::operation::{
    LocatedObservation, OperationIntent, OperationRefusal, PreparedOperation, StepOutcome,
    StepReceipt,
};
use strop_workspace::{Filesystem, RemoteEndpoint, ResourceLocation};

/// Spawn plus `hello`/`welcome` must resolve inside this window — the
/// same bound the local client holds (never an unbounded wait on a
/// half-alive ssh).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the remote worker's real handshake reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeFacts {
    pub protocol: u32,
    pub worker: EndpointInfo,
    pub session: Session,
}

/// Transport failures, classified for the deploy provider's honest
/// outcome vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkerTransportError {
    /// The local ssh(1) could not be started.
    #[error("ssh spawn: {0}")]
    Spawn(String),
    /// The process ran but the protocol handshake did not complete or
    /// failed to decode; the bounded stderr diagnostic travels in the
    /// message. Shell diagnostics can never impersonate a Welcome.
    #[error("worker handshake over ssh failed: {0}")]
    Handshake(String),
}
fn worker_line(object: &str) -> String {
    format!("{} --worker-stdio", crate::bootstrap::quote(object))
}

/// Spawn one ssh exec of the deployed object in worker mode. The
/// caller owns all three pipes.
fn spawn(
    endpoint: &RemoteEndpoint,
    object: &str,
) -> Result<(Child, ChildStdin, ChildStdout, std::process::ChildStderr), WorkerTransportError> {
    let mut command = crate::ssh::exec_command(endpoint, &worker_line(object));
    let mut child = command
        .spawn()
        .map_err(|error| WorkerTransportError::Spawn(error.to_string()))?;
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(WorkerTransportError::Spawn(
            "ssh stdio pipes unavailable".into(),
        ));
    };
    Ok((child, stdin, stdout, stderr))
}

/// A client lease on the deployed worker at `object` on `endpoint`,
/// whose handshake must report the endpoint's admitted artifact
/// `target` (the triple deployment verified, which may differ from
/// this client's platform). Lazy: nothing spawns until the first
/// request; every (re)connection is a fresh ssh exec and a fresh
/// worker incarnation, so a lost connection never replays against
/// stale authority.
pub fn worker(endpoint: &RemoteEndpoint, object: &str, target: &str) -> Worker {
    let endpoint = endpoint.clone();
    let object = object.to_owned();
    Worker::connect_deployed(target.to_owned(), move || {
        let (child, stdin, stdout, stderr) =
            spawn(&endpoint, &object).map_err(|error| std::io::Error::other(error.to_string()))?;
        Ok(Transport {
            reader: Box::new(stdout),
            writer: Box::new(stdin),
            child: Some(child),
            stderr: Some(StderrCapture::spawn(stderr)),
        })
    })
}

/// Launch the published object directly into worker mode and run the
/// real protocol handshake, returning what the worker reported. The
/// probe worker is retired before this returns — deployment's
/// activation handshake and the client's live lease are separate
/// outcomes by design (0058 §5 step 6).
pub fn handshake(
    endpoint: &RemoteEndpoint,
    object: &str,
    client: &EndpointInfo,
) -> Result<WelcomeFacts, WorkerTransportError> {
    let (mut child, mut stdin, stdout, stderr) = spawn(endpoint, object)?;
    let capture = StderrCapture::spawn(stderr);
    let outcome = handshake_inner(&mut stdin, stdout, client);
    let _ = child.kill();
    let _ = child.wait();
    let facts = outcome.map_err(|error| {
        let detail = capture.text();
        if detail.is_empty() {
            error
        } else {
            WorkerTransportError::Handshake(format!("{error}; stderr: {detail}"))
        }
    })?;
    if facts.protocol != PROTOCOL_VERSION {
        return Err(WorkerTransportError::Handshake(format!(
            "worker speaks protocol {}, expected {PROTOCOL_VERSION}",
            facts.protocol
        )));
    }
    Ok(facts)
}

/// The framed exchange: one `hello`, one bounded wait for `welcome`.
fn handshake_inner(
    stdin: &mut impl std::io::Write,
    stdout: impl Read + Send + 'static,
    client: &EndpointInfo,
) -> Result<WelcomeFacts, WorkerTransportError> {
    let (sender, receiver) = channel();
    thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        read_first_frame(&mut reader, sender);
    });
    codec::write_envelope(
        &mut *stdin,
        &ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            client: client.clone(),
        },
    )
    .map_err(|error| WorkerTransportError::Handshake(format!("cannot send hello: {error}")))?;
    match receiver.recv_timeout(HANDSHAKE_TIMEOUT) {
        Ok(Ok(Incoming::Envelope(WorkerMessage::Welcome {
            protocol,
            worker,
            session,
            ..
        }))) => Ok(WelcomeFacts {
            protocol,
            worker,
            session,
        }),
        Ok(Ok(Incoming::Envelope(WorkerMessage::Error { error, .. }))) => Err(
            WorkerTransportError::Handshake(format!("worker refused: {error}")),
        ),
        Ok(Ok(_)) => Err(WorkerTransportError::Handshake(
            "the first worker message was not a welcome".into(),
        )),
        Ok(Err(error)) => Err(WorkerTransportError::Handshake(error)),
        Err(_) => Err(WorkerTransportError::Handshake(format!(
            "no welcome within {}s",
            HANDSHAKE_TIMEOUT.as_secs()
        ))),
    }
}

/// Read exactly one frame on the reader thread; EOF, corruption and
/// chunk-class frames are all typed handshake failures.
fn read_first_frame(
    reader: &mut impl Read,
    sender: Sender<Result<Incoming<WorkerMessage>, String>>,
) {
    let mut decoder = FrameDecoder::new();
    let result = match frame::read_frame(reader, &mut decoder) {
        Ok(Some(body)) => codec::decode_body::<WorkerMessage>(&body)
            .map_err(|error| format!("welcome failed to decode: {error}")),
        Ok(None) => Err("worker closed the channel before any welcome".into()),
        Err(error) => Err(format!("welcome frame unreadable: {error}")),
    };
    let _ = sender.send(result);
}

/// One endpoint's admitted worker with namespace translation at the
/// client boundary (0058 WK07): the deployed worker serves its OWN
/// native namespace (`Filesystem::Local`); this wrapper is the one
/// place the editor's `Remote(endpoint)` identity is rewritten to the
/// worker's local spelling on the way out and restored on the way
/// back. Transport selection never enters the kernel, and a location
/// for any other namespace is a typed client failure, never a guessed
/// aliasing.
#[derive(Clone)]
pub struct RemoteWorker {
    endpoint: RemoteEndpoint,
    worker: Worker,
}

impl RemoteWorker {
    /// A lease on the deployed worker at `object` on `endpoint` (see
    /// [`worker`]); the handshake binds the endpoint's admitted
    /// `target`.
    pub fn connect(endpoint: &RemoteEndpoint, object: &str, target: &str) -> Self {
        Self {
            endpoint: endpoint.clone(),
            worker: worker(endpoint, object, target),
        }
    }

    /// In-process codec transport for consumer tests: the real worker
    /// serve loop, without requiring an SSH daemon or a second policy.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(endpoint: &RemoteEndpoint, worker: Worker) -> Self {
        Self {
            endpoint: endpoint.clone(),
            worker,
        }
    }

    /// The untranslated lease for location-free families (subscribe,
    /// unsubscribe, health, exec) and lease observation.
    pub fn worker(&self) -> &Worker {
        &self.worker
    }

    /// The exact SSH endpoint whose namespace this lease serves.
    pub fn endpoint(&self) -> &RemoteEndpoint {
        &self.endpoint
    }

    /// One editor-side location in the worker's own spelling. Any
    /// namespace but this endpoint's is a routing bug, surfaced typed.
    fn to_worker(&self, location: &ResourceLocation) -> Result<ResourceLocation, ClientError> {
        if location.filesystem == Filesystem::Remote(self.endpoint.clone()) {
            return Ok(ResourceLocation::local(location.path.clone()));
        }
        Err(ClientError::Protocol(
            strop_worker_protocol::ProtocolError::Unexpected {
                message: format!(
                    "location in {} routed to the {} worker",
                    location.filesystem.label(),
                    self.endpoint
                ),
            },
        ))
    }

    /// Restore the editor-side namespace on one worker-reported
    /// location. The worker only ever reports its native spelling.
    fn to_editor(&self, location: &ResourceLocation) -> ResourceLocation {
        ResourceLocation {
            filesystem: Filesystem::Remote(self.endpoint.clone()),
            path: location.path.clone(),
        }
    }

    fn intent_to_worker(&self, intent: &OperationIntent) -> Result<OperationIntent, ClientError> {
        let mut translated = intent.clone();
        translated.source = intent
            .source
            .as_ref()
            .map(|source| self.to_worker(source))
            .transpose()?;
        translated.destination = intent
            .destination
            .as_ref()
            .map(|destination| self.to_worker(destination))
            .transpose()?;
        Ok(translated)
    }

    fn observed_from_worker(&self, observation: &LocatedObservation) -> LocatedObservation {
        LocatedObservation {
            location: self.to_editor(&observation.location),
            value: observation.value.clone(),
        }
    }

    fn operation_from_worker(&self, operation: &PreparedOperation) -> PreparedOperation {
        let mut translated = operation.clone();
        translated.intent = self.intent_from_worker(&operation.intent);
        translated.source = operation
            .source
            .as_ref()
            .map(|source| self.observed_from_worker(source));
        translated.destination = operation
            .destination
            .as_ref()
            .map(|destination| self.observed_from_worker(destination));
        translated.parents = operation
            .parents
            .iter()
            .map(|parent| self.observed_from_worker(parent))
            .collect();
        translated.capability.trash_root = operation
            .capability
            .trash_root
            .as_ref()
            .map(|root| self.to_editor(root));
        translated
    }

    fn operation_to_worker(
        &self,
        operation: &PreparedOperation,
    ) -> Result<PreparedOperation, ClientError> {
        let mut translated = operation.clone();
        translated.intent = self.intent_to_worker(&operation.intent)?;
        let observation = |source: &LocatedObservation| {
            Ok::<_, ClientError>(LocatedObservation {
                location: self.to_worker(&source.location)?,
                value: source.value.clone(),
            })
        };
        translated.source = operation.source.as_ref().map(observation).transpose()?;
        translated.destination = operation
            .destination
            .as_ref()
            .map(observation)
            .transpose()?;
        translated.parents = operation
            .parents
            .iter()
            .map(observation)
            .collect::<Result<Vec<_>, _>>()?;
        translated.capability.trash_root = operation
            .capability
            .trash_root
            .as_ref()
            .map(|root| self.to_worker(root))
            .transpose()?;
        Ok(translated)
    }

    fn intent_from_worker(&self, intent: &OperationIntent) -> OperationIntent {
        let mut translated = intent.clone();
        translated.source = intent.source.as_ref().map(|source| self.to_editor(source));
        translated.destination = intent
            .destination
            .as_ref()
            .map(|destination| self.to_editor(destination));
        translated
    }

    fn outcome_from_worker(&self, outcome: &StepOutcome) -> StepOutcome {
        match outcome {
            StepOutcome::Committed {
                source_after,
                destination_after,
                recovery,
                warnings,
                publication,
            } => StepOutcome::Committed {
                source_after: source_after.clone(),
                destination_after: destination_after.clone(),
                recovery: recovery.as_ref().map(|root| self.to_editor(root)),
                warnings: warnings.clone(),
                publication: *publication,
            },
            StepOutcome::Unconfirmed {
                detail,
                observed_destination,
                recovery,
                publication,
            } => StepOutcome::Unconfirmed {
                detail: detail.clone(),
                observed_destination: observed_destination.clone(),
                recovery: recovery.as_ref().map(|root| self.to_editor(root)),
                publication: *publication,
            },
            other => other.clone(),
        }
    }

    fn receipt_from_worker(&self, receipt: &StepReceipt) -> StepReceipt {
        StepReceipt {
            step: receipt.step,
            operation: self.operation_from_worker(&receipt.operation),
            outcome: self.outcome_from_worker(&receipt.outcome),
        }
    }

    fn receipt_to_worker(&self, attempt: StepReceipt) -> Result<StepReceipt, ClientError> {
        Ok(StepReceipt {
            step: attempt.step,
            operation: self.operation_to_worker(&attempt.operation)?,
            outcome: match &attempt.outcome {
                StepOutcome::Committed {
                    source_after,
                    destination_after,
                    recovery,
                    warnings,
                    publication,
                } => StepOutcome::Committed {
                    source_after: source_after.clone(),
                    destination_after: destination_after.clone(),
                    recovery: recovery
                        .as_ref()
                        .map(|path| self.to_worker(path))
                        .transpose()?,
                    warnings: warnings.clone(),
                    publication: *publication,
                },
                StepOutcome::Unconfirmed {
                    detail,
                    observed_destination,
                    recovery,
                    publication,
                } => StepOutcome::Unconfirmed {
                    detail: detail.clone(),
                    observed_destination: observed_destination.clone(),
                    recovery: recovery
                        .as_ref()
                        .map(|path| self.to_worker(path))
                        .transpose()?,
                    publication: *publication,
                },
                other => other.clone(),
            },
        })
    }

    /// Bounded stat-class observation of exact resources.
    pub fn observe(
        &self,
        token: &strop_core::worker::CancelToken,
        locations: Vec<ResourceLocation>,
    ) -> Result<Vec<LocatedObservation>, ClientError> {
        let locations = locations
            .iter()
            .map(|location| self.to_worker(location))
            .collect::<Result<Vec<_>, _>>()?;
        let observations = self.worker.observe(token, locations)?;
        Ok(observations
            .iter()
            .map(|observation| self.observed_from_worker(observation))
            .collect())
    }

    /// One bounded listing; the snapshot's namespace is restored to
    /// this endpoint's identity.
    pub fn list(
        &self,
        token: &strop_core::worker::CancelToken,
        location: ResourceLocation,
    ) -> Result<strop_workspace::DirectorySnapshot, ClientError> {
        let location = self.to_worker(&location)?;
        let mut snapshot = self.worker.list(token, location)?;
        snapshot.location = self.to_editor(&snapshot.location);
        Ok(snapshot)
    }

    /// A complete or ranged read (bytes carry no namespace).
    pub fn read(
        &self,
        token: &strop_core::worker::CancelToken,
        location: ResourceLocation,
        offset: u64,
        length: Option<u64>,
    ) -> Result<strop_worker_client::ReadPayload, ClientError> {
        let location = self.to_worker(&location)?;
        self.worker.read(token, location, offset, length)
    }

    /// Prepare one batch in the endpoint's namespace. The session
    /// environment override stays `None`: the remote worker captures
    /// its own — local trash roots never cross namespaces.
    pub fn prepare(
        &self,
        token: &strop_core::worker::CancelToken,
        intents: Vec<OperationIntent>,
    ) -> Result<(Vec<PreparedOperation>, Vec<OperationRefusal>), ClientError> {
        let intents = intents
            .iter()
            .map(|intent| self.intent_to_worker(intent))
            .collect::<Result<Vec<_>, _>>()?;
        let (steps, refused) = self.worker.prepare(token, intents, None)?;
        Ok((
            steps
                .iter()
                .map(|step| self.operation_from_worker(step))
                .collect(),
            refused
                .into_iter()
                .map(|refusal| OperationRefusal {
                    intent: self.intent_from_worker(&refusal.intent),
                    failure: refusal.failure,
                })
                .collect(),
        ))
    }

    /// Apply prepared steps; receipts come back in the editor's
    /// namespace.
    pub fn apply(
        &self,
        token: &strop_core::worker::CancelToken,
        steps: Vec<PreparedOperation>,
        content: Option<&[u8]>,
    ) -> Result<Vec<StepReceipt>, ClientError> {
        let steps = steps
            .iter()
            .map(|step| self.operation_to_worker(step))
            .collect::<Result<Vec<_>, _>>()?;
        let receipts = self.worker.apply(token, steps, content)?;
        Ok(receipts
            .iter()
            .map(|receipt| self.receipt_from_worker(receipt))
            .collect())
    }

    /// Read one selection window with exactly the SFTP path's
    /// semantics (WK07 parity): observe the size, resolve the window
    /// against it, refuse unbounded allocation, trim only cut UTF-8
    /// edges — interior invalid bytes still fail validation honestly.
    pub fn read_selection(
        &self,
        token: &strop_core::worker::CancelToken,
        location: ResourceLocation,
        selection: &crate::ReadSelection,
    ) -> Result<(String, crate::RemoteWindow), RemoteReadFailure> {
        let mut observations = self.observe(token, vec![location.clone()])?;
        let Some(observation) = observations.pop().and_then(|observed| observed.value) else {
            return Err(RemoteReadFailure::Missing(location.label()));
        };
        if observation.kind != strop_workspace::EntryKind::File {
            return Err(RemoteReadFailure::NotRegularFile(location.label()));
        }
        let size = observation
            .size
            .ok_or_else(|| RemoteReadFailure::UnknownLength(location.label()))?;
        let window = crate::RemoteWindow::resolve(selection, crate::RemoteSize::new(size));
        if window.length().get() > crate::ReadLimit::MAX {
            return Err(RemoteReadFailure::TooLarge {
                bytes: window.length().get(),
                max: crate::ReadLimit::MAX,
            });
        }
        let mut payload = self.read(
            token,
            location,
            window.start().get(),
            Some(window.length().get()),
        )?;
        let mut bytes = Vec::with_capacity(window.length().get() as usize);
        std::io::Read::read_to_end(&mut payload, &mut bytes)
            .map_err(|error| ClientError::WorkerLost(format!("read payload: {error}")))?;
        if bytes.len() as u64 != window.length().get() {
            return Err(RemoteReadFailure::ShortRead {
                expected: window.length().get(),
                actual: bytes.len() as u64,
            });
        }
        let (front, back) = crate::selection::utf8_boundary_range(&bytes);
        let front = if window.start().get() == 0 { 0 } else { front };
        let back = if window.reaches_eof() {
            bytes.len()
        } else {
            back
        };
        let window = window.narrowed(front as u64, (back - front) as u64);
        bytes.truncate(back);
        if front != 0 {
            bytes.drain(..front);
        }
        let text = String::from_utf8(bytes)
            .map_err(|error| RemoteReadFailure::InvalidUtf8(error.utf8_error().valid_up_to()))?;
        Ok((text, window))
    }
    /// Install one notify subscription over a scope (the scope crosses
    /// the translation boundary; events carry only the subscription
    /// identity and scope-relative path hints).
    pub fn subscribe(
        &self,
        token: &strop_core::worker::CancelToken,
        scope: ResourceLocation,
        recursive: bool,
    ) -> Result<
        (
            strop_worker_protocol::Subscription,
            strop_worker_protocol::NotifyCoverage,
        ),
        ClientError,
    > {
        let scope = self.to_worker(&scope)?;
        self.worker.subscribe(token, scope, recursive)
    }

    /// Retire one subscription by its full identity.
    pub fn unsubscribe(
        &self,
        token: &strop_core::worker::CancelToken,
        subscription: strop_worker_protocol::Subscription,
    ) -> Result<(), ClientError> {
        self.worker.unsubscribe(token, subscription)
    }

    /// Verify one uncertain completed attempt against fresh evidence.
    pub fn verify(
        &self,
        token: &strop_core::worker::CancelToken,
        attempt: StepReceipt,
    ) -> Result<strop_workspace::operation::VerifiedOutcome, ClientError> {
        self.worker.verify(token, self.receipt_to_worker(attempt)?)
    }

    /// Reconcile a frozen old-session Store with fresh read authority;
    /// the worker refuses any changed native boot/mount/principal.
    pub fn verify_recovered(
        &self,
        token: &strop_core::worker::CancelToken,
        attempt: StepReceipt,
        namespace: strop_worker_protocol::NamespaceIdentity,
    ) -> Result<strop_workspace::operation::VerifiedOutcome, ClientError> {
        self.worker
            .verify_recovered(token, self.receipt_to_worker(attempt)?, namespace)
    }
}

/// What a worker-routed remote read failed with: transport failures
/// carry the client's taxonomy, and window/UTF-8 facts stay typed just
/// as on the SFTP path.
#[derive(Debug, thiserror::Error)]
pub enum RemoteReadFailure {
    #[error(transparent)]
    Client(#[from] ClientError),
    /// The resource vanished or stopped being a regular file.
    #[error("remote file missing: {0}")]
    Missing(String),
    /// The observed kind is not a regular file.
    #[error("not a regular file: {0}")]
    NotRegularFile(String),
    /// The observation reported no length.
    #[error("unknown file length: {0}")]
    UnknownLength(String),
    #[error("{bytes} bytes exceeds the {max}-byte snapshot cap; use a bounded tail or range")]
    TooLarge { bytes: u64, max: u64 },
    #[error("invalid UTF-8 at byte {0}")]
    InvalidUtf8(usize),
    #[error("captured {expected} bytes but {actual} arrived before the captured end")]
    ShortRead { expected: u64, actual: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use strop_worker_protocol::{Capabilities, Limits, NamespaceIdentity, NotifyCoverage};

    fn client_info() -> EndpointInfo {
        EndpointInfo {
            name: "strop".into(),
            version: "0.35.0".into(),
            build: None,
            target: "fixture".into(),
        }
    }

    #[test]
    fn the_worker_line_is_one_quoted_word_plus_the_fixed_flag() {
        assert_eq!(
            worker_line("/home/alice/.cache/strop-worker/objects/ab12"),
            "'/home/alice/.cache/strop-worker/objects/ab12' --worker-stdio"
        );
        assert_eq!(
            worker_line("/opt/strop/worker's strop"),
            "'/opt/strop/worker'\\''s strop' --worker-stdio"
        );
    }

    #[test]
    fn a_welcome_round_trips_the_reported_facts() {
        let (server_read, mut client_write) = std::io::pipe().expect("pipe");
        let (client_read, server_write) = std::io::pipe().expect("pipe");
        let session = Session {
            incarnation: 7,
            lease: strop_worker_protocol::LeaseId(9),
        };
        let worker = EndpointInfo {
            name: "strop".into(),
            version: "0.35.0".into(),
            build: None,
            target: "x86_64-unknown-linux-musl".into(),
        };
        let answering = {
            let worker = worker.clone();
            thread::spawn(move || {
                let mut decoder = FrameDecoder::new();
                let mut reader = std::io::BufReader::new(server_read);
                let body = frame::read_frame(&mut reader, &mut decoder)
                    .expect("frame")
                    .expect("hello frame");
                match codec::decode_body::<ClientMessage>(&body).expect("hello decodes") {
                    Incoming::Envelope(ClientMessage::Hello { protocol, .. }) => {
                        assert_eq!(protocol, PROTOCOL_VERSION);
                    }
                    other => panic!("expected hello, got {other:?}"),
                }
                let mut writer = server_write;
                codec::write_envelope(
                    &mut writer,
                    &WorkerMessage::Welcome {
                        protocol: PROTOCOL_VERSION,
                        worker,
                        session,
                        namespace: NamespaceIdentity {
                            identity: "fixture".into(),
                            principal: Some(1000),
                        },
                        limits: Limits {
                            max_frame_bytes: 0,
                            max_chunk_bytes: 0,
                            max_pending_requests: 0,
                            max_batch_steps: 0,
                            max_listing_entries: 0,
                            max_subscriptions: 0,
                            max_streams: 0,
                            max_exec_processes: 0,
                            max_concurrent_reads: 0,
                            control_reserve: 0,
                            max_queued_data_chunks: 0,
                        },
                        capabilities: Capabilities {
                            observe: false,
                            list: false,
                            read: false,
                            write: false,
                            trash: false,
                            notify: NotifyCoverage::Unsupported,
                            exec_finite: false,
                            exec_service: false,
                            pty: false,
                        },
                    },
                )
                .expect("welcome writes");
            })
        };
        let facts = handshake_inner(&mut client_write, client_read, &client_info())
            .expect("welcome round-trips");
        answering.join().expect("answerer finishes");
        assert_eq!(facts.protocol, PROTOCOL_VERSION);
        assert_eq!(facts.worker.target, "x86_64-unknown-linux-musl");
        assert_eq!(facts.session, session);
    }

    #[test]
    fn shell_diagnostics_can_never_impersonate_a_welcome() {
        let (_server_read, mut client_write) = std::io::pipe().expect("pipe");
        let (client_read, mut server_write) = std::io::pipe().expect("pipe");
        server_write
            .write_all(b"sh: 1: /cache/objects/ab12: Permission denied\n")
            .expect("diagnostic writes");
        drop(server_write);
        let error = handshake_inner(&mut client_write, client_read, &client_info())
            .expect_err("shell text is never a welcome");
        assert!(matches!(error, WorkerTransportError::Handshake(_)));
    }
}
