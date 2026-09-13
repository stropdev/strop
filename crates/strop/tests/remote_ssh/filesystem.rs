use super::*;

#[test]
fn remote_rename_preserves_dirty_text_and_requires_fresh_write_authority() {
    if std::env::var_os("STROP_REQUIRE_SSH_TESTS").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let fixture = Fixture::new();
    let original = fixture.root().join("original.txt");
    let renamed = fixture.root().join("renamed.txt");
    std::fs::write(&original, "original\n").unwrap();
    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o640)).unwrap();
    let destination = uri("fixture", &renamed);
    let steps = fixture.script("rename.steps", &format!(
        "settle\nkeys :remote edit<cr>\nsettle\nkeys ggciwDIRTY<esc>:fs rename renamed.txt<cr>\nsettle\nkeys :apply-change<cr>\nsettle\nkeys :e {destination}<cr>\nsettle\nkeys yy\nstate\nkeys :w<cr>\nsettle\nstate\nkeys :remote edit<cr>\nsettle\nstate\nkeys :w<cr>\nsettle\nstate\nframe\nkeys :qa!<cr>\nsettle\n"
    ));
    let trace = fixture.root().join("rename.jsonl");
    let output = fixture.run(&[
        "--headless".into(),
        steps.into(),
        uri("fixture", &original).into(),
        "--log-file".into(),
        trace.clone().into(),
        "--log-content".into(),
    ]);
    let observed = states(&output);
    assert_eq!(observed[0]["register"], "DIRTY\n", "{output}");
    assert_eq!(observed[0]["dirty"], true);
    assert_eq!(observed[1]["dirty"], true);
    assert!(
        observed[1]["message"]
            .as_str()
            .unwrap()
            .contains("remote edit"),
        "{output}"
    );
    assert_eq!(observed[3]["dirty"], false, "{output}");
    assert!(!original.exists());
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "DIRTY\n");
    assert_eq!(
        std::fs::metadata(&renamed).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    std::fs::write(
        fixture.root().join("bin/ssh"),
        "#!/bin/sh\necho forbidden >&2\nexit 99\n",
    )
    .unwrap();
    fixture.run(&["--replay".into(), trace.into()]);
    assert!(!original.exists());
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "DIRTY\n");
}

#[test]
fn remote_creation_and_copy_publish_the_explicit_version_without_clobbering() {
    if std::env::var_os("STROP_REQUIRE_SSH_TESTS").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let fixture = Fixture::new();
    let scope = fixture.root().join("workspace");
    std::fs::create_dir(&scope).unwrap();
    let source = scope.join("nested/source.txt");
    let stored = scope.join("nested/stored-copy.txt");
    let current = scope.join("nested/buffer-copy.txt");
    let source_uri = uri("fixture", &source);
    let occupied_uri = uri("fixture", &stored);
    let steps = fixture.script("create-copy.steps", &format!(
        "settle\nkeys :fs create nested/source.txt<cr>\nsettle\nkeys :apply-change<cr>\nsettle\nkeys :remote edit<cr>\nsettle\nkeys istored<esc>:w<cr>\nsettle\nkeys 0ciwbuffer<esc>:fs copy stored stored-copy.txt<cr>\nsettle\nkeys :apply-change<cr>\nsettle\nkeys :e {source_uri}<cr>\nsettle\nkeys :fs copy buffer buffer-copy.txt<cr>\nsettle\nkeys :apply-change<cr>\nsettle\nkeys :fs create {occupied_uri}<cr>\nsettle\nkeys :apply-change<cr>\nsettle\nframe\nkeys :qa!<cr>\nsettle\n"
    ));
    let trace = fixture.root().join("create-copy.jsonl");
    let output = fixture.run(&[
        "--headless".into(),
        steps.into(),
        uri("fixture", &scope).into(),
        "--log-file".into(),
        trace.clone().into(),
        "--log-content".into(),
    ]);
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        "stored",
        "{output}"
    );
    assert_eq!(
        std::fs::read_to_string(&stored).unwrap(),
        "stored",
        "{output}"
    );
    assert_eq!(
        std::fs::read_to_string(&current).unwrap(),
        "buffer",
        "{output}"
    );
    std::fs::write(fixture.root().join("bin/ssh"), "#!/bin/sh\nexit 99\n").unwrap();
    fixture.run(&["--replay".into(), trace.into()]);
    assert_eq!(std::fs::read_to_string(&stored).unwrap(), "stored");
    assert_eq!(std::fs::read_to_string(&current).unwrap(), "buffer");
}

#[test]
fn remote_directory_emptied_by_strop_can_be_removed_and_replayed() {
    if std::env::var_os("STROP_REQUIRE_SSH_TESTS").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let fixture = Fixture::new();
    let directory = fixture.root().join("directory");
    std::fs::create_dir(&directory).unwrap();
    let target = uri("fixture", &directory);
    let steps = fixture.script("remove-directory.steps", &format!(
        "settle\nkeys :fs create child<cr>\nsettle\nkeys :apply-change<cr>\nsettle\nkeys :fs remove<cr>\nsettle\nkeys :apply-change<cr>\nsettle\nkeys :e {target}<cr>\nsettle\nkeys -\nsettle\nkeys :fs remove<cr>\nsettle\nkeys :apply-change<cr>\nsettle\nframe\nkeys :qa!<cr>\nsettle\n"
    ));
    let trace = fixture.root().join("remove-directory.jsonl");
    let output = fixture.run(&[
        "--headless".into(),
        steps.into(),
        target.into(),
        "--log-file".into(),
        trace.clone().into(),
        "--log-content".into(),
    ]);
    assert!(!directory.exists(), "{output}");
    std::fs::write(fixture.root().join("bin/ssh"), "#!/bin/sh\nexit 99\n").unwrap();
    fixture.run(&["--replay".into(), trace.into()]);
    assert!(!directory.exists());
}
