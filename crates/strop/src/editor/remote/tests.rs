use super::*;
use crate::editor::document::RemoteDocument;
use strop_core::Buffer;
use strop_remote::{ReadSelection, RemoteSize, RemoteWindow};

fn document(text: &str, selection: ReadSelection) -> Document {
    Document::remote(
        Buffer::from_text(text),
        RemoteDocument {
            file: RemoteFile::parse("ssh://fixture/repo/app.log").unwrap(),
            window: RemoteWindow::resolve(&selection, RemoteSize::new(text.len() as u64)),
            selection,
            connection: None,
            return_to: None,
            write: None,
        },
    )
}
fn editor(text: &str) -> Editor {
    let mut editor = Editor::new_in(Buffer::from_text("keepalive\n"), "/isolated".into());
    editor.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
        Err(std::io::Error::other("native work forbidden"))
    }));
    let id = editor.docs.insert(document(text, ReadSelection::Full));
    editor.switch_to(id);
    editor
}

#[test]
fn follow_moves_the_eof_cursor_but_not_marks_or_jump_history() {
    let mut e = editor("first\nend\n");
    let id = e.current();
    let old_tail = 8; // final character of "end"
    e.set_head(old_tail);
    e.feed_text("ma");
    e.push_jump();
    e.publish_remote_snapshot(id, document("first\nend\nnew\n", ReadSelection::Full), true)
        .unwrap();
    assert_eq!(e.head(), 12); // final character of the appended "new"
    e.jump_back();
    assert_eq!(e.head(), old_tail);
    e.feed_text("`a");
    assert_eq!(e.head(), old_tail);
}

#[test]
fn browsing_does_not_become_eof_stickiness() {
    let mut e = editor("first\nend\n");
    let id = e.current();
    e.set_head(1);
    e.publish_remote_snapshot(id, document("first\nend\nnew\n", ReadSelection::Full), true)
        .unwrap();
    assert_eq!(e.head(), 1);
}

#[test]
fn refresh_clamps_the_column_within_the_original_line() {
    let mut e = editor("abcdefgh\nsecond\nthird\n");
    let id = e.current();
    e.set_head(6);
    e.publish_remote_snapshot(
        id,
        document("x\nsecond\nthird\n", ReadSelection::Full),
        false,
    )
    .unwrap();
    assert_eq!(e.buf().line_of(e.head()), 0);
    assert_eq!(e.head(), 0);
}

#[test]
fn shrink_clamps_browsing_to_a_real_grapheme() {
    let mut e = editor("first\nold tail\n");
    let id = e.current();
    e.set_head(8);
    e.publish_remote_snapshot(id, document("e\u{301}\n", ReadSelection::Full), true)
        .unwrap();
    assert_eq!(
        e.head(),
        0,
        "neither the virtual EOF row nor a combining codepoint is a cursor target"
    );
}

#[test]
fn same_path_snapshots_never_share_cached_gutters() {
    let mut e = editor("abcdef\n");
    let full = e.current();
    e.blame_gutters.insert(
        full,
        crate::editor::BlameGutter {
            lines: vec![strop_git::memory::BlameLine {
                sha: "1234".into(),
                author: "first snapshot".into(),
                age: "1d".into(),
                ts: 1,
            }],
            revision: e.buf().revision(),
            request: None,
        },
    );
    // A tail may cover an entire small file. Same path, line count and buffer
    // revision still do not make two captured contents the same document.
    let tail = e.docs.insert(document(
        "uvwxyz\n",
        ReadSelection::Tail(ReadLimit::new(7).unwrap()),
    ));
    assert!(e.blame_gutter_for(full).is_some());
    assert!(e.blame_gutter_for(tail).is_none());
}

#[test]
fn historical_permalink_uses_the_rendered_source_coordinate() {
    let mut e = editor("working\n");
    let repo = strop_git::RepoTarget::Remote {
        endpoint: RemoteEndpoint::parse("ssh://fixture").unwrap(),
        workdir: "/repo".into(),
    };
    e.git = Some(strop_git::GitContext {
        repo: repo.clone(),
        head_sha: None,
        head_branch: None,
        remotes: vec![(
            "origin".into(),
            "https://github.com/acme/project.git".into(),
        )],
    });
    let hunk = strop_git::Hunk {
        kind: strop_git::HunkKind::Change,
        old_start: 40,
        old_count: 1,
        new_start: 40,
        new_count: 1,
        lines: vec![
            strop_git::DiffLine {
                origin: strop_git::LineOrigin::Deletion,
                old_lineno: Some(40),
                new_lineno: None,
                text: b"old".to_vec(),
                has_newline: true,
            },
            strop_git::DiffLine {
                origin: strop_git::LineOrigin::Addition,
                old_lineno: None,
                new_lineno: Some(40),
                text: b"new".to_vec(),
                has_newline: true,
            },
        ],
    };
    e.open_delta(
        "delta",
        crate::editor::git_memory::PreparedDiff::new("app.log".into(), vec![hunk]),
        None,
        Some(crate::editor::git_memory::CommitFiles {
            repo,
            sha: "0123456789abcdef0123456789abcdef01234567".into(),
            files: crate::editor::git_memory::PreparedFiles::new(
                "0123456789abcdef0123456789abcdef01234567".into(),
                Vec::new(),
            ),
            current: "app.log".into(),
        }),
    );
    e.set_head(e.buf().line_start(3));
    e.yank_permalink();
    assert!(
        e.register(None).text.ends_with("/app.log#L40"),
        "{}: {}",
        e.message,
        e.register(None).text
    );
    e.set_head(e.buf().line_start(2));
    assert!(
        e.build_permalink(crate::editor::permalink::PermalinkIntent::Yank)
            .is_err(),
        "a deleted line has no location in the selected commit"
    );
}

#[test]
fn directory_metadata_preserves_unknown_zero_and_native_row_identity() {
    use crate::editor::document::RemoteDirectory;
    use strop_remote::{RemoteEntry, RemoteEntryKind, RemotePermissions};
    let root = RemoteFile::parse("ssh://fixture/repo").unwrap();
    let entries = vec![
        RemoteEntry {
            file: root.with_path("/repo/missing".into()).unwrap(),
            kind: RemoteEntryKind::Unknown,
            permissions: None,
            size: None,
        },
        RemoteEntry {
            file: root.with_path("/repo/zero".into()).unwrap(),
            kind: RemoteEntryKind::File,
            permissions: Some(RemotePermissions::new(0).unwrap()),
            size: Some(RemoteSize::new(0)),
        },
        RemoteEntry {
            file: root.with_path("/repo/line\nbreak".into()).unwrap(),
            kind: RemoteEntryKind::SymbolicLink,
            permissions: Some(RemotePermissions::new(0o777).unwrap()),
            size: Some(RemoteSize::new(7)),
        },
    ];
    let directory = RemoteDirectory {
        directory: root,
        entries: entries.into(),
        visible: vec![0, 1, 2],
        filter: String::new(),
        connection: None,
        return_to: None,
    };
    let mut editor = editor("origin\n");
    let document = Document::directory(Buffer::from_text(&directory.text()), directory);
    let id = editor.docs.insert(document);
    editor.switch_to(id);
    let frame = crate::headless::frame_string(&mut editor, 80, 10).unwrap();
    assert!(frame.contains("??????????     ? missing"), "{frame}");
    assert!(frame.contains("----------     0 zero"), "{frame}");
    assert!(frame.contains("lrwxrwxrwx     7 line�break@"), "{frame}");
    editor.feed_text("3Gyy");
    assert!(editor
        .register(None)
        .text
        .starts_with("----------     0 zero"));
}
