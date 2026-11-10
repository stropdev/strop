//! Plan 0090 §5: the plain `ssh` exec path over a real sshd — one
//! multiplexed master, quoted argv, and the pooled SFTP session's read,
//! save and listing with the changed-on-disk refusal.
use super::*;
use std::ffi::OsStr;
use strop_remote::exec::ExecHost;
use strop_remote::{Expect, RemoteClient, RemoteEntryKind, RemoteFailureKind};
use strop_workspace::{RemoteFile, RemoteLocation};

fn stdout_of(host: &ExecHost, argv: &[&OsStr], cwd: Option<&Path>) -> Vec<u8> {
    let output = host.command(argv, cwd).output().unwrap();
    assert!(
        output.status.success(),
        "{argv:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn file(path: &Path) -> RemoteFile {
    RemoteFile::from_path(endpoint("fixture"), path.to_path_buf()).unwrap()
}

fn location(path: &Path) -> RemoteLocation {
    file(path).into()
}

#[test]
fn exec_runs_argv_exactly_in_cwd_over_one_master() {
    let Some(scope) = serial() else { return };
    let fixture = &scope.fixture;
    let (token, _owner) = token();
    let connect = || {
        ExecHost::connect(&endpoint("fixture"), &fixture.control(), &token)
            .expect("the master authenticates")
    };
    let host = connect();
    assert!(host.is_connected());
    let work = tempfile::tempdir_in(fixture.root()).unwrap();
    let cwd = work.path().join("it's a dir");
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
    let again = connect();
    assert!(again.is_connected());
    host.disconnect().unwrap();
    assert!(!again.is_connected());
    let fresh = connect();
    assert_eq!(stdout_of(&fresh, &[OsStr::new("true")], None), b"");
    fresh.disconnect().unwrap();
}

#[test]
fn save_is_atomic_preserves_mode_and_refuses_a_changed_file() {
    let Some(scope) = serial() else { return };
    let fixture = &scope.fixture;
    let client = RemoteClient::with_control_dir(fixture.control());
    let (token, _owner) = token();
    let work = tempfile::tempdir_in(fixture.root()).unwrap();
    let path = work.path().join("note.txt");
    std::fs::write(&path, "before\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();

    let loaded = client.read(&location(&path), 1 << 20, &token).unwrap();
    assert_eq!(loaded.buffer.text(), "before\n");
    let saved = client
        .save(
            &file(&path),
            b"mine\n".to_vec(),
            Expect::Unchanged(loaded.stamp),
            &token,
        )
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"mine\n");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    assert_eq!(saved.size, 5);

    // Another writer moves the file: the save is refused and their
    // bytes stay; the session survives the refusal.
    std::fs::write(&path, "theirs, longer\n").unwrap();
    let refused = client
        .save(
            &file(&path),
            b"stale\n".to_vec(),
            Expect::Unchanged(saved),
            &token,
        )
        .unwrap_err();
    assert_eq!(refused.kind(), RemoteFailureKind::Changed, "{refused}");
    assert!(refused.to_string().contains("changed on disk"), "{refused}");
    assert_eq!(std::fs::read(&path).unwrap(), b"theirs, longer\n");
    // `:w!` replaces it anyway.
    client
        .save(&file(&path), b"forced\n".to_vec(), Expect::Any, &token)
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"forced\n");

    // Save-as never clobbers; a new file is created.
    let exists = client
        .save(&file(&path), b"x".to_vec(), Expect::Absent, &token)
        .unwrap_err();
    assert_eq!(exists.kind(), RemoteFailureKind::Exists, "{exists}");
    let created = work.path().join("created.txt");
    client
        .save(&file(&created), b"new\n".to_vec(), Expect::Absent, &token)
        .unwrap();
    assert_eq!(std::fs::read(&created).unwrap(), b"new\n");

    // A save through a symlink lands on its target; the link stays.
    let link = work.path().join("link.txt");
    std::os::unix::fs::symlink(&created, &link).unwrap();
    let through = client.read(&location(&link), 1 << 20, &token).unwrap();
    client
        .save(
            &file(&link),
            b"via link\n".to_vec(),
            Expect::Unchanged(through.stamp),
            &token,
        )
        .unwrap();
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(&created).unwrap(), b"via link\n");

    // A file past the read window round-trips exactly, and no temporary
    // is left behind.
    let large: String = (0..300_000_u32).map(|n| format!("{n:07}\n")).collect();
    let big = work.path().join("big.txt");
    client
        .save(
            &file(&big),
            large.clone().into_bytes(),
            Expect::Absent,
            &token,
        )
        .unwrap();
    assert_eq!(
        client
            .read(&location(&big), 1 << 24, &token)
            .unwrap()
            .buffer
            .text(),
        large.as_str()
    );
    let listed = client.list(&location(work.path()), &token).unwrap();
    let mut names: Vec<_> = listed
        .entries
        .iter()
        .map(|entry| {
            entry
                .file
                .path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(names, ["big.txt", "created.txt", "link.txt", "note.txt"]);
    // The read bound is enforced before any byte moves.
    let Err(bounded) = client.read(&location(&big), 1024, &token) else {
        panic!("a read past its bound must be refused");
    };
    assert_eq!(bounded.kind(), RemoteFailureKind::TooLarge, "{bounded}");
    client.disconnect_all().unwrap();
}

#[test]
fn listing_reports_kinds_and_sizes_and_the_client_owns_its_master() {
    let Some(scope) = serial() else { return };
    let fixture = &scope.fixture;
    let client = RemoteClient::with_control_dir(fixture.control());
    let (token, _owner) = token();
    let work = tempfile::tempdir_in(fixture.root()).unwrap();
    std::fs::write(work.path().join("file"), "12345").unwrap();
    std::fs::create_dir(work.path().join("dir")).unwrap();
    let mut entries = client.list(&location(work.path()), &token).unwrap().entries;
    entries.sort_by(|a, b| a.file.path().cmp(b.file.path()));
    assert_eq!(entries[0].kind, RemoteEntryKind::Directory);
    assert_eq!(entries[1].kind, RemoteEntryKind::File);
    assert_eq!(entries[1].size, Some(5));
    // A held connection (an open document holds one) answers the input
    // path's host lookup without I/O.
    let lease = client.connect(&endpoint("fixture"), &token).unwrap();
    let host = client.host(&endpoint("fixture"), &token).unwrap();
    assert!(host.is_connected());
    assert_eq!(
        client.connected_host(&endpoint("fixture")),
        Some(host.clone())
    );
    // Dropping the last client stops the master it authenticated.
    drop(lease);
    drop(client);
    assert!(!host.is_connected());
}

#[test]
fn file_management_creates_renames_copies_removes_and_never_replaces() {
    use strop_remote::Mutation;
    let Some(scope) = serial() else { return };
    let fixture = &scope.fixture;
    let client = RemoteClient::with_control_dir(fixture.control());
    let (token, _owner) = token();
    let host = endpoint("fixture");
    let work = tempfile::tempdir_in(fixture.root()).unwrap();
    let at = |name: &str| work.path().join(name);
    let run = |mutation| client.mutate(&host, mutation, &token);

    run(Mutation::CreateDirectory { path: at("dir") }).unwrap();
    let created = run(Mutation::CreateFile {
        path: at("dir/a.txt"),
        bytes: b"alpha\n".to_vec(),
    })
    .unwrap()
    .unwrap();
    assert_eq!(created.size, Some(6));
    std::fs::set_permissions(at("dir/a.txt"), std::fs::Permissions::from_mode(0o640)).unwrap();
    run(Mutation::Copy {
        from: at("dir/a.txt"),
        to: at("b.txt"),
        max: 1 << 20,
    })
    .unwrap();
    assert_eq!(std::fs::read(at("b.txt")).unwrap(), b"alpha\n");
    assert_eq!(
        std::fs::metadata(at("b.txt")).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    run(Mutation::Rename {
        from: at("dir/a.txt"),
        to: at("c.txt"),
    })
    .unwrap();
    assert!(!at("dir/a.txt").exists());
    assert_eq!(std::fs::read(at("c.txt")).unwrap(), b"alpha\n");

    // Nothing replaces an existing name, and the session survives each
    // refusal.
    for refused in [
        Mutation::CreateFile {
            path: at("b.txt"),
            bytes: b"x".to_vec(),
        },
        Mutation::CreateDirectory { path: at("dir") },
        Mutation::Rename {
            from: at("c.txt"),
            to: at("b.txt"),
        },
        Mutation::Copy {
            from: at("c.txt"),
            to: at("b.txt"),
            max: 1 << 20,
        },
    ] {
        let Err(error) = run(refused.clone()) else {
            panic!("{refused:?} replaced an existing name");
        };
        assert_eq!(error.kind(), RemoteFailureKind::Failure, "{error}");
    }
    assert_eq!(std::fs::read(at("b.txt")).unwrap(), b"alpha\n");
    assert_eq!(std::fs::read(at("c.txt")).unwrap(), b"alpha\n");

    // A non-empty directory is refused; files and empty directories go.
    std::fs::write(at("dir/keep"), "keep").unwrap();
    assert!(run(Mutation::Remove {
        path: at("dir"),
        directory: true,
    })
    .is_err());
    assert!(at("dir/keep").exists());
    run(Mutation::Remove {
        path: at("dir/keep"),
        directory: false,
    })
    .unwrap();
    run(Mutation::Remove {
        path: at("dir"),
        directory: true,
    })
    .unwrap();
    assert!(!at("dir").exists());
    let observed = client
        .observe(&file(&at("c.txt")), &token)
        .unwrap()
        .unwrap();
    assert_eq!(observed.kind, RemoteEntryKind::File);
    assert!(client.observe(&file(&at("dir")), &token).unwrap().is_none());
    client.disconnect_all().unwrap();
}
