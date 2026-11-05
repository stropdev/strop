#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;
use strop_containers::devcontainer::{
    discover, AttachRequest, CliEngine, ConfigSelection, JobState, LockfilePolicy, RebuildRequest,
    ReferenceCli, StartRequest,
};
use strop_core::worker::CancelToken;

struct Cleanup {
    root: PathBuf,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let output = Command::new("docker")
            .args([
                "ps",
                "-aq",
                "--filter",
                &format!("label=devcontainer.local_folder={}", self.root.display()),
            ])
            .output();
        if let Ok(output) = output {
            for id in String::from_utf8_lossy(&output.stdout).lines() {
                let _ = Command::new("docker").args(["rm", "-f", id]).output();
            }
        }
    }
}

fn required() -> Option<PathBuf> {
    if std::env::var_os("STROP_CONTAINER_TESTS").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return None;
    }
    std::env::var_os("STROP_DEVCONTAINER_CLI").map(PathBuf::from)
}

#[test]
fn reference_cli_start_attach_and_rebuild_use_verified_current_environment() {
    let Some(executable) = required() else {
        eprintln!("skipping live devcontainer CLI journey: STROP_DEVCONTAINER_CLI is unset");
        return;
    };
    let project = tempfile::tempdir().unwrap();
    let _cleanup = Cleanup {
        root: project.path().to_path_buf(),
    };
    let configuration = project.path().join(".devcontainer/devcontainer.json");
    std::fs::create_dir_all(configuration.parent().unwrap()).unwrap();
    std::fs::write(
        &configuration,
        r#"{
          "image": "busybox",
          "workspaceFolder": "/workspaces/project",
          "workspaceMount": "type=bind,source=${localWorkspaceFolder},target=/workspaces/project",
          "onCreateCommand": "touch /tmp/strop-live-hook"
        }"#,
    )
    .unwrap();
    let found = discover(project.path())
        .unwrap()
        .configurations
        .into_iter()
        .next()
        .unwrap();
    let selection = ConfigSelection::new(&found, CliEngine::DefaultLocal);
    let (token, _guard) = CancelToken::standalone();
    let cli = ReferenceCli::probe(Some(&executable), &token).unwrap();
    let started = cli
        .start(StartRequest::new(
            selection.clone(),
            LockfilePolicy::Disabled,
        ))
        .unwrap()
        .run();
    assert_eq!(started.state, JobState::Succeeded, "{:?}", started.error);
    let context = started.context.clone().unwrap();
    let receipt = started.receipt.unwrap();
    assert_eq!(context.workspace.remote_folder, "/workspaces/project");
    assert!(Command::new("docker")
        .args([
            "exec",
            &context.container_id,
            "test",
            "-f",
            "/tmp/strop-live-hook"
        ])
        .status()
        .unwrap()
        .success());

    let attached = cli
        .attach(
            AttachRequest::new(selection.clone())
                .owned(&receipt)
                .unwrap(),
        )
        .unwrap()
        .run();
    assert_eq!(attached.state, JobState::Succeeded, "{:?}", attached.error);
    assert_eq!(attached.context.unwrap().container_id, context.container_id);

    let rebuilt = cli
        .rebuild(RebuildRequest::new(selection, LockfilePolicy::Disabled, &receipt).unwrap())
        .unwrap()
        .run();
    assert_eq!(rebuilt.state, JobState::Succeeded, "{:?}", rebuilt.error);
    let rebuilt_context = rebuilt.context.unwrap();
    assert_ne!(rebuilt_context.container_id, context.container_id);
    assert_eq!(
        rebuilt_context.supersedes.as_deref(),
        Some(context.container_id.as_str())
    );
}
