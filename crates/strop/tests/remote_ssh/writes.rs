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

#[cfg(target_os = "linux")]
fn fault_interposer(fixture: &Fixture, phase: &str) {
    // Test-only syscall fault injection. The production helper and SSH/lease
    // protocol are unchanged; only this fixture basename is intercepted.
    let source = fixture.root().join("rename-fault.c");
    let library = fixture.root().join("rename-fault.so");
    std::fs::write(
        &source,
        r#"
#define _GNU_SOURCE
#include <dlfcn.h>
#include <signal.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int renameat(int olddir, const char *oldpath, int newdir, const char *newpath) {
    typedef int (*rename_fn)(int, const char *, int, const char *);
    rename_fn original = (rename_fn)dlsym(RTLD_NEXT, "renameat");
    const char *phase = getenv("STROP_SAVE_FAULT");
    int targeted = phase && strcmp(newpath, "fault-save.txt") == 0;
    if (targeted && strcmp(phase, "before") == 0) kill(getpid(), SIGKILL);
    int result = original(olddir, oldpath, newdir, newpath);
    if (targeted && result == 0 && strcmp(phase, "after") == 0) kill(getpid(), SIGKILL);
    return result;
}
"#,
    )
    .unwrap();
    successful(
        Command::new("cc")
            .args(["-shared", "-fPIC", "-O2", "-o"])
            .arg(&library)
            .arg(&source)
            .arg("-ldl"),
    );
    use std::io::Write;
    let mut config = std::fs::OpenOptions::new()
        .append(true)
        .open(fixture.root().join("sshd_config"))
        .unwrap();
    writeln!(
        config,
        "SetEnv LD_PRELOAD={} STROP_SAVE_FAULT={phase}",
        library.display()
    )
    .unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn ssh_crashes_at_rename_never_publish_partial_files_and_verify_lost_receipts() {
    if !required() {
        return;
    }
    for phase in ["before", "after"] {
        let fixture = Fixture::new();
        let path = fixture.root().join("fault-save.txt");
        std::fs::write(&path, "before\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        fault_interposer(&fixture, phase);
        let output = fixture.open(&uri("fixture", &path), "settle\nkeys :remote edit<cr>\nsettle\nkeys ggciwAFTER<esc>:w<cr>\nsettle\nstate\nkeys :remote verify<cr>\nsettle\nstate\n");
        let observations = states(&output);
        assert_eq!(
            observations[0]["dirty"], true,
            "a killed helper cannot acknowledge a write"
        );
        assert!(
            observations[0]["message"]
                .as_str()
                .unwrap()
                .contains("unconfirmed"),
            "{output}"
        );
        if phase == "before" {
            assert_eq!(std::fs::read(&path).unwrap(), b"before\n");
            assert_eq!(observations[1]["dirty"], true);
            let stage = std::fs::read_dir(fixture.root())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .as_encoded_bytes()
                        .starts_with(b".strop-save-")
                })
                .unwrap();
            assert_eq!(std::fs::metadata(stage).unwrap().mode() & 0o7777, 0o700);
        } else {
            assert_eq!(std::fs::read(&path).unwrap(), b"AFTER\n");
            assert_eq!(
                observations[1]["dirty"], false,
                "verification acknowledges the complete durable state"
            );
        }
    }
}
