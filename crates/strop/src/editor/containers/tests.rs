//! Container attach/browse ownership: injected completions, no engine.

use super::*;
use crate::editor::Editor;
use strop_core::worker::Outcome;
use strop_core::Buffer;

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

fn entry(name: &str, kind: strop_containers::DirEntryKind) -> strop_containers::DirEntry {
    strop_containers::DirEntry {
        name: name.into(),
        kind,
        size: None,
    }
}

fn deliver(e: &mut Editor, result: ContainerResult) {
    let ticket = e.containers.pending.clone().expect("request in flight");
    e.handle_container_event(Completion {
        ticket,
        outcome: Outcome::Success(result),
    });
}

#[test]
fn discover_offers_and_attach_binds_a_container_workspace() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.request_containers();
    deliver(&mut e, ContainerResult::Containers(vec![identity('a')]));
    // picker offers the container; accepting attaches and lists root
    e.accept_picker(strop_picker::Payload::Container("a".repeat(64)), None);
    assert!(matches!(
        e.containers.pending.as_ref().map(|t| &t.key.job),
        Some(ContainerJob::Attach { .. })
    ));
    deliver(
        &mut e,
        ContainerResult::Listing {
            identity: identity('a'),
            path: "/".into(),
            entries: vec![entry("etc", strop_containers::DirEntryKind::Dir)],
        },
    );
    let text = e.buf().text().to_string();
    assert!(text.contains("container fixture"), "{text}");
    assert!(text.contains("d etc/"), "{text}");
    assert!(e.buf().readonly, "listings are read-only");
    // the workspace registry bound the container namespace
    let bound = e.workspaces.iter().any(|(_, context)| {
        matches!(
            context.filesystem,
            strop_workspace::Filesystem::Container(_)
        )
    });
    assert!(bound, "attach binds a container context");
}

#[test]
fn enter_on_a_listing_row_requests_descent_or_read() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.request_containers();
    deliver(&mut e, ContainerResult::Containers(vec![identity('a')]));
    e.attach_container("a".repeat(64));
    deliver(
        &mut e,
        ContainerResult::Listing {
            identity: identity('a'),
            path: "/".into(),
            entries: vec![
                entry("etc", strop_containers::DirEntryKind::Dir),
                entry("init.log", strop_containers::DirEntryKind::File),
            ],
        },
    );
    e.close_picker(); // accept_picker closes it in production
                      // row 1 (line 2): the file → a bounded read job
    e.set_head(e.buf().line_start(2));
    e.feed(crate::editor::Key::Enter);
    assert!(matches!(
        e.containers.pending.as_ref().map(|t| &t.key.job),
        Some(ContainerJob::ReadFile { path, .. }) if path == "/init.log"
    ));
    deliver(
        &mut e,
        ContainerResult::File {
            identity: identity('a'),
            path: "/init.log".into(),
            text: "boot ok\n".into(),
        },
    );
    assert_eq!(e.buf().text().to_string(), "boot ok\n");
    assert!(e.buf().readonly, "container files are read-only");
}

#[test]
fn a_stale_completion_changes_nothing() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.request_containers();
    let stale = e.containers.pending.clone().unwrap();
    e.containers.pending = None; // superseded
    e.handle_container_event(Completion {
        ticket: stale,
        outcome: Outcome::Success(ContainerResult::Containers(vec![identity('a')])),
    });
    assert!(!e.picker_open(), "no picker from a stale answer");
    assert_eq!(e.buf().text().to_string(), "x\n");
}

#[test]
fn engine_failure_surfaces_on_the_status_line() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.request_containers();
    let ticket = e.containers.pending.clone().unwrap();
    e.handle_container_event(Completion {
        ticket,
        outcome: Outcome::failed(
            FailureKind::Io,
            "docker engine unavailable: is docker running?".to_string(),
        ),
    });
    assert!(e.message.contains("docker engine unavailable"));
}

#[test]
fn container_files_refuse_every_write_form() {
    // 0037 DC1b write policy: no in-container save, no :w! bypass,
    // never a local-path fallback
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.request_containers();
    deliver(&mut e, ContainerResult::Containers(vec![identity('a')]));
    e.close_picker();
    e.attach_container("a".repeat(64));
    deliver(
        &mut e,
        ContainerResult::File {
            identity: identity('a'),
            path: "/init.log".into(),
            text: "boot ok\n".into(),
        },
    );
    e.feed_text(":w\r");
    assert!(e.message.contains("read-only"), "{}", e.message);
    e.feed_text(":w!\r");
    assert!(
        e.message.contains("read-only"),
        "w! refuses too: {}",
        e.message
    );
    e.feed_text(":wq!\r");
    assert!(e.message.contains("read-only"), "{}", e.message);
    assert!(!e.should_quit);
}
