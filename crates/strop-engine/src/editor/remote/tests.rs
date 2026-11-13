use super::*;

use crate::editor::test_support::remote::{document, snapshot_editor as editor};

#[test]
fn workspace_picker_selects_the_exact_remote_project_root() {
    let mut e = editor("remote\n");
    e.open_remote_picker();
    let root = e
        .picker
        .as_ref()
        .unwrap()
        .picker
        .items
        .iter()
        .find_map(|item| match &item.payload {
            strop_picker::Payload::WorkspaceRoot(root) => Some(root.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(root.path, std::path::Path::new("/repo"));
    e.accept_picker(strop_picker::Payload::WorkspaceRoot(root.clone()), None);
    assert!(e.workspaces.resolve(&root).is_some());
    assert_eq!(e.message, "project root selected");
}

#[test]
fn refresh_clamps_the_column_within_the_original_line() {
    let mut e = editor("abcdefgh\nsecond\nthird\n");
    let id = e.current();
    e.set_head(6);
    e.publish_source_snapshot(id, document("x\nsecond\nthird\n"))
        .unwrap();
    assert_eq!(e.buf().line_of(e.head()), 0);
    assert_eq!(e.head(), 0);
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
    // Same path, line count and buffer revision still do not make two
    // captured contents the same document.
    let other = e.docs.try_insert(document("uvwxyz\n")).unwrap();
    assert!(e.blame_gutter_for(full).is_some());
    assert!(e.blame_gutter_for(other).is_none());
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
                strop_git::memory::CommitSummary {
                    sha: "0123456789abcdef0123456789abcdef01234567".into(),
                    ..Default::default()
                },
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
