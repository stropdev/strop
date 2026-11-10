use super::*;
use std::os::unix::fs::MetadataExt;

#[test]
fn remote_edit_and_save_keep_the_native_path_and_mode() {
    if !ssh_required("remote_edit_and_save_keep_the_native_path_and_mode") {
        return;
    }
    let fixture = Fixture::new();
    let path = fixture
        .root()
        .join(std::ffi::OsStr::from_bytes(b"edit \xff.txt"));
    std::fs::write(&path, "before\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let script = fixture.script(
        "write.steps",
        "settle\nkeys ggciwAFTER<esc>:w<cr>\nsettle\nstate\nkeys :e!<cr>\nsettle\nstate\nframe\n",
    );
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
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o640);
    let observed = states(&output);
    assert_eq!(observed[0]["message"], "written", "{output}");
    assert!(observed.iter().all(|state| state["dirty"] == false));
    assert!(output.contains("AFTER"));
    // No temporary is left beside the file.
    let names: Vec<_> = std::fs::read_dir(fixture.root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.as_bytes().starts_with(b".edit"))
        .collect();
    assert!(names.is_empty(), "{names:?}");
    std::fs::write(fixture.root().join("bin/ssh"), "#!/bin/sh\nexit 99\n").unwrap();
    fixture.run(&["--replay".into(), trace.into()]);
}

#[test]
fn a_file_changed_on_the_host_is_refused_until_forced_and_save_as_never_clobbers() {
    if !ssh_required(
        "a_file_changed_on_the_host_is_refused_until_forced_and_save_as_never_clobbers",
    ) {
        return;
    }
    let fixture = Fixture::new();
    let path = fixture.root().join("file.txt");
    std::fs::write(&path, "aaaa\n").unwrap();
    let target = uri("fixture", &path);
    // A local document is the other writer in this inetd fixture: its
    // completed rewrite (another size) must be refused by the remote `:w`
    // until `:w!`.
    let script = format!(
        "settle\nkeys :e {}<cr>\nsettle\nkeys ggciwbbbbbb<esc>:w<cr>\nsettle\nkeys :e {target}<cr>\nsettle\nkeys ggciwcccc<esc>:w<cr>\nsettle\nstate\nkeys :w!<cr>\nsettle\nstate\n",
        path.display()
    );
    let output = fixture.open(&target, &script);
    let observed = states(&output);
    assert!(
        observed[0]["message"]
            .as_str()
            .unwrap()
            .contains("changed on disk"),
        "{output}"
    );
    assert_eq!(observed[0]["dirty"], true);
    // The message after `:w!` may already be the local copy's reload
    // notice (the same file is open locally): the outcome is the state.
    assert_eq!(observed[1]["dirty"], false, "{output}");
    assert_eq!(std::fs::read(&path).unwrap(), b"cccc\n");

    // `:w other` writes a copy on the host and never replaces a file.
    let copy = fixture.root().join("copy.txt");
    let output = fixture.open(
        &target,
        "settle\nkeys ggciwmine<esc>:w copy.txt<cr>\nsettle\nstate\nkeys ggciwagain<esc>:w copy.txt<cr>\nsettle\nstate\n",
    );
    let observed = states(&output);
    assert_eq!(observed[0]["dirty"], true, "a copy leaves the buffer dirty");
    assert!(
        observed[1]["message"]
            .as_str()
            .unwrap()
            .contains("already exists"),
        "{output}"
    );
    assert_eq!(std::fs::read(&copy).unwrap(), b"mine\n", "{output}");
    assert_eq!(std::fs::read(&path).unwrap(), b"cccc\n");
}
