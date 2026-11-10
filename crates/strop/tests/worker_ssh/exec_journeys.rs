//! Plan 0090 §5: the plain `ssh` exec path over the same real sshd —
//! one multiplexed master, quoted argv, SFTP read/save/list with the
//! changed-on-disk refusal. Nothing of strop is installed on the host.
use super::*;
use std::ffi::OsStr;
use strop_remote::exec::{ExecError, ExecHost, Expect};

fn connect(fixture: &Fixture) -> ExecHost {
    ExecHost::connect(&endpoint("fixture"), &fixture.root().join("ctl"))
        .expect("the master authenticates")
}

fn stdout_of(host: &ExecHost, argv: &[&OsStr], cwd: Option<&Path>) -> Vec<u8> {
    let output = host.command(argv, cwd).output().unwrap();
    assert!(
        output.status.success(),
        "{argv:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn exec_runs_argv_exactly_in_cwd_over_one_master() {
    let Some(host_fixture) = serial() else { return };
    let fixture = &host_fixture.fixture;
    let host = connect(fixture);
    assert!(host.is_connected());
    let scope = tempfile::tempdir_in(fixture.root()).unwrap();
    let cwd = scope.path().join("it's a dir");
    std::fs::create_dir(&cwd).unwrap();
    let words = [
        "it's",
        "$HOME `id`",
        "a; b && c | d",
        "line\nbreak",
        "*",
        "",
    ];
    let mut argv: Vec<&OsStr> = vec![OsStr::new("printf"), OsStr::new("%s\\0")];
    argv.extend(words.iter().map(OsStr::new));
    let printed = stdout_of(&host, &argv, Some(&cwd));
    let parsed: Vec<&[u8]> = printed.split(|byte| *byte == 0).collect();
    assert_eq!(&parsed[..words.len()], words.map(str::as_bytes));
    let pwd = stdout_of(&host, &[OsStr::new("pwd")], Some(&cwd));
    assert_eq!(pwd, format!("{}\n", cwd.display()).into_bytes());

    // A second connect reuses the live master; disconnect stops it and
    // the next connect authenticates again.
    let again = connect(fixture);
    assert!(again.is_connected());
    host.disconnect().unwrap();
    assert!(!again.is_connected());
    let fresh = connect(fixture);
    assert_eq!(stdout_of(&fresh, &[OsStr::new("true")], None), b"");
    fresh.disconnect().unwrap();
}

#[test]
fn save_is_atomic_preserves_mode_and_refuses_a_changed_file() {
    let Some(host_fixture) = serial() else { return };
    let fixture = &host_fixture.fixture;
    let host = connect(fixture);
    let mut files = host.files().unwrap();
    let scope = tempfile::tempdir_in(fixture.root()).unwrap();
    let path = scope.path().join("note.txt");
    std::fs::write(&path, "before\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();

    let loaded = files.read(&path, 1 << 20).unwrap();
    assert_eq!(loaded.bytes, b"before\n");
    let saved = files
        .write(&path, b"mine\n", Expect::Unchanged(loaded.stamp))
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"mine\n");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    assert_eq!(saved.size, 5);

    // Another writer moves the file: the save is refused and their
    // bytes stay.
    std::fs::write(&path, "theirs, longer\n").unwrap();
    let refused = files
        .write(&path, b"stale\n", Expect::Unchanged(saved))
        .unwrap_err();
    assert!(
        matches!(refused, ExecError::ChangedOnDisk { .. }),
        "{refused}"
    );
    assert!(refused.to_string().contains("changed on disk"), "{refused}");
    assert_eq!(std::fs::read(&path).unwrap(), b"theirs, longer\n");
    // `:w!` replaces it anyway.
    files.write(&path, b"forced\n", Expect::Any).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"forced\n");

    // Save-as never clobbers; a new file is created.
    assert!(matches!(
        files.write(&path, b"x", Expect::Absent),
        Err(ExecError::Exists { .. })
    ));
    let created = scope.path().join("created.txt");
    files.write(&created, b"new\n", Expect::Absent).unwrap();
    assert_eq!(std::fs::read(&created).unwrap(), b"new\n");

    // A save through a symlink lands on its target; the link stays.
    let link = scope.path().join("link.txt");
    std::os::unix::fs::symlink(&created, &link).unwrap();
    let through = files.read(&link, 1 << 20).unwrap();
    files
        .write(&link, b"via link\n", Expect::Unchanged(through.stamp))
        .unwrap();
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(&created).unwrap(), b"via link\n");

    // A multi-chunk file round-trips exactly, and no temporary is left.
    let large: Vec<u8> = (0..3_000_000_u32).map(|n| (n % 251) as u8).collect();
    let big = scope.path().join("big.bin");
    files.write(&big, &large, Expect::Absent).unwrap();
    assert_eq!(files.read(&big, 1 << 24).unwrap().bytes, large);
    let mut names: Vec<_> = files
        .list(scope.path())
        .unwrap()
        .into_iter()
        .map(|entry| entry.name.to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["big.bin", "created.txt", "link.txt", "note.txt"]);
    // The read bound is enforced before any byte moves.
    assert!(matches!(
        files.read(&big, 1024),
        Err(ExecError::Sftp(detail)) if detail.contains("exceeds")
    ));
    drop(files);
    host.disconnect().unwrap();
}

#[test]
fn listing_reports_kinds_and_sizes() {
    let Some(host_fixture) = serial() else { return };
    let fixture = &host_fixture.fixture;
    let host = connect(fixture);
    let mut files = host.files().unwrap();
    let scope = tempfile::tempdir_in(fixture.root()).unwrap();
    std::fs::write(scope.path().join("file"), "12345").unwrap();
    std::fs::create_dir(scope.path().join("dir")).unwrap();
    let mut entries = files.list(scope.path()).unwrap();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(entries[0].kind, strop_remote::RemoteEntryKind::Directory);
    assert_eq!(entries[1].kind, strop_remote::RemoteEntryKind::File);
    assert_eq!(entries[1].size, Some(5));
    assert!(files.stat(&scope.path().join("absent")).unwrap().is_none());
    drop(files);
    host.disconnect().unwrap();
}
