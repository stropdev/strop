use super::*;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use strop_core::worker::CancelToken;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("strop-devcontainer-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn write(&self, relative: &str, contents: &str) {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Reply {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    logs: Vec<(LogStream, Vec<u8>)>,
    failure: Option<ProcessFailure>,
}
impl Reply {
    fn json(code: i32, value: serde_json::Value) -> Self {
        Self {
            code: Some(code),
            stdout: format!("{value}\n"),
            stderr: String::new(),
            logs: Vec::new(),
            failure: None,
        }
    }
    fn version(version: &str) -> Self {
        Self {
            code: Some(0),
            stdout: format!("{version}\n"),
            stderr: String::new(),
            logs: Vec::new(),
            failure: None,
        }
    }
    fn failure(failure: ProcessFailure) -> Self {
        Self {
            code: None,
            stdout: String::new(),
            stderr: String::new(),
            logs: Vec::new(),
            failure: Some(failure),
        }
    }
}

struct FakeProcess {
    requests: Mutex<Vec<ProcessRequest>>,
    replies: Mutex<VecDeque<Reply>>,
}
impl FakeProcess {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            replies: Mutex::new(replies.into()),
        }
    }
    fn requests(&self) -> Vec<ProcessRequest> {
        self.requests.lock().unwrap().clone()
    }
}
impl CliProcess for FakeProcess {
    fn run(
        &self,
        request: &ProcessRequest,
        token: &CancelToken,
        log: &mut dyn FnMut(ProcessLog),
    ) -> Result<ProcessOutput, ProcessFailure> {
        self.requests.lock().unwrap().push(request.clone());
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted reply");
        for (stream, bytes) in reply.logs {
            log(ProcessLog { stream, bytes });
        }
        if token.is_cancelled() {
            return Err(ProcessFailure::Cancelled);
        }
        if let Some(failure) = reply.failure {
            return Err(failure);
        }
        Ok(ProcessOutput {
            code: reply.code,
            stdout: reply.stdout.into_bytes(),
            stderr_tail: reply.stderr.into_bytes(),
        })
    }
}

fn success(id: &str) -> serde_json::Value {
    serde_json::json!({
        "outcome": "success",
        "containerId": id,
        "remoteUser": "vscode",
        "remoteWorkspaceFolder": "/workspaces/project"
    })
}

fn discovered(root: &TestDir, json: &str) -> DiscoveredConfiguration {
    root.write(".devcontainer/devcontainer.json", json);
    discover(root.path()).unwrap().configurations.remove(0)
}

fn cli_with(
    root: &TestDir,
    replies: Vec<Reply>,
) -> (
    ReferenceCli,
    Arc<FakeProcess>,
    strop_core::worker::CancelHandle,
) {
    root.write("reference-cli", "not executed by tests");
    let runner = Arc::new(FakeProcess::new(replies));
    let (token, guard) = CancelToken::standalone();
    let cli = ReferenceCli::probe_with_runner(
        Some(&root.path().join("reference-cli")),
        runner.clone(),
        &token,
    )
    .unwrap();
    (cli, runner, guard)
}

fn arguments(request: &ProcessRequest) -> Vec<String> {
    request
        .args
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn discovery_is_bounded_ordered_and_digest_scoped() {
    let root = TestDir::new();
    root.write(
        ".devcontainer/devcontainer.json",
        r#"{"name":"primary","image":"one"}"#,
    );
    root.write(
        ".devcontainer.json",
        r#"{"name":"fallback","dockerFile":"Dockerfile"}"#,
    );
    root.write(
        ".devcontainer/zed/devcontainer.json",
        r#"{"name":"zed","image":"z"}"#,
    );
    root.write(
        ".devcontainer/alpha/devcontainer.json",
        r#"{"name":"alpha","image":"a"}"#,
    );
    root.write(
        ".devcontainer/alpha/deep/devcontainer.json",
        r#"{"name":"deep","image":"bad"}"#,
    );
    root.write(
        "nested/.devcontainer/devcontainer.json",
        r#"{"name":"nested","image":"bad"}"#,
    );
    fs::create_dir_all(root.path().join(".devcontainer/empty")).unwrap();

    let first = discover(root.path()).unwrap();
    assert!(first.issues.is_empty(), "{:?}", first.issues);
    let names = first
        .configurations
        .iter()
        .map(|configuration| configuration.configuration.name.as_deref().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["primary", "fallback", "alpha", "zed"]);
    assert_eq!(
        first.preferred().unwrap().precedence,
        DiscoveryPrecedence::DevcontainerDirectory
    );
    assert_eq!(
        first.configurations[1].precedence,
        DiscoveryPrecedence::WorkspaceRoot
    );
    assert_eq!(
        first.configurations[2].precedence,
        DiscoveryPrecedence::NamedSubfolder
    );

    let old = first.preferred().unwrap().identity.clone();
    root.write(
        ".devcontainer/devcontainer.json",
        "// trust must be reviewed again\n{\"name\":\"primary\",\"image\":\"one\"}",
    );
    let second = discover(root.path()).unwrap();
    let new = &second.preferred().unwrap().identity;
    assert_eq!(old.config_path(), new.config_path());
    assert_ne!(old.digest(), new.digest());
    assert_ne!(old, *new);
}

#[test]
fn malicious_hooks_are_reviewed_without_execution_and_without_secrets() {
    let root = TestDir::new();
    let marker = root.path().join("marker");
    let secret = "sk-test-very-secret-value";
    let json = format!(
        r#"{{
          "name": "hostile",
          "image": "example.invalid/image:latest",
          "initializeCommand": "touch {}",
          "onCreateCommand": {{"parallel": ["touch {}", "echo {secret}"]}},
          "postCreateCommand": "echo {secret}",
          "containerEnv": {{"TOKEN": "{secret}", "SSH_AUTH_SOCK": "/run/agent.sock"}},
          "mounts": ["type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock"],
          "privileged": true,
          "capAdd": ["SYS_ADMIN"],
          "securityOpt": ["seccomp=unconfined"],
          "devices": ["/dev/fuse"],
          "networkMode": "host",
          "forwardPorts": ["0.0.0.0:8080:80"]
        }}"#,
        marker.display(),
        marker.display(),
    );
    let found = discovered(&root, &json);
    let review = TrustReview::new(found.identity.clone(), &found.configuration);
    assert!(
        !marker.exists(),
        "discovery/review executed a malicious hook"
    );
    assert!(review
        .host_effects()
        .any(|effect| effect.kind == EffectKind::HostInitializeCommand));
    assert!(review
        .container_effects()
        .any(|effect| effect.kind == EffectKind::ContainerLifecycleCommand));
    assert!(review
        .effects
        .iter()
        .any(|effect| effect.kind == EffectKind::EngineSocket));
    assert!(review
        .effects
        .iter()
        .any(|effect| effect.kind == EffectKind::Privileged));
    assert!(review
        .effects
        .iter()
        .any(|effect| effect.kind == EffectKind::PortPublishing));
    assert_eq!(
        review.credentials,
        [CredentialKind::SshAgent, CredentialKind::OtherCredentials]
    );
    let exposed = format!("{review}\n{review:?}\n{:?}", found.configuration);
    assert!(!exposed.contains(secret), "{exposed}");
    assert!(!exposed.contains("touch "), "{exposed}");
    assert!(exposed.contains("command text is redacted"));
}

#[test]
fn jsonc_and_provider_shapes_remain_typed_and_inspectable() {
    let image = RawConfiguration::parse_jsonc(
        br#"{
          // comments and trailing commas are accepted
          "image": "example/image",
          "workspaceFolder": "/src",
          "runArgs": ["--network=host",],
        }"#,
        Path::new("image.json"),
        DEFAULT_INPUT_LIMIT,
    )
    .unwrap();
    assert_eq!(image.kind, ConfigurationKind::Image);
    assert_eq!(image.workspace_folder.as_deref(), Some("/src"));

    let dockerfile = RawConfiguration::parse_jsonc(
        br#"{"build":{"dockerfile":"Dockerfile.dev","context":"..","args":{"TOKEN":"hidden"}}}"#,
        Path::new("build.json"),
        DEFAULT_INPUT_LIMIT,
    )
    .unwrap();
    assert_eq!(dockerfile.kind, ConfigurationKind::Dockerfile);
    assert_eq!(dockerfile.dockerfile.as_deref(), Some("Dockerfile.dev"));
    assert!(!format!("{dockerfile:?}").contains("hidden"));

    let compose = RawConfiguration::parse_jsonc(
        br#"{"dockerComposeFile":["compose.yml","compose.dev.yml"],"service":"app","runServices":["db"]}"#,
        Path::new("compose.json"),
        DEFAULT_INPUT_LIMIT,
    )
    .unwrap();
    assert_eq!(compose.kind, ConfigurationKind::Compose);
    assert_eq!(compose.service.as_deref(), Some("app"));
    assert_eq!(compose.run_services, ["db"]);

    let invalid = RawConfiguration::parse_jsonc(
        br#"{"image":"example","dockerFile":"Dockerfile","privileged":"yes"}"#,
        Path::new("invalid.json"),
        DEFAULT_INPUT_LIMIT,
    )
    .unwrap();
    assert!(invalid.has_blocking_issues());
    assert_eq!(invalid.kind, ConfigurationKind::Unsupported);
    assert_eq!(invalid.source()["privileged"], "yes");
    assert!(invalid
        .issues
        .iter()
        .any(|issue| issue.kind == ConfigIssueKind::InvalidType));
}

#[test]
fn cli_version_is_checked_before_any_other_operation() {
    let root = TestDir::new();
    root.write("reference-cli", "fixture");
    let runner = Arc::new(FakeProcess::new(vec![Reply::version("0.86.9")]));
    let (token, _guard) = CancelToken::standalone();
    let error = ReferenceCli::probe_with_runner(
        Some(&root.path().join("reference-cli")),
        runner.clone(),
        &token,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        DevcontainerError::UnsupportedVersion { .. }
    ));
    assert_eq!(arguments(&runner.requests()[0]), ["--version"]);

    let runner = Arc::new(FakeProcess::new(vec![Reply::version("not-a-version")]));
    assert!(matches!(
        ReferenceCli::probe_with_runner(Some(&root.path().join("reference-cli")), runner, &token),
        Err(DevcontainerError::VersionProtocol)
    ));
    assert!(matches!(
        locate_cli(Some(&root.path().join("missing"))),
        Err(DevcontainerError::CliNotFound)
    ));
}

#[test]
fn read_configuration_is_structured_and_merged_resolution_is_explicit() {
    let root = TestDir::new();
    let found = discovered(
        &root,
        r#"{"image":"example","containerEnv":{"TOKEN":"secret"}}"#,
    );
    let selection = ConfigSelection::new(&found, CliEngine::DefaultLocal);
    let body = serde_json::json!({
        "configuration": {"image":"example","remoteUser":"vscode"},
        "mergedConfiguration": {"image":"example","privileged":true}
    });
    let (cli, runner, _guard) = cli_with(
        &root,
        vec![
            Reply::version("0.90.0"),
            Reply::json(0, body.clone()),
            Reply::json(0, body),
        ],
    );
    let (token, _read_guard) = CancelToken::standalone();
    let plain = cli.read_configuration(&selection, &token).unwrap();
    assert_eq!(
        plain.configuration().unwrap().kind,
        ConfigurationKind::Image
    );
    assert!(plain.merged_configuration().unwrap().unwrap().privileged);
    let merged = cli
        .read_merged_configuration(&selection, MetadataResolutionConsent::explicit(), &token)
        .unwrap();
    assert!(merged.merged_configuration().is_some());
    let requests = runner.requests();
    let plain_args = arguments(&requests[1]);
    assert!(plain_args.contains(&"read-configuration".into()));
    assert!(!plain_args.contains(&"--include-merged-configuration".into()));
    let merged_args = arguments(&requests[2]);
    assert!(merged_args.contains(&"--include-merged-configuration".into()));
    assert!(merged_args.contains(&"--include-features-configuration".into()));
}

#[test]
fn attach_start_and_rebuild_have_distinct_argv_and_receipts() {
    let root = TestDir::new();
    let found = discovered(
        &root,
        r#"{"image":"example","waitFor":"onCreateCommand","postCreateCommand":"echo later","workspaceFolder":"/src"}"#,
    );
    let selection = ConfigSelection::new(&found, CliEngine::DefaultLocal);
    let old = "a".repeat(64);
    let new = "b".repeat(64);
    let (cli, runner, _guard) = cli_with(
        &root,
        vec![
            Reply::version("0.90.0"),
            Reply::json(0, success(&old)),
            Reply::json(0, success(&new)),
            Reply::json(0, success(&new)),
        ],
    );
    let started = cli
        .start(StartRequest::new(selection.clone(), LockfilePolicy::Frozen))
        .unwrap()
        .run();
    assert!(started.is_ready());
    assert_eq!(
        started.context.as_ref().unwrap().readiness.deferred_stages,
        [LifecycleStage::PostCreate]
    );
    let receipt = started.receipt.unwrap();
    assert_eq!(receipt.coverage(), OwnershipCoverage::SingleContainer);
    let changed = ConfigSelection::new(
        &discovered(
            &root,
            r#"{"image":"example","waitFor":"onCreateCommand","postCreateCommand":"echo changed","workspaceFolder":"/src"}"#,
        ),
        CliEngine::DefaultLocal,
    );
    assert_ne!(changed.identity.digest(), receipt.identity().digest());

    let rebuilt = cli
        .rebuild(RebuildRequest::new(changed.clone(), LockfilePolicy::Frozen, &receipt).unwrap())
        .unwrap()
        .run();
    assert!(rebuilt.is_ready());
    assert_eq!(
        rebuilt.context.as_ref().unwrap().supersedes.as_deref(),
        Some(old.as_str())
    );
    assert!(matches!(
        rebuilt.reconciliation,
        Reconciliation::ReplacedOwned { .. }
    ));
    let rebuilt_receipt = rebuilt.receipt.clone().unwrap();

    let attached = cli
        .attach(AttachRequest::new(changed).owned(&rebuilt_receipt).unwrap())
        .unwrap()
        .run();
    assert!(attached.is_ready());
    assert!(attached.receipt.is_none());
    assert_eq!(
        attached.reconciliation,
        Reconciliation::ExternalResourceNotOwned
    );

    let requests = runner.requests();
    let start_args = arguments(&requests[1]);
    assert!(start_args.contains(&"--frozen-lockfile".into()));
    assert!(!start_args.contains(&"--expect-existing-container".into()));
    assert!(!start_args.contains(&"--remove-existing-container".into()));
    assert!(start_args.contains(&"--skip-non-blocking-commands".into()));
    let owner = start_args
        .iter()
        .find(|argument| argument.starts_with("strop.devcontainer.owner="))
        .expect("start pins unique ownership")
        .clone();
    let rebuild_args = arguments(&requests[2]);
    assert!(rebuild_args.contains(&"--remove-existing-container".into()));
    assert!(rebuild_args.contains(&owner));
    let attach_args = arguments(&requests[3]);
    assert!(attach_args.contains(&"--expect-existing-container".into()));
    assert!(attach_args.contains(&owner));
    assert!(!attach_args.contains(&"--remove-existing-container".into()));
    assert!(!attach_args.contains(&"--frozen-lockfile".into()));
    assert!(attach_args.contains(&"--no-lockfile".into()));
    assert!(!attach_args.iter().any(|argument| argument == "run"));
}

#[test]
fn attach_failure_never_falls_back_to_start_or_rebuild() {
    let root = TestDir::new();
    let found = discovered(&root, r#"{"image":"example"}"#);
    let selection = ConfigSelection::new(&found, CliEngine::DefaultLocal);
    let mut failure = Reply::json(
        1,
        serde_json::json!({"outcome":"error","description":"missing"}),
    );
    failure.stderr = "no matching container".into();
    let (cli, runner, _guard) = cli_with(&root, vec![Reply::version("0.90.0"), failure]);
    let outcome = cli.attach(AttachRequest::new(selection)).unwrap().run();
    assert_eq!(outcome.state, JobState::Failed);
    assert_eq!(
        outcome.reconciliation,
        Reconciliation::AttachFailedExternalResourceRetained
    );
    assert_eq!(runner.requests().len(), 2, "no implicit second operation");
}

#[test]
fn compose_receipt_does_not_invent_sidecar_ownership() {
    let root = TestDir::new();
    let found = discovered(
        &root,
        r#"{"dockerComposeFile":"compose.yml","service":"app","runServices":["db"]}"#,
    );
    let selection = ConfigSelection::new(&found, CliEngine::DefaultLocal);
    let id = "c".repeat(64);
    let (cli, _, _guard) = cli_with(
        &root,
        vec![Reply::version("0.90.0"), Reply::json(0, success(&id))],
    );
    let outcome = cli
        .start(StartRequest::new(selection, LockfilePolicy::Disabled))
        .unwrap()
        .run();
    assert_eq!(
        outcome.receipt.unwrap().coverage(),
        OwnershipCoverage::ComposeMainOnlySidecarsNotEnumerated
    );
}

#[test]
fn cancellation_is_requested_until_process_settles_and_logs_are_redacted() {
    let root = TestDir::new();
    let found = discovered(
        &root,
        r#"{"image":"example","containerEnv":{"TOKEN":"split-secret"},"onCreateCommand":"echo split-secret"}"#,
    );
    let selection = ConfigSelection::new(&found, CliEngine::DefaultLocal);
    let mut reply = Reply::failure(ProcessFailure::Cancelled);
    reply.logs = vec![
        (LogStream::Stderr, b"prefix split-".to_vec()),
        (LogStream::Stderr, b"secret suffix".to_vec()),
        (LogStream::Stdout, b"echo split-secret".to_vec()),
    ];
    let (cli, _, _guard) = cli_with(&root, vec![Reply::version("0.90.0"), reply]);
    let job = cli
        .start(StartRequest::new(
            selection,
            LockfilePolicy::GenerateOrUpdate,
        ))
        .unwrap();
    let control = job.control();
    assert_eq!(control.snapshot().state, JobState::Created);
    control.cancel();
    assert_eq!(control.snapshot().state, JobState::CancellationRequested);
    let outcome = job.run();
    assert_eq!(outcome.state, JobState::Cancelled);
    assert!(!outcome.is_ready());
    let snapshot = control.snapshot();
    assert_eq!(snapshot.state, JobState::Cancelled);
    let logs = snapshot
        .logs
        .iter()
        .map(|log| log.text.as_str())
        .collect::<String>();
    assert!(!logs.contains("split-secret"), "{logs}");
    assert!(logs.contains("<redacted>"), "{logs}");
}

#[test]
fn failed_start_returns_redacted_error_and_partial_resource_receipt() {
    let root = TestDir::new();
    let found = discovered(
        &root,
        r#"{"image":"example","remoteEnv":{"PASSWORD":"do-not-log"}}"#,
    );
    let selection = ConfigSelection::new(&found, CliEngine::DefaultLocal);
    let id = "d".repeat(64);
    let mut reply = Reply::json(
        1,
        serde_json::json!({
            "outcome":"error",
            "description":"hook failed",
            "containerId":id,
            "didStopContainer":false
        }),
    );
    reply.stderr = "token do-not-log".into();
    let (cli, _, _guard) = cli_with(&root, vec![Reply::version("0.90.0"), reply]);
    let outcome = cli
        .start(StartRequest::new(selection, LockfilePolicy::Frozen))
        .unwrap()
        .run();
    assert_eq!(outcome.state, JobState::Failed);
    let partial = outcome.partial_resource.unwrap();
    assert_eq!(partial.container_id, "d".repeat(64));
    assert_eq!(partial.cli_reported_stopped, Some(false));
    assert!(partial.ownership_label.is_some());
    let error = outcome.error.unwrap().to_string();
    assert!(!error.contains("do-not-log"), "{error}");
    assert!(error.contains("<redacted>"));
}
