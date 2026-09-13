//! Container attachment and common Directory/read ownership, without native work.
use super::*;
use crate::editor::io::{IoEvent, Opened};
use crate::editor::{Directory, Document, Editor, Key};
use crate::files::FileTarget;
use strop_core::Buffer;
use strop_workspace::{ContainerId, Filesystem, ResourceLocation};

fn identity(id: char) -> strop_containers::ContainerIdentity {
    strop_containers::ContainerIdentity {
        id: id.to_string().repeat(64),
        name: "fixture".into(),
        image: "busybox:latest".into(),
        started_at: "2026-09-09T00:00:00Z".into(),
        user: String::new(),
        workdir: String::new(),
    }
}
fn editor() -> Editor {
    let mut editor = Editor::new_in(Buffer::from_text("origin\n"), "/isolated".into());
    editor.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
        Err(std::io::Error::other(
            "native observation forbidden in ownership fixture",
        ))
    }));
    editor
}
fn deliver(editor: &mut Editor, result: ContainerResult) {
    let ticket = editor.containers.pending.clone().unwrap();
    editor.handle_container_event(Completion {
        ticket,
        outcome: Outcome::Success(result),
    });
}
fn deliver_open(editor: &mut Editor, document: Document, canonical: FileTarget) {
    let (&request, key) = editor.io.open.iter().next().unwrap();
    let ticket = Ticket {
        request,
        key: key.clone(),
    };
    editor.handle_io(IoEvent::Open(Box::new(Completion {
        ticket,
        outcome: Outcome::Success(Opened {
            document,
            canonical,
        }),
    })));
}
fn listing(editor: &mut Editor, path: &str, children: &[(&str, strop_containers::DirEntryKind)]) {
    let container = ContainerId::canonical("a".repeat(64)).unwrap();
    let location = ResourceLocation {
        filesystem: Filesystem::Container(container.clone()),
        path: path.into(),
    };
    let entries = children
        .iter()
        .map(|(name, kind)| strop_containers::DirEntry {
            name: (*name).into(),
            kind: *kind,
            size: None,
        })
        .collect();
    let source = Directory::from_listing(strop_fs::from_container(location, entries).unwrap());
    let document = Document::directory(Buffer::from_text(&source.text()), source);
    deliver_open(
        editor,
        document,
        FileTarget::Container {
            container,
            path: path.into(),
        },
    );
}
fn file(editor: &mut Editor, path: &str, text: &str) {
    let container = ContainerId::canonical("a".repeat(64)).unwrap();
    let document =
        Document::container_file(Buffer::from_text(text), container.clone(), path.into());
    deliver_open(
        editor,
        document,
        FileTarget::Container {
            container,
            path: path.into(),
        },
    );
}

#[test]
fn attached_container_uses_shared_directory_navigation_without_local_paths() {
    let mut editor = editor();
    editor.attach_container("a".repeat(64));
    deliver(&mut editor, ContainerResult::Attached(identity('a')));
    listing(
        &mut editor,
        "/",
        &[("etc", strop_containers::DirEntryKind::Dir)],
    );
    assert!(matches!(
        editor.directory().unwrap().location.filesystem,
        Filesystem::Container(_)
    ));
    assert!(editor.buf().readonly);
    editor.feed(Key::Enter);
    listing(
        &mut editor,
        "/etc",
        &[("init.log", strop_containers::DirEntryKind::File)],
    );
    assert_eq!(
        editor.directory().unwrap().location.path,
        std::path::Path::new("/etc")
    );
    editor.feed(Key::Enter);
    file(&mut editor, "/etc/init.log", "boot ok\n");
    assert_eq!(editor.buf().text(), "boot ok\n");
    assert!(editor.buf().readonly);
    assert!(
        editor.buf().path.is_none(),
        "container bytes never acquire a local write path"
    );
    editor.feed(Key::CtrlO);
    assert_eq!(
        editor.directory().unwrap().location.path,
        std::path::Path::new("/etc")
    );
}

#[test]
fn late_container_inspection_cannot_replace_newer_typing() {
    let mut editor = editor();
    editor.request_containers();
    editor.feed_text("Ityped <esc>");
    deliver(
        &mut editor,
        ContainerResult::Containers(vec![identity('a')]),
    );
    assert!(!editor.picker_open());
    assert_eq!(editor.buf().text(), "typed origin\n");
}

#[test]
fn superseded_container_result_cannot_open_an_old_context() {
    let mut editor = editor();
    editor.attach_container("a".repeat(64));
    let stale = editor.containers.pending.clone().unwrap();
    editor.attach_container("b".repeat(64));
    editor.handle_container_event(Completion {
        ticket: stale,
        outcome: Outcome::Success(ContainerResult::Attached(identity('a'))),
    });
    assert!(editor.io.open.is_empty());
    assert!(editor.containers.attached.is_empty());
    assert_eq!(editor.buf().text(), "origin\n");
}

#[test]
fn container_files_refuse_writes_and_writable_flag_bypasses() {
    let mut editor = editor();
    editor.attach_container_target(
        ContainerId::canonical("a".repeat(64)).unwrap(),
        "/init.log".into(),
        OpenIntent::Switch { readonly: true },
    );
    deliver(&mut editor, ContainerResult::Attached(identity('a')));
    file(&mut editor, "/init.log", "boot ok\n");
    let document = editor.current();
    for command in [
        ":w<cr>",
        ":w!<cr>",
        ":wq!<cr>",
        ":w /local-copy<cr>",
        ":set noro<cr>",
    ] {
        editor.feed_text(command);
        assert_eq!(editor.current(), document);
        assert_eq!(editor.buf().text(), "boot ok\n");
        assert!(editor.buf().readonly);
        assert!(!editor.io_pending());
        assert!(!editor.should_quit);
    }
}
