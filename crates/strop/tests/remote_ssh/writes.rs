use super::*;
use std::os::unix::fs::MetadataExt;

fn required() -> bool {
    std::env::var_os("STROP_REQUIRE_SSH_TESTS").as_deref() == Some(std::ffi::OsStr::new("1"))
}

#[test]
fn remote_edit_save_refresh_preserves_native_path_mode_and_time() {
    if !required() {
        return;
    }
    let fixture = Fixture::new();
    let path = fixture
        .root()
        .join(std::ffi::OsStr::from_bytes(b"edit \xff.txt"));
    std::fs::write(&path, "before\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let before = std::fs::metadata(&path).unwrap();
    let script = fixture.script("write.steps", "settle\nkeys :remote edit<cr>\nsettle\nkeys ggciwAFTER<esc>:w<cr>\nsettle\nstate\nkeys :e!<cr>\nsettle\nstate\nframe\n");
    let trace = fixture.root().join("write.jsonl");
    let output = fixture.run(&[
        "--headless".into(),
        script.into(),
        uri("fixture", &path).into(),
        "--log-file".into(),
        trace.clone().into(),
        "--log-content".into(),
    ]);
    assert_eq!(std::fs::read(&path).unwrap(), b"AFTER\n");
    let after = std::fs::metadata(&path).unwrap();
    assert_eq!(after.mode() & 0o7777, 0o640);
    assert_eq!(
        (after.mtime(), after.mtime_nsec()),
        (before.mtime(), before.mtime_nsec())
    );
    assert!(states(&output).iter().all(|state| state["dirty"] == false));
    assert!(output.contains("AFTER") && output.contains("[RO]"));
    std::fs::write(fixture.root().join("bin/ssh"), "#!/bin/sh\nexit 99\n").unwrap();
    fixture.run(&["--replay".into(), trace.into()]);
}

#[test]
fn remote_conflict_keeps_local_edits_and_write_quit_target_cannot_overwrite_original() {
    if !required() {
        return;
    }
    let fixture = Fixture::new();
    let path = fixture.root().join("file.txt");
    std::fs::write(&path, "aaaa\n").unwrap();
    let target = uri("fixture", &path);
    // A local document is the nonparticipating writer in this inetd fixture.
    // Its completed same-size write must conflict with the retained remote base.
    let script = format!("settle\nkeys :remote edit<cr>\nsettle\nkeys :e {}<cr>\nsettle\nkeys ggciwbbbb<esc>:w<cr>\nsettle\nkeys :e {target}<cr>\nkeys ggciwcccc<esc>:w!<cr>\nsettle\nstate\nframe\n", path.display());
    let output = fixture.open(&target, &script);
    assert_eq!(std::fs::read(&path).unwrap(), b"bbbb\n");
    assert_eq!(states(&output).last().unwrap()["dirty"], true);
    assert!(output.contains("cccc"));
    let export = fixture.root().join("should-not-exist");
    let output = fixture.open(
        &target,
        &format!(
            "settle\nkeys :remote edit<cr>\nsettle\nkeys ggciwmine<esc>:wq {}<cr>\nsettle\nstate\n",
            export.display()
        ),
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"bbbb\n");
    assert!(!export.exists());
    assert_eq!(states(&output).last().unwrap()["dirty"], true);
}
