use super::*;

fn save(editor: &Editor) -> Result<bool, SessionError> {
    let Some(request) = capture_save(editor) else {
        return Ok(false);
    };
    request.persist()?;
    Ok(true)
}

#[test]
fn roundtrip_restores_buffers_and_position() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let mut e = Editor::new(Buffer::open(root.join("a.rs")).unwrap());
    e.cwd = root.to_path_buf();
    e.state_dir = Some(root.join("state"));
    e.feed_text("jl");
    e.feed_text("ix");
    e.feed(crate::editor::Key::Esc);
    assert!(save(&e).unwrap());
    let mut e2 = Editor::new(Buffer::from_text(""));
    e2.cwd = root.to_path_buf();
    e2.state_dir = Some(root.join("state"));
    assert!(restore(&mut e2).unwrap());
    assert_eq!(e2.buf().path.as_deref(), Some(root.join("a.rs").as_path()));
    assert_eq!(e2.buf().line_of(e2.head()), 1);
    assert_eq!(e2.buf().col_of(e2.head()), 1);
    assert_eq!(
        e2.buf().history().depth(),
        1,
        "dirty history must not replay against the disk version"
    );
}

#[test]
fn empty_or_readonly_never_persist() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Editor::new(Buffer::from_text(""));
    e.cwd = dir.path().to_path_buf();
    e.state_dir = Some(dir.path().join("state"));
    assert!(!save(&e).unwrap());
    assert!(!dir.path().join("state").exists());
    let path = dir.path().join("a");
    std::fs::write(&path, "a").unwrap();
    let mut buf = Buffer::open(path).unwrap();
    buf.readonly = true;
    let mut e = Editor::new(buf);
    e.state_dir = Some(dir.path().join("state"));
    assert!(!save(&e).unwrap());
    assert!(!dir.path().join("state").exists());
}

#[test]
fn undo_history_crosses_when_disk_matches() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    let mut e = Editor::new(Buffer::open(root.join("a.rs")).unwrap());
    e.cwd = root.to_path_buf();
    e.state_dir = Some(root.join("state"));
    e.feed_text("o// note");
    e.feed(crate::editor::Key::Esc);
    e.feed_text(":w\r");
    e.wait_io().unwrap();
    assert!(save(&e).unwrap());
    let mut e2 = Editor::new(Buffer::from_text(""));
    e2.cwd = root.to_path_buf();
    e2.state_dir = Some(root.join("state"));
    assert!(restore(&mut e2).unwrap());
    assert!(
        e2.buf().text().to_string().contains("// note"),
        "restore starts from the saved revision"
    );
    e2.feed_text("u");
    assert_eq!(e2.buf().text().to_string(), "fn a() {}\n");
}

#[test]
fn trust_store_remembers_projects() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let root = Path::new("/tmp/proj-a");
    assert!(!is_trusted(Some(&state), root).unwrap());
    trust(Some(&state), root).unwrap();
    assert!(is_trusted(Some(&state), root).unwrap());
    assert!(!is_trusted(Some(&state), Path::new("/tmp/proj-b")).unwrap());
    assert!(!is_trusted(None, root).unwrap());
}

#[test]
fn failed_restore_leaves_live_state_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a");
    std::fs::write(&path, "a").unwrap();
    let e = Editor::new(Buffer::open(&path).unwrap());
    let mut session = capture(&e).unwrap();
    session.buffers.push(BufferState {
        path: dir.path().to_owned(),
        line: 0,
        col: 0,
        view_top: 0,
        undo: None,
        undo_hash: 0,
    });
    let mut live = Editor::new(Buffer::from_text("keep me"));
    let id = live.current();
    let generation = live.generation;
    assert!(session.restore(&mut live).is_err());
    assert_eq!(live.current(), id);
    assert_eq!(live.generation, generation);
    assert_eq!(live.buf().text().to_string(), "keep me");
}

#[test]
fn legacy_paths_migrate_and_ambiguous_paths_fail() {
    let make = |path: &str| {
        serde_json::json!({"buffers": [{
        "path": path, "line": 0, "col": 0, "view_top": 0, "undo": null
    }], "current": 0})
    };
    let session: Session = serde_json::from_value(make("safe.rs")).unwrap();
    let value = serde_json::to_value(session).unwrap();
    assert_eq!(value["buffers"][0]["path"]["version"], 1);
    assert!(serde_json::from_value::<Session>(make("bad\u{fffd}.rs")).is_err());
}

#[cfg(unix)]
#[test]
fn native_paths_roundtrip_without_loss() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(std::ffi::OsStr::from_bytes(b"\xff.rs"));
    std::fs::write(&path, "native\n").unwrap();
    std::fs::write(dir.path().join("\u{fffd}.rs"), "wrong alias\n").unwrap();
    let mut e = Editor::new(Buffer::open(&path).unwrap());
    e.cwd = dir.path().to_owned();
    e.state_dir = Some(dir.path().join("state"));
    let request = capture_save(&e).unwrap();
    drop(e);
    std::thread::spawn(move || request.persist())
        .join()
        .unwrap()
        .unwrap();
    let mut live = Editor::new(Buffer::from_text(""));
    live.cwd = dir.path().to_owned();
    live.state_dir = Some(dir.path().join("state"));
    assert!(restore(&mut live).unwrap());
    assert_eq!(live.buf().path.as_deref(), Some(path.as_path()));
    assert_eq!(live.buf().text().to_string(), "native\n");
    use std::os::unix::fs::PermissionsExt;
    let target = session_path(live.state_dir.as_deref(), &live.cwd).unwrap();
    assert_eq!(
        std::fs::metadata(target).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn failed_publication_preserves_previous_session_and_cleans_staging() {
    use std::io::Write;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.json");
    std::fs::write(&path, b"previous session").unwrap();
    let result = persistence::publish(&path, |file| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(file.metadata()?.permissions().mode() & 0o777, 0o600);
        }
        file.write_all(b"partial private content")?;
        Err(std::io::Error::other("injected sync failure"))
    });
    assert!(matches!(result, Err(SessionError::Io { .. })));
    assert_eq!(std::fs::read(&path).unwrap(), b"previous session");
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        1,
        "no abandoned private staging file"
    );
    persistence::publish(&path, |file| {
        file.write_all(b"next session")
            .and_then(|()| file.sync_all())
    })
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"next session");
}

#[cfg(unix)]
#[test]
fn atomic_publication_replaces_destination_symlink_without_touching_target() {
    use std::io::Write;
    use std::os::unix::fs::{symlink, PermissionsExt};
    let directory = tempfile::tempdir().unwrap();
    let outside = directory.path().join("outside");
    let path = directory.path().join("session");
    std::fs::write(&outside, b"untouched").unwrap();
    symlink(&outside, &path).unwrap();
    persistence::publish(&path, |file| {
        assert_eq!(file.metadata()?.permissions().mode() & 0o777, 0o600);
        file.write_all(b"private session")?;
        file.sync_all()
    })
    .unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"untouched");
    assert!(!std::fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(&path).unwrap(), b"private session");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
