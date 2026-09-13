use super::*;
use std::path::Path;

fn helper(request: Value, body: &[u8], fault: &str) -> Value {
    let mut header = serde_json::to_vec(&request).unwrap();
    header.push(b'\n');
    let body = body.to_vec();
    let source = format!("{fault}\n{HELPER}");
    crate::test_support::in_worker(move |token| {
        let mut command = std::process::Command::new("python3");
        command.args(["-I", "-S", "-c"]).arg(source);
        let chunks = [header.as_slice(), body.as_slice()];
        let output = strop_core::process::capture_with(
            &mut command,
            &token,
            &strop_core::process::CapturePolicy {
                stdin: strop_core::process::StdinPolicy::HeldInput(&chunks),
                deadline: std::time::Duration::from_secs(10),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    })
}
fn plan(
    kind: OperationKind,
    source: Option<&Path>,
    destination: Option<&Path>,
    buffer: bool,
) -> Value {
    fn encode(path: Option<&Path>) -> Option<&[u8]> {
        path.map(strop_workspace::addr::uri::path_bytes)
    }
    let input = json!({"version":1,"action":"prepare","kind":kind,"source":encode(source),"destination":encode(destination),"buffer_copy":buffer,"allow_occupied":false});
    let reply = helper(input.clone(), &[], "");
    assert!(reply.get("value").is_some(), "{reply}");
    let prepared = &reply["value"];
    json!({"version":1,"action":"apply","kind":kind,"source":prepared["source"]["path"],"destination":prepared["destination"]["path"],
        "logical_source":input["source"],"logical_destination":input["destination"],"before_source":prepared["source"]["value"],
        "before_destination":prepared["destination"]["value"],"parents":prepared["parents"],"capability":prepared["capability"],"buffer_copy":buffer,"vacated":false})
}
fn committed(reply: Value) -> StepOutcome {
    let outcome: StepOutcome =
        serde_json::from_value(reply["value"].clone()).unwrap_or_else(|_| panic!("{reply}"));
    assert!(outcome.is_committed(), "{outcome:?}");
    outcome
}

#[test]
fn rename_verification_requires_the_owned_after_version() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::write(&source, "owned\n").unwrap();
    let mut request = plan(OperationKind::Rename, Some(&source), Some(&target), false);
    let reply = helper(request.clone(), &[], "");
    committed(reply.clone());
    request["action"] = json!("verify");
    request["observed_destination"] = reply["value"]["Committed"]["destination_after"].clone();
    let outcome: VerifiedOutcome =
        serde_json::from_value(helper(request.clone(), &[], "")["value"].clone()).unwrap();
    assert!(matches!(outcome, VerifiedOutcome::Unknown { .. }));
    request["publication"] = reply["value"]["Committed"]["publication"].clone();
    let outcome: VerifiedOutcome =
        serde_json::from_value(helper(request, &[], "")["value"].clone()).unwrap();
    assert!(matches!(outcome, VerifiedOutcome::Committed(_)));
}

#[test]
fn lock_only_directory_removal_closes_unlinked_locks_before_rmdir() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("directory");
    std::fs::create_dir(&directory).unwrap();
    let child = directory.join("child");
    committed(helper(
        plan(OperationKind::CreateFile, None, Some(&child), false),
        &[],
        "",
    ));
    committed(helper(
        plan(OperationKind::Remove, Some(&child), None, false),
        &[],
        "",
    ));
    let fault = r#"import os, errno
real_rmdir = os.rmdir
def nfs_rmdir(path, *args, **kwargs):
    for entry in os.listdir('/proc/self/fd'):
        try:
            held = os.readlink('/proc/self/fd/' + entry)
        except FileNotFoundError:
            continue
        if '.strop-lock-' in held and held.endswith(' (deleted)'):
            raise OSError(errno.ENOTEMPTY, 'simulated NFS open-lock retention')
    return real_rmdir(path, *args, **kwargs)
os.rmdir = nfs_rmdir
"#;
    committed(helper(
        plan(OperationKind::Remove, Some(&directory), None, false),
        &[],
        fault,
    ));
    assert!(!directory.exists());
}

#[test]
fn directory_removal_preserves_an_active_cooperative_lock() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("directory");
    std::fs::create_dir(&directory).unwrap();
    let child = directory.join("child");
    committed(helper(
        plan(OperationKind::CreateFile, None, Some(&child), false),
        &[],
        "",
    ));
    committed(helper(
        plan(OperationKind::Remove, Some(&child), None, false),
        &[],
        "",
    ));
    let lock_path = std::fs::read_dir(&directory)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    lock.try_lock().unwrap();
    let reply = helper(
        plan(OperationKind::Remove, Some(&directory), None, false),
        &[],
        "",
    );
    assert_eq!(reply["error"]["kind"], "busy", "{reply}");
    assert!(lock_path.exists());
    drop(lock);
    committed(helper(
        plan(OperationKind::Remove, Some(&directory), None, false),
        &[],
        "",
    ));
    assert!(!directory.exists());
}

#[test]
fn occupied_and_changed_sources_never_overwrite_or_remove() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::write(&source, "original").unwrap();
    let rename = plan(OperationKind::Rename, Some(&source), Some(&target), false);
    std::fs::write(&target, "other actor").unwrap();
    let refused = helper(rename, &[], "");
    assert_eq!(refused["error"]["kind"], "conflict");
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "original");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "other actor");
    let remove = plan(OperationKind::Remove, Some(&source), None, false);
    std::fs::write(&source, "new contents").unwrap();
    assert_eq!(helper(remove, &[], "")["error"]["kind"], "conflict");
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "new contents");
}

#[test]
fn native_name_copy_uses_the_explicit_content_version() {
    use std::os::unix::ffi::OsStringExt;
    let root = tempfile::tempdir().unwrap();
    let source = root
        .path()
        .join(std::ffi::OsString::from_vec(b"ssh:literal\n\xff".to_vec()));
    let stored = root.path().join("stored");
    let buffer = root.path().join("buffer");
    std::fs::write(&source, "stored bytes").unwrap();
    committed(helper(
        plan(OperationKind::Copy, Some(&source), Some(&stored), false),
        &[],
        "",
    ));
    let text = b"unsaved buffer bytes";
    let mut copy = plan(OperationKind::Copy, Some(&source), Some(&buffer), true);
    let digest: [u8; 32] = Sha256::digest(text).into();
    copy["length"] = json!(text.len());
    copy["digest"] = json!(digest);
    committed(helper(copy, text, ""));
    assert_eq!(std::fs::read(&source).unwrap(), b"stored bytes");
    assert_eq!(std::fs::read(&stored).unwrap(), b"stored bytes");
    assert_eq!(std::fs::read(&buffer).unwrap(), text);
    assert!(!std::fs::read_dir(root.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .as_encoded_bytes()
        .starts_with(b".strop-fs-")));
}

#[test]
fn post_publication_failure_retains_verifiable_object_evidence() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("created");
    let mut request = plan(OperationKind::CreateFile, None, Some(&target), false);
    let fault = "import os, stat, errno\nreal_sync = os.fsync\ndef failed_directory_sync(fd):\n    if stat.S_ISDIR(os.fstat(fd).st_mode):\n        raise OSError(errno.EIO, 'injected synchronization failure')\n    return real_sync(fd)\nos.fsync = failed_directory_sync";
    let reply = helper(request.clone(), &[], fault);
    assert_eq!(reply["error"]["unconfirmed"], true);
    assert!(target.is_file());
    request["action"] = json!("verify");
    request["observed_destination"] = reply["error"]["observed_destination"].clone();
    let unknown: VerifiedOutcome =
        serde_json::from_value(helper(request.clone(), &[], "")["value"].clone()).unwrap();
    assert!(matches!(unknown, VerifiedOutcome::Unknown { .. }));
    request["publication"] = reply["error"]["publication"].clone();
    let verified: VerifiedOutcome =
        serde_json::from_value(helper(request, &[], "")["value"].clone()).unwrap();
    assert!(matches!(verified, VerifiedOutcome::Committed(_)));
}
