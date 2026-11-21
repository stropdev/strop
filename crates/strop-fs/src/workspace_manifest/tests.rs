use super::*;
use rustix::fs::{mknodat, FileType, Mode, CWD};
use std::fs;
use std::os::unix::fs::symlink;
use std::process::Command;

fn setup(text: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".strop")).unwrap();
    fs::write(root.path().join(MANIFEST), text).unwrap();
    root
}

#[test]
fn no_upward_or_nested_lookup_and_no_missing_diagnostic() {
    let outer = setup("not even TOML");
    let root = outer.path().join("child");
    fs::create_dir_all(root.join("nested/.strop")).unwrap();
    fs::write(root.join("nested").join(MANIFEST), "invalid").unwrap();
    assert!(read(&root).unwrap().is_empty());
    fs::create_dir(root.join(".strop")).unwrap();
    assert!(read(&root).unwrap().is_empty());
    assert!(!root.join(MANIFEST).exists());
}

#[test]
fn unsafe_spelling_and_bad_schema_reject_even_after_a_valid_record() {
    let prefix = "version = 1\n[[project]]\npath = 'valid'\n";
    for path in [
        "",
        ".",
        "..",
        "child/./pkg",
        "child/../pkg",
        "child//pkg",
        "child/",
        "/etc",
        "C:/dir",
        r"C:\dir",
        "https://host/x",
        "file:relative",
        "~/dir",
        "$HOME/dir",
        "${HOME}",
        "c*",
        "p?",
        "a[bc]",
        "{a,b}",
        "valid",
    ] {
        let text = format!("{prefix}[[project]]\npath = '{path}'\n");
        assert!(parse(&text).is_err(), "{path:?}");
    }
    for tail in [
        "command = 'run'",
        "server = 'a'",
        "args = []",
        "env = {}",
        "trust = true",
        "languages = 'rust'",
        "languages = [1]",
        "path = 'dup'",
        "languages = ['']",
        "[[project]]\nname = 'no path'",
    ] {
        assert!(parse(&format!("{prefix}{tail}")).is_err(), "{tail}");
    }
    for text in [
        "",
        "version =",
        "version = 2",
        "version = '1'",
        "version = 1\nunknown = []",
        "version = 1\n[[project]]\npath = \"a\\u0000b\"",
    ] {
        assert!(parse(text).is_err(), "{text}");
    }
    let temp = setup(&format!("{prefix}[[project]]\npath = '/outside'"));
    fs::create_dir(temp.path().join("valid")).unwrap();
    let error = read(temp.path()).unwrap_err();
    assert!(error.to_string().contains(MANIFEST));
}

#[test]
fn byte_and_record_limits_apply_to_whole_input() {
    let temp = setup("");
    let root = temp.path();
    let mut text = "version = 1\n".to_owned();
    for n in 0..MAX_PROJECTS {
        use std::fmt::Write;
        writeln!(text, "[[project]]\npath = 'project {n}'").unwrap();
        fs::create_dir(root.join(format!("project {n}"))).unwrap();
    }
    fs::write(root.join(MANIFEST), &text).unwrap();
    assert_eq!(read(root).unwrap().len(), MAX_PROJECTS);
    let extra = format!("{text}[[project]]\npath = 'missing'\n");
    fs::write(root.join(MANIFEST), extra).unwrap();
    assert!(read(root).unwrap_err().to_string().contains("256"));
    text.push('#');
    text.push_str(&"x".repeat(MAX_BYTES - text.len()));
    fs::write(root.join(MANIFEST), &text).unwrap();
    assert_eq!(read(root).unwrap().len(), MAX_PROJECTS);
    text.push('x');
    fs::write(root.join(MANIFEST), text).unwrap();
    assert!(read(root).unwrap_err().to_string().contains("64 KiB"));
    fs::write(root.join(MANIFEST), b"version=1\n#\xff").unwrap();
    assert!(read(root).unwrap_err().to_string().contains("UTF-8"));
}

#[test]
fn linked_manifest_directory_leaf_and_fifo_never_supply_config() {
    let outside = setup("version = 1");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    symlink(outside.path().join(".strop"), root.join(".strop")).unwrap();
    assert!(read(root).is_err());
    fs::remove_file(root.join(".strop")).unwrap();
    fs::create_dir(root.join(".strop")).unwrap();
    symlink(outside.path().join(MANIFEST), root.join(MANIFEST)).unwrap();
    assert!(read(root).is_err());
    fs::remove_file(root.join(MANIFEST)).unwrap();
    mknodat(CWD, root.join(MANIFEST), FileType::Fifo, Mode::RUSR, 0).unwrap();
    assert!(read(root).unwrap_err().to_string().contains("regular file"));
    fs::remove_file(root.join(MANIFEST)).unwrap();
    fs::create_dir(root.join(MANIFEST)).unwrap();
    assert!(read(root).is_err());
}

#[test]
fn unavailable_members_skip_only_their_own_bindings() {
    let temp = setup("version = 1\n[[project]]\npath = 'one'\nlanguages = ['unknown-lang', 'rust']\n[[project]]\npath = 'two'\nlanguages = []\n[[project]]\npath = 'three'\n[[project]]\npath = 'missing'\n[[project]]\npath = 'fifo'\n[[project]]\npath = 'plain'\n[[project]]\npath = 'link'\n[[project]]\npath = 'link/nested'\n");
    let root = temp.path();
    for dir in ["one", "two", "three"] {
        fs::create_dir(root.join(dir)).unwrap();
    }
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(outside.path().join("nested")).unwrap();
    symlink(outside.path(), root.join("link")).unwrap();
    fs::write(root.join("plain"), "").unwrap();
    mknodat(CWD, root.join("fifo"), FileType::Fifo, Mode::RUSR, 0).unwrap();
    let records = read(root).unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].path, Path::new("one"));
    assert_eq!(
        records[0].languages.as_ref().unwrap(),
        &["unknown-lang", "rust"]
    );
    assert_eq!(records[1].languages, Some(vec![]));
    assert_eq!(records[2].languages, None);
    for path in ["", "./one", "../one", "one/", "link", "link/nested", "fifo"] {
        assert!(open_directory(root, Path::new(path)).is_err(), "{path}");
    }
}

#[test]
fn descriptor_cwd_does_not_follow_a_retargeted_member() {
    let temp = setup("version = 1\n[[project]]\npath = 'child/member'");
    let root = temp.path();
    let child = Path::new("child/member");
    fs::create_dir_all(root.join(child)).unwrap();
    fs::write(root.join(child).join("who"), "inside").unwrap();
    let declaration = read(root).unwrap().remove(0);
    let fd = declaration.witness.reopen(root, child).unwrap();
    fs::rename(root.join("child"), root.join("moved")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(outside.path().join("member")).unwrap();
    fs::write(outside.path().join("member/who"), "outside").unwrap();
    symlink(outside.path(), root.join("child")).unwrap();
    assert!(declaration.witness.reopen(root, child).is_err());
    assert!(read(root).unwrap().is_empty());
    let result = Command::new("/bin/cat")
        .arg("who")
        .current_dir(format!("/proc/self/fd/{}", fd.as_raw_fd()))
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(result.stdout, b"inside");
    fs::remove_file(root.join("child")).unwrap();
    fs::create_dir_all(root.join(child)).unwrap();
    assert!(declaration.witness.reopen(root, child).is_err());
    assert!(read(root).unwrap()[0].witness.reopen(root, child).is_ok());
}

#[test]
fn manifest_in_place_edit_replacement_and_removal_retire_binding() {
    let text = "version = 1\n[[project]]\npath = 'one'\nlanguages = ['cpp']";
    let temp = setup(text);
    let root = temp.path();
    let path = Path::new("one");
    fs::create_dir(root.join(path)).unwrap();
    let first = read(root).unwrap().remove(0);
    let manifest = root.join(MANIFEST);
    let stamp = fs::metadata(&manifest).unwrap().modified().unwrap();
    fs::write(&manifest, text.replace("cpp", "nim")).unwrap();
    File::options()
        .write(true)
        .open(&manifest)
        .unwrap()
        .set_modified(stamp)
        .unwrap();
    assert!(first.witness.reopen(root, path).is_err());
    fs::write(root.join("new"), text).unwrap();
    fs::rename(root.join("new"), &manifest).unwrap();
    assert!(first.witness.reopen(root, path).is_err());
    let next = read(root).unwrap().remove(0);
    fs::remove_file(&manifest).unwrap();
    assert!(next.witness.reopen(root, path).is_err());
    assert!(read(root).unwrap().is_empty());
}

#[test]
fn same_root_spelling_cannot_borrow_another_incarnation() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("workarea");
    let old = base.path().join("retired");
    let path = Path::new("member");
    fs::create_dir_all(root.join(".strop")).unwrap();
    fs::write(root.join(MANIFEST), "version=1\n[[project]]\npath='member'").unwrap();
    fs::create_dir(root.join(path)).unwrap();
    let record = read(&root).unwrap().remove(0);
    fs::rename(&root, &old).unwrap();
    fs::create_dir(&root).unwrap();
    // Both the manifest and member are the very same inodes. Only the
    // workspace directory changed at the same textual spelling.
    fs::rename(old.join(".strop"), root.join(".strop")).unwrap();
    fs::rename(old.join(path), root.join(path)).unwrap();
    assert!(record.witness.reopen(&root, path).is_err());
    assert!(read(&root).unwrap()[0].witness.reopen(&root, path).is_ok());
}
