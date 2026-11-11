//! Plan 0092 §2: the pinned project owns its library files and its
//! crashed servers restart.
use super::*;

fn library_editor() -> (
    Editor,
    strop_core::id::DocumentId,
    strop_core::id::DocumentId,
) {
    let mut buffer = Buffer::from_text("fn main() {}\n");
    buffer.path = Some(PathBuf::from("/workspace/src/main.rs"));
    let mut e = Editor::new_in(buffer, PathBuf::from("/workspace"));
    let origin = e.current();
    let library = insert_target(
        &mut e,
        "/home/u/.rustup/toolchains/stable/lib/rustlib/src/rust/library/core/src/option.rs",
        "pub enum Option<T> {}\n",
    );
    e.lsp_state.attach.attached.push(super::attach::Attachment {
        language: "rust".into(),
        root: PathBuf::from("/workspace"),
        server: ServerId::new(7),
        target: strop_workspace::Filesystem::Local,
    });
    (e, origin, library)
}

fn served_by(e: &Editor, document: strop_core::id::DocumentId) -> Option<(ServerId, PathBuf)> {
    let location = e.lsp_document_location(document)?;
    e.lsp_server_for(document, &location.path, "rust", &location.filesystem)
}

#[test]
fn a_library_file_opened_without_a_jump_belongs_to_the_projects_server() {
    // 0092 §2.3: the stdlib file was reached by the file finder, `:e`, a
    // restored session — no navigation context — and still resolves to
    // the one server the project runs, never to "no server" (which sent
    // discovery off to root a second server in the library directory).
    let (mut e, origin, library) = library_editor();
    assert_eq!(
        served_by(&e, library),
        Some((ServerId::new(7), PathBuf::from("/workspace")))
    );
    assert_eq!(
        served_by(&e, origin),
        Some((ServerId::new(7), PathBuf::from("/workspace")))
    );
    // A different language's server is not an owner.
    let location = e.lsp_document_location(library).unwrap();
    assert_eq!(
        e.lsp_server_for(library, &location.path, "python", &location.filesystem),
        None
    );
    // A server rooted outside the pinned project is not the project's.
    e.lsp_state.attach.attached[0].root = PathBuf::from("/elsewhere");
    assert_eq!(served_by(&e, library), None);
}

#[test]
fn a_crashed_server_restarts_within_its_budget_and_says_so() {
    use super::status::RESTART_BUDGET;
    let (mut e, origin, library) = library_editor();
    let crash = |e: &mut Editor, id: u64| {
        let server = ServerId::new(id);
        e.lsp_state.attach.attached.clear();
        e.lsp_state.attach.attached.push(super::attach::Attachment {
            language: "rust".into(),
            root: PathBuf::from("/workspace"),
            server,
            target: strop_workspace::Filesystem::Local,
        });
        e.lsp_servers.push(LspServer {
            id: server,
            name: "rust-analyzer".into(),
            language: "rust".into(),
            root: PathBuf::from("/workspace"),
            target: strop_workspace::Filesystem::Local,
            client: None,
            rx: std::sync::mpsc::channel().1,
            ready: true,
        });
        for document in [origin, library] {
            let location = e.lsp_document_location(document).unwrap();
            let revision = e.docs.get(document).unwrap().buf.revision();
            e.lsp_state.bindings.insert(
                document,
                state::Binding {
                    server,
                    path: location.path,
                    root: PathBuf::from("/workspace"),
                    language: "rust".into(),
                    target: strop_workspace::Filesystem::Local,
                    revision,
                },
            );
        }
        e.handle_lsp_event(LspEvent::Failed {
            server,
            name: "rust-analyzer".into(),
            hint: "exit status: 101".into(),
        });
    };
    for attempt in 1..=RESTART_BUDGET {
        crash(&mut e, 10 + u64::from(attempt));
        assert!(
            e.message
                .contains(&format!("restarting ({attempt}/{RESTART_BUDGET})")),
            "{}",
            e.message
        );
        assert!(e.message.contains(":explain"), "{}", e.message);
        let restart = &e.lsp_state.restarts[0];
        assert_eq!(restart.attempts, attempt);
        assert_eq!(
            restart.documents,
            vec![origin, library],
            "the library file rides the restart with the project file"
        );
        assert_eq!(e.lsp_state.failures.len(), 1, "the failure stays visible");
        assert!(e.lsp_state.bindings.is_empty());
    }
    crash(&mut e, 99);
    assert!(e.message.contains("not restarted"), "{}", e.message);
    assert!(e.lsp_state.restarts[0].documents.is_empty());
    // A server that never finished its handshake is a start failure,
    // not a crash: restarting it would only fail the same way.
    let (mut e, origin, _) = library_editor();
    let server = ServerId::new(7);
    e.lsp_servers.push(LspServer {
        id: server,
        name: "rust-analyzer".into(),
        language: "rust".into(),
        root: PathBuf::from("/workspace"),
        target: strop_workspace::Filesystem::Local,
        client: None,
        rx: std::sync::mpsc::channel().1,
        ready: false,
    });
    let _ = origin;
    e.handle_lsp_event(LspEvent::Failed {
        server,
        name: "rust-analyzer".into(),
        hint: "cannot run".into(),
    });
    assert!(e.lsp_state.restarts.is_empty());
    assert!(e.message.contains("failed — :explain"), "{}", e.message);
}

#[test]
fn cd_moves_the_project_whose_server_owns_outside_files() {
    // The "outside the project" test reads the root at each use: after
    // `:cd` the old project's server no longer claims the library file,
    // and a server rooted in the new project does.
    let (mut e, _origin, library) = library_editor();
    assert!(served_by(&e, library).is_some());
    crate::editor::ex::change_directory(&mut e, PathBuf::from("/elsewhere"));
    assert_eq!(e.cwd, PathBuf::from("/elsewhere"));
    assert_eq!(served_by(&e, library), None);
    e.lsp_state.attach.attached.push(super::attach::Attachment {
        language: "rust".into(),
        root: PathBuf::from("/elsewhere/crate"),
        server: ServerId::new(9),
        target: strop_workspace::Filesystem::Local,
    });
    assert_eq!(
        served_by(&e, library),
        Some((ServerId::new(9), PathBuf::from("/elsewhere/crate")))
    );
}
