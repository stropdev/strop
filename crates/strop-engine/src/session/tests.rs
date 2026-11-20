use super::*;
use strop_core::Buffer;

fn save(editor: &Editor) {
    capture_save(editor).unwrap().persist().unwrap();
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
    e.feed(strop_core::frontend_input::Key::Esc);
    save(&e);
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
fn command_line_history_roundtrips_per_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    let mut e = Editor::new(Buffer::open(root.join("a.txt")).unwrap());
    e.cwd = root.to_path_buf();
    e.state_dir = Some(root.join("state"));
    e.feed_text(":2<cr>/thr<cr>:1<cr>");
    save(&e);
    let mut e2 = Editor::new(Buffer::from_text(""));
    e2.cwd = root.to_path_buf();
    e2.state_dir = Some(root.join("state"));
    assert!(restore(&mut e2).unwrap());
    let ex: Vec<_> = e2.cmdline.history.ex.iter().collect();
    let search: Vec<_> = e2.cmdline.history.search.iter().collect();
    assert_eq!(ex, ["2", "1"]);
    assert_eq!(search, ["thr"]);
    e2.feed_text(":<up><up>");
    assert_eq!(e2.cmdline.text(), ":2");
}

#[test]
fn jump_history_roundtrips_across_documents_with_index_and_view() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let a_path = root.join("a.txt");
    let b_path = root.join("b.txt");
    let text = "zero\none\ntwo\nthree\nfour\n";
    std::fs::write(&a_path, text).unwrap();
    std::fs::write(&b_path, text).unwrap();
    let mut e = Editor::new(Buffer::open(&a_path).unwrap());
    e.cwd = root.to_owned();
    e.state_dir = Some(root.join("state"));
    e.set_head(e.buf().line_start(2) + 1);
    let anchor = e.buf().line_start(2);
    let head = e.head();
    e.sels_mut().stretch_primary(anchor, head);
    let extra = e.buf().line_start(3);
    e.sels_mut()
        .set_extra_selections(vec![strop_core::selection::Selection::cursor(extra)]);
    e.view_mut().view_top = 1;
    e.view_mut().hscroll = strop_core::id::DisplayColumn::new(4);
    e.push_jump();
    let b = e
        .docs
        .try_insert(Document::new(Buffer::open(&b_path).unwrap()))
        .unwrap();
    e.switch_to(b);
    e.set_head(e.buf().line_start(1));
    e.push_jump();
    e.set_head(e.buf().line_start(3));
    e.jump_back();
    let selected = e.view().jumps.position;
    save(&e);

    let mut restored = Editor::new(Buffer::from_text(""));
    restored.cwd = root.to_owned();
    restored.state_dir = Some(root.join("state"));
    assert!(restore(&mut restored).unwrap());
    assert_eq!(
        restored.current(),
        restored.view().jumps.entries[2].document
    );
    let path_of = |index: usize| {
        restored
            .doc(restored.view().jumps.entries[index].document)
            .buf
            .path
            .clone()
    };
    assert_eq!(path_of(1).as_deref(), Some(a_path.as_path()));
    assert_eq!(path_of(2).as_deref(), Some(b_path.as_path()));
    assert_eq!(restored.view().jumps.entries.len(), 4);
    assert_eq!(restored.view().jumps.position, selected);
    restored.scroll_to_cursor(3);
    restored.jump_back();
    assert_eq!(restored.buf().path.as_deref(), Some(a_path.as_path()));
    assert_eq!(restored.head(), restored.buf().line_start(2) + 1);
    assert_eq!(
        restored.sels().primary().anchor,
        restored.buf().line_start(2)
    );
    assert_eq!(
        restored.sels().extra_heads()[0].head,
        restored.buf().line_start(3)
    );
    assert_eq!(restored.view_top(), 1);
    assert_eq!(
        restored.view().hscroll,
        strop_core::id::DisplayColumn::new(4)
    );
}

#[test]
fn changed_files_clamp_restored_jump_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "zero\none\ntwo\nthree\n").unwrap();
    let mut e = Editor::new(Buffer::open(&path).unwrap());
    e.cwd = dir.path().to_owned();
    e.set_head(e.buf().line_start(3));
    e.view_mut().view_top = 3;
    e.push_jump();
    let session = capture(&e).unwrap();
    drop(e);
    std::fs::write(&path, "x\n").unwrap();

    let mut restored = Editor::new(Buffer::from_text(""));
    restored.cwd = dir.path().to_owned();
    session.restore(&mut restored).unwrap();
    let record = &restored.view().jumps.entries[1];
    assert_eq!(record.offset, restored.buf().len_bytes());
    assert_eq!(record.anchor, restored.buf().len_bytes());
    assert_eq!(record.view_top, restored.buf().len_bytes());
}

#[test]
fn quit_all_persists_every_buffer_and_the_complete_jump_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "a0\na1\n").unwrap();
    std::fs::write(&b, "b0\nb1\n").unwrap();
    let mut e = Editor::new(Buffer::open(&a).unwrap());
    e.cwd = root.to_owned();
    e.state_dir = Some(root.join("state"));
    e.open_fixture(&b).unwrap();
    e.feed_text("G");
    e.quit_all(false);
    e.wait_io().unwrap();

    let session = load(e.state_dir.as_deref(), &e.cwd)
        .unwrap()
        .expect("quit-all captured a session");
    assert_eq!(session.buffers.len(), 2);
    assert!(session.buffers.iter().any(|buffer| buffer.path == a));
    assert!(session.buffers.iter().any(|buffer| buffer.path == b));
    assert!(session.jumps.entries.iter().any(|jump| jump.path == a));
    assert!(session.jumps.entries.iter().any(|jump| jump.path == b));
}

#[test]
fn empty_or_readonly_never_persist() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Editor::new(Buffer::from_text(""));
    e.cwd = dir.path().to_path_buf();
    e.state_dir = Some(dir.path().join("state"));
    save(&e);
    assert!(!dir.path().join("state").exists());
    let path = dir.path().join("a");
    std::fs::write(&path, "a").unwrap();
    let mut buf = Buffer::open(path).unwrap();
    buf.readonly = true;
    let mut e = Editor::new(buf);
    e.state_dir = Some(dir.path().join("state"));
    save(&e);
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
    e.feed(strop_core::frontend_input::Key::Esc);
    e.feed_text(":w\r");
    e.wait_io().unwrap();
    save(&e);
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
fn bare_string_paths_are_refused() {
    let session = serde_json::json!({"buffers": [{
        "path": "safe.rs", "line": 0, "col": 0, "view_top": 0, "undo": null
    }], "current": 0});
    assert!(serde_json::from_value::<Session>(session).is_err());
}

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
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(file.metadata()?.permissions().mode() & 0o777, 0o600);
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

#[test]
fn a_malformed_trust_store_fails_closed() {
    // 0057 VF15: a corrupt trust store is an honest error — never a
    // silent grant, never a silent wipe-and-grant on top of it.
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let store = state.join("strop").join("trusted-projects");
    std::fs::create_dir_all(store.parent().unwrap()).unwrap();
    std::fs::write(&store, b"[{\"not\": \"closed\"").unwrap();
    let root = Path::new("/tmp/proj-a");
    assert!(is_trusted(Some(&state), root).is_err());
    assert!(
        trust(Some(&state), root).is_err(),
        "a grant does not silently overwrite a store it cannot read"
    );
    assert!(is_trusted(Some(&state), root).is_err());
}

#[test]
fn trust_rejects_unusable_native_paths() {
    // 0057 VF15: invalid native paths cannot manufacture a capability.
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let bad = {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/nul\0byte"))
    };
    assert!(trust(Some(&state), &bad).is_err());
    assert!(
        !trust(Some(&state), Path::new("/tmp/valid")).is_ok()
            || !is_trusted(Some(&state), &bad).unwrap()
    );
}

#[test]
fn cd_saves_the_session_under_the_new_root() {
    let dir = tempfile::tempdir().unwrap();
    let (one, two) = (dir.path().join("one"), dir.path().join("two"));
    std::fs::create_dir(&one).unwrap();
    std::fs::create_dir(&two).unwrap();
    std::fs::write(one.join("a.rs"), "fn a() {}\n").unwrap();
    let state = dir.path().join("state");
    let mut e = Editor::new_in(Buffer::open(one.join("a.rs")).unwrap(), one.clone());
    e.state_dir = Some(state.clone());
    let old = session_path(Some(&state), &one).unwrap();
    let new = session_path(Some(&state), &two).unwrap();
    assert_eq!(capture_save(&e).unwrap().path, old);

    crate::editor::ex::change_directory(&mut e, two.clone());
    e.wait_io().unwrap();
    assert_eq!(capture_save(&e).unwrap().path, new);
    // both projects have a session on disk: the old one as it was left,
    // the new one from the moment the root moved
    assert!(old.exists());
    assert!(new.exists());
    // and a session started in the new root finds the buffer
    let mut restarted = Editor::new_in(Buffer::from_text(""), two);
    restarted.state_dir = Some(state);
    assert!(restore(&mut restarted).unwrap());
    assert_eq!(
        restarted.buf().path.as_deref(),
        Some(one.join("a.rs").as_path())
    );
}

/// 0108 H6: a restored buffer goes through `:e`'s guarded read — a path
/// that has since become a device is refused, never read without end.
#[test]
fn restore_refuses_a_buffer_path_that_became_a_device() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    let mut e = Editor::new(Buffer::open(root.join("a.rs")).unwrap());
    e.cwd = root.to_path_buf();
    e.state_dir = Some(root.join("state"));
    e.feed_text("x");
    save(&e);
    std::fs::remove_file(root.join("a.rs")).unwrap();
    std::os::unix::fs::symlink("/dev/zero", root.join("a.rs")).unwrap();
    let mut e2 = Editor::new(Buffer::from_text(""));
    e2.cwd = root.to_path_buf();
    e2.state_dir = Some(root.join("state"));
    let error = restore(&mut e2).expect_err("a device is not restored");
    assert!(
        error
            .to_string()
            .ends_with("only regular files and directories can be opened"),
        "{error}"
    );
}
