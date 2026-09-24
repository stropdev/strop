use super::*;
use serde_json::{json, Value};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

fn fixture_directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap())
        .unwrap()
}

fn helper(
    path: &Path,
    request: Value,
    body: &[u8],
    fault: &str,
) -> strop_core::process::CommandOutput {
    let path = path.to_owned();
    let mut header = serde_json::to_vec(&request).unwrap();
    header.push(b'\n');
    let body = body.to_vec();
    let source = format!("{fault}\n{}", protocol::HELPER);
    crate::test_support::in_worker(move |token| {
        let mut command = std::process::Command::new("python3");
        command.args(["-I", "-S", "-c"]).arg(source).arg(path);
        let chunks = [header.as_slice(), body.as_slice()];
        let policy = strop_core::process::CapturePolicy {
            stdin: strop_core::process::StdinPolicy::HeldInput(&chunks),
            deadline: std::time::Duration::from_secs(10),
            ..Default::default()
        };
        strop_core::process::capture_with(&mut command, &token, &policy).unwrap()
    })
}
fn result(output: &strop_core::process::CommandOutput) -> Value {
    serde_json::from_slice::<Value>(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)))
        ["result"]
        .clone()
}
fn request(operation: &str, text: &str, before: Option<Value>) -> Value {
    let digest = ContentDigest::of(&Rope::from_str(text));
    let mut value =
        json!({"version": 1, "operation": operation, "length": text.len(), "digest": digest});
    if let Some(before) = before {
        value["before"] = before;
    }
    value
}
fn baseline(path: &Path, text: &str) -> Value {
    let output = helper(path, request("edit", text, None), b"", "");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    result(&output)["stamp"].clone()
}
fn private_stages(directory: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .as_encoded_bytes()
                .starts_with(b".strop-save-")
        })
        .collect()
}

#[test]
fn save_preserves_permissions_time_and_conflicts_on_same_size_changes() {
    let directory = fixture_directory();
    let path = directory.path().join("file");
    std::fs::write(&path, "old\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let attributes = std::process::Command::new("python3")
        .args([
            "-I",
            "-S",
            "-c",
            "import os,sys; os.setxattr(sys.argv[1], b'user.strop_test', b'preserved attribute')",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        attributes.status.success(),
        "{}",
        String::from_utf8_lossy(&attributes.stderr)
    );
    let original = std::fs::metadata(&path).unwrap();
    let before = baseline(&path, "old\n");
    let output = helper(
        &path,
        request("save", "new\n", Some(before.clone())),
        b"new\n",
        "",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"new\n");
    let metadata = std::fs::metadata(&path).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o7777, 0o640);
    assert_eq!(
        (metadata.mtime(), metadata.mtime_nsec()),
        (original.mtime(), original.mtime_nsec())
    );
    assert!(private_stages(directory.path()).is_empty());
    let attributes = std::process::Command::new("python3")
        .args([
            "-I",
            "-S",
            "-c",
            "import os,sys; sys.stdout.buffer.write(os.getxattr(sys.argv[1], b'user.strop_test'))",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(attributes.status.success());
    assert_eq!(attributes.stdout, b"preserved attribute");
    let saved = result(&output)["stamp"].clone();
    std::fs::write(&path, "else").unwrap();
    std::fs::File::open(&path)
        .unwrap()
        .set_modified(metadata.modified().unwrap())
        .unwrap();
    let conflict = helper(&path, request("save", "mine", Some(saved)), b"mine", "");
    assert!(!conflict.status.success());
    assert_eq!(result(&conflict)["kind"], "conflict");
    assert_eq!(std::fs::read(&path).unwrap(), b"else");
}

#[test]
fn symlinks_hardlinks_and_protocol_namespace_cannot_gain_write_authority() {
    let directory = fixture_directory();
    let original = directory.path().join("original");
    std::fs::write(&original, "safe").unwrap();
    let symlink = directory.path().join("link");
    std::os::unix::fs::symlink(&original, &symlink).unwrap();
    let linked = helper(&symlink, request("edit", "safe", None), b"", "");
    assert!(!linked.status.success());
    let hardlink = directory.path().join("hardlink");
    std::fs::hard_link(&original, &hardlink).unwrap();
    let hard = helper(&hardlink, request("edit", "safe", None), b"", "");
    assert_eq!(result(&hard)["kind"], "invalid_path");
    let reserved = directory.path().join(".strop-lock-user");
    std::fs::write(&reserved, "safe").unwrap();
    let control = helper(&reserved, request("edit", "safe", None), b"", "");
    assert_eq!(result(&control)["kind"], "invalid_path");
    assert_eq!(std::fs::read(&original).unwrap(), b"safe");
}

#[test]
fn symlinked_ancestor_directories_resolve_but_the_target_stays_strict() {
    // Enterprise NFS layout: /home/user is a symlink to /home24/user.
    // The no-follow walk resolves intermediate links and restarts from
    // the root; the file itself still opens O_NOFOLLOW, so a symlinked
    // FINAL component stays refused (covered above).
    let directory = fixture_directory();
    let real = directory.path().join("real");
    std::fs::create_dir_all(real.join("deep")).unwrap();
    std::fs::write(real.join("deep").join("g.txt"), "y\n").unwrap();
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let through = link.join("deep").join("g.txt");
    let before = baseline(&through, "y\n");
    let saved = helper(&through, request("save", "z\n", Some(before)), b"z\n", "");
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stdout)
    );
    assert_eq!(
        std::fs::read(real.join("deep").join("g.txt")).unwrap(),
        b"z\n"
    );
    assert!(private_stages(&real.join("deep")).is_empty());
    // A relative target resolves against the link's real directory, and
    // '..' inside a target is resolved by the kernel step by step.
    let relative = directory.path().join("relative");
    std::os::unix::fs::symlink("real", &relative).unwrap();
    let output = helper(
        &relative.join("deep").join("g.txt"),
        request("edit", "z\n", None),
        b"",
        "",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let dotted = directory.path().join("dotted");
    std::os::unix::fs::symlink("real/../real", &dotted).unwrap();
    let output = helper(
        &dotted.join("deep").join("g.txt"),
        request("edit", "z\n", None),
        b"",
        "",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    // A symlink loop is refused after a bounded number of hops, and a
    // regular file as an intermediate component is not a directory.
    let loop_a = directory.path().join("loopa");
    let loop_b = directory.path().join("loopb");
    std::os::unix::fs::symlink(&loop_b, &loop_a).unwrap();
    std::os::unix::fs::symlink(&loop_a, &loop_b).unwrap();
    let output = helper(&loop_a.join("g.txt"), request("edit", "z\n", None), b"", "");
    assert!(!output.status.success());
    assert_eq!(result(&output)["kind"], "invalid_path");
    let output = helper(
        &real.join("deep").join("g.txt").join("x"),
        request("edit", "z\n", None),
        b"",
        "",
    );
    assert!(!output.status.success());
    assert_eq!(result(&output)["kind"], "invalid_path");
}

#[test]
fn crash_after_metadata_restoration_keeps_draft_behind_private_directory() {
    let directory = fixture_directory();
    let path = directory.path().join("file");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let before = baseline(&path, "old");
    let fault = "import os, signal\ndef fail_rename(*args, **kwargs):\n    os.kill(os.getpid(), signal.SIGKILL)\nos.replace = fail_rename\n";
    let crashed = helper(
        &path,
        request("save", "private draft", Some(before)),
        b"private draft",
        fault,
    );
    assert!(!crashed.status.success());
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
    let stages = private_stages(directory.path());
    assert_eq!(stages.len(), 1);
    assert_eq!(
        std::fs::metadata(&stages[0]).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(stages[0].join("contents"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o644
    );
}

#[test]
fn postrename_lost_receipt_requires_explicit_durable_verification() {
    let directory = fixture_directory();
    let path = directory.path().join("file");
    std::fs::write(&path, "before").unwrap();
    let before = baseline(&path, "before");
    let unchanged = helper(
        &path,
        request("verify", "after", Some(before.clone())),
        b"",
        "",
    );
    assert_eq!(result(&unchanged)["status"], "unchanged");
    let fault = "import os, signal\nreplace = os.replace\ndef lose_receipt(*args, **kwargs):\n    replace(*args, **kwargs)\n    os.kill(os.getpid(), signal.SIGKILL)\nos.replace = lose_receipt\n";
    let crashed = helper(
        &path,
        request("save", "after", Some(before.clone())),
        b"after",
        fault,
    );
    assert!(!crashed.status.success());
    assert_eq!(std::fs::read(&path).unwrap(), b"after");
    let verified = helper(
        &path,
        request("verify", "after", Some(before.clone())),
        b"",
        "",
    );
    assert!(verified.status.success());
    assert_eq!(result(&verified)["status"], "written");
    std::fs::write(&path, "third party").unwrap();
    let conflict = helper(&path, request("verify", "after", Some(before)), b"", "");
    assert_eq!(result(&conflict)["kind"], "conflict");
    assert_eq!(std::fs::read(&path).unwrap(), b"third party");
}

#[test]
fn observed_termination_during_stage_write_preserves_original_and_cleans_stage() {
    let directory = fixture_directory();
    let path = directory.path().join("file");
    std::fs::write(&path, "before").unwrap();
    let before = baseline(&path, "before");
    let fault = "import os, signal\nwrite = os.write\ndef stop_during_write(fd, data):\n    written = write(fd, data[:1])\n    os.kill(os.getpid(), signal.SIGTERM)\n    return written\nos.write = stop_during_write\n";
    let cancelled = helper(
        &path,
        request("save", "after", Some(before)),
        b"after",
        fault,
    );
    assert_eq!(result(&cancelled)["kind"], "cancelled");
    assert_eq!(result(&cancelled)["unconfirmed"], false);
    assert_eq!(std::fs::read(&path).unwrap(), b"before");
    assert!(private_stages(directory.path()).is_empty());
}

#[test]
fn competing_lock_refuses_save_without_touching_the_original() {
    use std::io::{Read, Write};
    use std::process::Stdio;
    let directory = fixture_directory();
    let path = directory.path().join("file");
    std::fs::write(&path, "before").unwrap();
    let before = baseline(&path, "before");
    let lock_path = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .as_encoded_bytes()
                .starts_with(b".strop-lock-")
        })
        .unwrap();
    crate::test_support::in_worker(move |token| {
        let mut command = std::process::Command::new("python3");
        command.args(["-I", "-S", "-c", "import os,sys,fcntl; f=open(sys.argv[1],'r+b'); fcntl.flock(f,fcntl.LOCK_EX); os.write(1,b'ready'); sys.stdin.buffer.read(1)"])
            .arg(lock_path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut holder = strop_core::process::OwnedProcess::spawn(&mut command, &token).unwrap();
        let mut ready = [0u8; 5];
        holder
            .take_stdout()
            .unwrap()
            .read_exact(&mut ready)
            .unwrap();
        assert_eq!(&ready, b"ready");
        let refused = helper(&path, request("save", "after", Some(before)), b"after", "");
        assert_eq!(result(&refused)["kind"], "busy");
        assert_eq!(std::fs::read(&path).unwrap(), b"before");
        holder.take_stdin().unwrap().write_all(b"x").unwrap();
        assert!(holder.wait().unwrap().success());
    });
}

// VF09 fault-injection campaign (plans/0057 §5 "Remote saving and filesystem
// effects"): faults land at the actual semantic boundaries of the shipped
// helper — the rename syscall, the post-commit directory sync, cleanup, the
// metadata restoration and the pre-commit revalidation window. Outcomes are
// asserted typed and honest: a pre-commit failure is uncertain rather than
// "clean", a committed effect can never be reported as failed or unchanged,
// and every uncertain outcome settles only through explicit verification.

#[test]
fn failed_rename_is_uncertain_and_verifies_unchanged() {
    // The commit syscall itself fails: commit_started was already set, so the
    // honest answer is unconfirmed — never "refused, nothing happened".
    let directory = fixture_directory();
    let path = directory.path().join("file");
    std::fs::write(&path, "before").unwrap();
    let before = baseline(&path, "before");
    let fault = "import os\ndef deny_replace(*args, **kwargs):\n    raise PermissionError(13, 'simulated replace denial')\nos.replace = deny_replace\n";
    let refused = helper(
        &path,
        request("save", "after", Some(before.clone())),
        b"after",
        fault,
    );
    assert!(!refused.status.success());
    assert_eq!(result(&refused)["kind"], "permission");
    assert_eq!(result(&refused)["unconfirmed"], true);
    assert_eq!(std::fs::read(&path).unwrap(), b"before");
    assert!(private_stages(directory.path()).is_empty());
    let verified = helper(&path, request("verify", "after", Some(before)), b"", "");
    assert_eq!(result(&verified)["status"], "unchanged");
}

#[test]
fn post_commit_sync_failure_is_uncertain_and_verifies_written() {
    // The rename committed; the directory sync after it failed. The reply
    // must be uncertain (the effect is real but unacknowledged), and the
    // explicit verification settles it as written — never a blind retry.
    let directory = fixture_directory();
    let path = directory.path().join("file");
    std::fs::write(&path, "before").unwrap();
    let before = baseline(&path, "before");
    let fault = "import os\n_replace = os.replace\n_fsync = os.fsync\n_state = {'committed': False}\ndef replace_then_flag(*args, **kwargs):\n    _replace(*args, **kwargs)\n    _state['committed'] = True\ndef fsync_after_commit(fd):\n    if _state['committed']:\n        raise OSError(5, 'simulated post-commit sync failure')\n    return _fsync(fd)\nos.replace = replace_then_flag\nos.fsync = fsync_after_commit\n";
    let uncertain = helper(
        &path,
        request("save", "after", Some(before.clone())),
        b"after",
        fault,
    );
    assert!(!uncertain.status.success());
    assert_eq!(result(&uncertain)["kind"], "io");
    assert_eq!(result(&uncertain)["unconfirmed"], true);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"after",
        "the committed effect is on disk despite the failed sync"
    );
    let verified = helper(&path, request("verify", "after", Some(before)), b"", "");
    assert_eq!(result(&verified)["status"], "written");
}

#[test]
fn cleanup_failure_after_commit_cannot_report_a_clean_failure() {
    // Stage-directory removal fails after the commit: the committed save is
    // reported uncertain with the cleanup detail, the private stage remains
    // (empty, mode 0700) as evidence, and verification settles the outcome.
    let directory = fixture_directory();
    let path = directory.path().join("file");
    std::fs::write(&path, "before").unwrap();
    let before = baseline(&path, "before");
    let fault = "import os\ndef deny_rmdir(*args, **kwargs):\n    raise PermissionError(13, 'simulated cleanup denial')\nos.rmdir = deny_rmdir\n";
    let uncertain = helper(
        &path,
        request("save", "after", Some(before.clone())),
        b"after",
        fault,
    );
    assert!(!uncertain.status.success());
    let reply = result(&uncertain);
    assert_eq!(reply["kind"], "io");
    assert_eq!(reply["unconfirmed"], true);
    assert!(
        reply["detail"].as_str().unwrap().contains("cleanup"),
        "the cleanup failure is named: {reply}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"after");
    let stages = private_stages(directory.path());
    assert_eq!(stages.len(), 1, "the orphaned stage stays as evidence");
    assert_eq!(
        std::fs::metadata(&stages[0]).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    assert!(std::fs::read_dir(&stages[0]).unwrap().next().is_none());
    let verified = helper(&path, request("verify", "after", Some(before)), b"", "");
    assert_eq!(result(&verified)["status"], "written");
}

#[test]
fn external_drift_during_stage_preparation_refuses_and_preserves_the_drift() {
    // A nonparticipant rewrites the target inside the stage-preparation
    // window: the pre-commit revalidation refuses as a conflict, the stage
    // is cleaned, and the third party's bytes — not ours — remain.
    let directory = fixture_directory();
    let path = directory.path().join("file");
    std::fs::write(&path, "before").unwrap();
    let before = baseline(&path, "before");
    let fault = "import os, sys\n_fsync = os.fsync\n_drifted = [False]\ndef drift_once(fd):\n    if not _drifted[0]:\n        _drifted[0] = True\n        with open(os.fsdecode(sys.argv[1]), 'ab') as target:\n            target.write(b'drift')\n    return _fsync(fd)\nos.fsync = drift_once\n";
    let refused = helper(
        &path,
        request("save", "after", Some(before.clone())),
        b"after",
        fault,
    );
    assert!(!refused.status.success());
    assert_eq!(result(&refused)["kind"], "conflict");
    assert_eq!(result(&refused)["unconfirmed"], false);
    assert_eq!(std::fs::read(&path).unwrap(), b"beforedrift");
    assert!(private_stages(directory.path()).is_empty());
}

#[test]
fn metadata_restoration_failure_refuses_before_commit_and_cleans_the_stage() {
    // A metadata-restoration failure is pre-commit: a typed clean refusal,
    // the original byte-identical, and no private stage left behind.
    let directory = fixture_directory();
    let path = directory.path().join("file");
    std::fs::write(&path, "before").unwrap();
    let before = baseline(&path, "before");
    let fault = "import os\ndef deny_fchmod(*args, **kwargs):\n    raise PermissionError(13, 'simulated metadata denial')\nos.fchmod = deny_fchmod\n";
    let refused = helper(
        &path,
        request("save", "after", Some(before.clone())),
        b"after",
        fault,
    );
    assert!(!refused.status.success());
    assert_eq!(result(&refused)["kind"], "permission");
    assert_eq!(result(&refused)["unconfirmed"], false);
    assert_eq!(std::fs::read(&path).unwrap(), b"before");
    assert!(private_stages(directory.path()).is_empty());
    let verified = helper(&path, request("verify", "after", Some(before)), b"", "");
    assert_eq!(result(&verified)["status"], "unchanged");
}
