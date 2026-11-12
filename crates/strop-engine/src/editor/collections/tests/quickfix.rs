//! Vim's quickfix names over a collection's hits (0094 §3.1, 0097 §3.2).
use super::*;
use strop_core::frontend_input::Key;

fn many(root: &std::path::Path) -> (Editor, Vec<std::path::PathBuf>) {
    let paths: Vec<_> = ["a.txt", "b.txt", "c.txt"]
        .iter()
        .map(|name| root.join(name))
        .collect();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), root.to_path_buf());
    let mut items = Vec::new();
    for path in &paths {
        // 6 KiB of filler: `:s` would plan off the input path here
        let filler = "filler line of text\n".repeat(320);
        std::fs::write(path, format!("old top\nkeep\n{filler}old end old\n")).unwrap();
        e.open_fixture(path).unwrap();
        for (line, text) in [(1, "old top"), (323, "old end old")] {
            items.push(Item {
                badge: None,
                text: format!("{}:{line}", path.display()),
                payload: Payload::Grep {
                    location: strop_workspace::ResourceLocation::local(path.clone()),
                    line,
                    col: 1,
                    match_len: 3,
                    line_text: text.into(),
                },
            });
        }
    }
    e.open_picker(Kind::Search);
    e.picker_items_fixture(items);
    e.feed(Key::CtrlO);
    (e, paths)
}

fn text_at(e: &Editor, path: &std::path::Path) -> String {
    e.docs
        .iter()
        .find_map(|(_, doc)| {
            (doc.buf.path.as_deref() == Some(path)).then(|| doc.buf.text().to_string())
        })
        .unwrap()
}

#[test]
fn quickfix_names_refuse_without_a_list() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    for command in [":copen<cr>", ":cnext<cr>", ":cdo s/a/b/<cr>", "]q"] {
        e.feed_text(command);
        assert!(
            e.message.contains("nothing collected yet"),
            "{command}: {}",
            e.message
        );
    }
}

/// Outside the collection the keys visit each hit in its file, counted
/// from the current hit; `:copen` comes back to it.
#[test]
fn hits_are_visited_in_their_files_and_copen_returns_to_the_current_one() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, paths) = many(dir.path());
    let list = e.current();
    // collecting put the cursor on hit 1; leave the collection
    e.feed(Key::CtrlO);
    assert_ne!(e.current(), list);
    e.feed_text("]q");
    assert_eq!(e.buf().path.as_deref(), Some(paths[0].as_path()));
    assert_eq!(e.buf().line_of(e.head()), 322);
    assert_eq!(e.message, "hit 2 of 6: a.txt:323");
    e.feed_text(":cnext<cr>");
    assert_eq!(e.buf().path.as_deref(), Some(paths[1].as_path()));
    assert_eq!(e.buf().line_of(e.head()), 0);
    e.feed_text("[q");
    assert_eq!(e.buf().path.as_deref(), Some(paths[0].as_path()));
    e.feed_text(":cc 6<cr>");
    assert_eq!(e.buf().path.as_deref(), Some(paths[2].as_path()));
    assert_eq!(e.buf().line_of(e.head()), 322);
    e.feed_text("]q");
    assert_eq!(e.message, "last hit");
    e.feed_text(":cc 7<cr>");
    assert_eq!(e.message, "there are 6 hits");
    e.feed_text(":cc 1<cr>[q");
    assert_eq!(e.message, "first hit");
    e.feed_text(":cc 4<cr>:copen<cr>");
    assert_eq!(e.current(), list);
    assert_eq!(e.head(), e.collection_hits(list)[3].view);
    assert!(e.message.starts_with("6 hits in 3 files"), "{}", e.message);
}

#[test]
fn cdo_substitutes_at_every_hit_line_and_cfdo_once_per_file() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, paths) = many(dir.path());
    let list = e.current();
    e.feed_text(":cdo s/old/new/<cr>");
    assert_eq!(e.current(), list, "focus returns to where it was");
    assert!(
        e.message.contains("ran on 6 lines in 3 files"),
        "{}",
        e.message
    );
    for path in &paths {
        let text = text_at(&e, path);
        assert!(text.starts_with("new top\nkeep\n"), "first hit line");
        assert!(
            text.ends_with("new end old\n"),
            "one substitution per hit line"
        );
    }
    assert!(e.buf().text().to_string().contains("new top"));

    e.feed_text(":cfdo %s/old/OLD/g<cr>");
    assert!(e.message.contains("ran in 3 files"), "{}", e.message);
    for path in &paths {
        assert!(text_at(&e, path).ends_with("new end OLD\n"));
    }

    // `:cfdo w` writes each file through its own save
    e.feed_text(":cfdo w<cr>");
    e.wait_io().unwrap();
    for path in &paths {
        assert!(std::fs::read_to_string(path)
            .unwrap()
            .ends_with("new end OLD\n"));
    }
    assert!(e.docs.iter().all(|(_, doc)| !doc.buf.is_dirty()));
}

#[test]
fn cdo_needs_a_command_and_does_not_nest() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, _) = many(dir.path());
    e.feed_text(":cdo<cr>");
    assert!(e.message.contains("needs a command"), "{}", e.message);
    e.feed_text(":cdo cdo s/a/b/<cr>");
    assert!(e.message.contains("does not nest"), "{}", e.message);
}

/// `:cdo` runs whatever the `:` line runs (0095 §2.6): `:norm` types
/// Normal-mode keys at each hit, `:d` deletes each hit line, and one
/// `u` in a file takes back one hit's command.
#[test]
fn cdo_runs_normal_keys_and_line_commands_at_every_hit() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, paths) = many(dir.path());
    let list = e.current();
    e.feed_text(":cdo norm A;<cr>");
    assert_eq!(e.current(), list, "focus returns to where it was");
    assert_eq!(e.mode, crate::editor::Mode::Normal);
    assert!(
        e.message.contains("ran on 6 lines in 3 files"),
        "{}",
        e.message
    );
    for path in &paths {
        let text = text_at(&e, path);
        assert!(text.starts_with("old top;\nkeep\n"), "{text:.20}");
        assert!(text.ends_with("old end old;\n"));
    }
    assert!(e.buf().text().to_string().contains("old top;"));

    // hit 1 in the collection, Enter to its file, and undo there
    e.feed_text(":cc 1<cr>");
    e.feed(Key::Enter);
    e.feed_text("u");
    let text = text_at(&e, &paths[0]);
    assert!(text.ends_with("old end old\n"), "u undid the newest hit");
    assert!(text.starts_with("old top;\n"), "and only that one");

    e.feed_text(":cdo d<cr>");
    for path in &paths {
        let text = text_at(&e, path);
        assert!(text.starts_with("keep\n"), "{text:.20}");
        assert!(!text.contains("old"));
    }
}
