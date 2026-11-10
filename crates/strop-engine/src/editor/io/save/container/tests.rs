use super::*;
use crate::editor::document::Document;
use crate::editor::namespace::{StoreDispatch, StoreOutcome, WorkspaceCapability};
use strop_core::Buffer;
use strop_worker_client::{Transport, Worker};
use strop_workspace::ContainerId;

fn identity() -> strop_containers::ContainerIdentity {
    strop_containers::ContainerIdentity {
        id: "a".repeat(64),
        name: "fixture".into(),
        image: "busybox:latest".into(),
        started_at: "2026-09-09T00:00:00Z".into(),
        user: String::new(),
        workdir: "/workspace".into(),
        env: Vec::new(),
    }
}

fn pipe_worker() -> Worker {
    Worker::connect_with(|| {
        let (client_read, worker_write) = std::io::pipe()?;
        let (worker_read, client_write) = std::io::pipe()?;
        std::thread::spawn(move || {
            strop_worker::serve::run(worker_read, worker_write).unwrap();
        });
        Ok(Transport {
            reader: Box::new(client_read),
            writer: Box::new(client_write),
            child: None,
            stderr: None,
        })
    })
}

struct Fixture {
    editor: Editor,
    document: DocumentId,
    identity: strop_containers::ContainerIdentity,
    path: std::path::PathBuf,
    worker: Worker,
    _directory: tempfile::TempDir,
}

fn fixture(text: &str) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, text).unwrap();
    let identity = identity();
    let container = ContainerId::canonical(identity.id.clone()).unwrap();
    let mut editor = Editor::new(Buffer::from_text("origin\n"));
    editor.config.completion.enabled = false;
    let document = editor
        .docs
        .try_insert(Document::container_file(
            Buffer::from_text(text),
            container,
            path.clone(),
        ))
        .unwrap();
    editor.switch_to(document);
    editor
        .containers
        .attached
        .insert(identity.id.clone(), identity.clone());
    let worker = pipe_worker();
    editor
        .containers
        .workers
        .install_test(&identity, worker.clone());
    Fixture {
        editor,
        document,
        identity,
        path,
        worker,
        _directory: directory,
    }
}

fn enable(fixture: &mut Fixture) {
    fixture.editor.enable_container_edit().unwrap();
    fixture.editor.wait_io().unwrap();
    assert!(!fixture.editor.buf().readonly);
    assert!(fixture
        .editor
        .current_namespace_capabilities()
        .get(WorkspaceCapability::Write)
        .available());
}

#[test]
fn displayed_snapshot_permit_admits_real_edit_and_conditional_save() {
    let mut fixture = fixture("before\n");
    enable(&mut fixture);
    fixture.editor.feed_text("A after<esc>");
    assert!(fixture.editor.buf().is_dirty());
    assert!(fixture
        .editor
        .request_save_document(fixture.document, None, false, false,));
    fixture.editor.wait_io().unwrap();
    assert_eq!(std::fs::read(&fixture.path).unwrap(), b"before after\n");
    assert!(!fixture.editor.buf().is_dirty());
    assert_eq!(fixture.editor.message, "written in container");
    fixture.worker.shutdown().unwrap();
}

#[test]
fn changed_stored_bytes_cannot_gain_an_edit_permit() {
    let mut fixture = fixture("displayed\n");
    std::fs::write(&fixture.path, b"changed elsewhere\n").unwrap();
    fixture.editor.enable_container_edit().unwrap();
    fixture.editor.wait_io().unwrap();
    assert!(fixture.editor.buf().readonly);
    assert!(fixture.editor.message.contains("Conflict"));
    assert_eq!(
        std::fs::read(&fixture.path).unwrap(),
        b"changed elsewhere\n"
    );
    fixture.worker.shutdown().unwrap();
}

#[test]
fn worker_refuses_a_location_from_another_container() {
    let fixture = fixture("before\n");
    let bound = fixture
        .editor
        .containers
        .workers
        .get(&fixture.identity)
        .unwrap();
    let other = ResourceLocation {
        filesystem: strop_workspace::Filesystem::Container(
            ContainerId::canonical("b".repeat(64)).unwrap(),
        ),
        path: fixture.path.clone(),
    };
    let (token, _owner) = worker::CancelToken::standalone();
    let result = store::prepare_edit(
        bound,
        &other,
        &fixture.identity.started_at,
        &Rope::from_str("before\n"),
        &token,
    );
    assert!(matches!(
        result,
        Err(ContainerSaveError::Refused {
            kind: FsFailureKind::Protocol,
            ..
        })
    ));
    assert_eq!(std::fs::read(&fixture.path).unwrap(), b"before\n");
    fixture.worker.shutdown().unwrap();
}

#[test]
fn recycled_incarnation_cannot_borrow_document_permit() {
    let mut fixture = fixture("before\n");
    enable(&mut fixture);
    fixture.editor.feed_text("Adraft");
    let mut recycled = fixture.identity.clone();
    recycled.started_at = "2026-09-10T00:00:00Z".into();
    fixture
        .editor
        .containers
        .attached
        .insert(recycled.id.clone(), recycled);
    assert!(!fixture
        .editor
        .request_save_document(fixture.document, None, false, false,));
    assert!(fixture.editor.message.contains("recycled"));
    assert_eq!(std::fs::read(&fixture.path).unwrap(), b"before\n");
    assert!(fixture.editor.buf().is_dirty());
    assert!(!fixture
        .editor
        .current_namespace_capabilities()
        .get(WorkspaceCapability::Write)
        .available());
    fixture.worker.shutdown().unwrap();
}

#[test]
fn moved_baseline_conflicts_without_overwriting_foreign_bytes() {
    let mut fixture = fixture("before\n");
    enable(&mut fixture);
    std::fs::write(&fixture.path, b"foreign\n").unwrap();
    fixture.editor.feed_text("Adraft");
    assert!(fixture
        .editor
        .request_save_document(fixture.document, None, false, false,));
    fixture.editor.wait_io().unwrap();
    assert_eq!(std::fs::read(&fixture.path).unwrap(), b"foreign\n");
    assert!(fixture.editor.buf().is_dirty());
    assert!(fixture.editor.message.contains("Conflict"));
    fixture.worker.shutdown().unwrap();
}

#[test]
fn unconfirmed_save_verifies_frozen_receipt_instead_of_rewriting() {
    let mut fixture = fixture("before\n");
    enable(&mut fixture);
    fixture.editor.feed_text("A after<esc>");
    fixture.editor.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
        Ok(serde_json::Value::Bool(false))
    }));
    assert!(fixture
        .editor
        .request_save_document(fixture.document, None, false, false,));
    let ticket = fixture.editor.io.container_writes.pending[&fixture.document].clone();
    let attempt = &fixture.editor.io.container_writes.attempts[&fixture.document];
    let before = attempt.before.clone();
    let contents = attempt.contents.clone();
    let digest = store::digest_of(&contents);
    let intent = strop_workspace::operation::OperationIntent {
        kind: strop_workspace::operation::OperationKind::Store,
        source: None,
        destination: Some(before.location.clone()),
        copy_version: strop_workspace::operation::CopyVersion::Stored,
        expected_content: Some(digest),
        store: Some(strop_workspace::operation::StorePolicy {
            baseline: Some(before.modified),
            baseline_object: before.identity,
            baseline_attributes: before.attributes,
            force: false,
            expect_absent: false,
            displayed: Some(before.content),
        }),
    };
    let bound = fixture
        .editor
        .containers
        .workers
        .get(&fixture.identity)
        .unwrap();
    let namespace = bound.worker().namespace().unwrap();
    let dispatch = StoreDispatch::Container(bound.clone());
    let (token, _owner) = worker::CancelToken::standalone();
    let prepared = dispatch.prepare_store(intent, &token).unwrap();
    let operation = prepared.operation.clone();
    let receipt = match dispatch.apply_store(prepared, contents.to_string().as_bytes(), &token) {
        StoreOutcome::Receipt(receipt) => *receipt,
        StoreOutcome::Refused(error) => panic!("store refused: {error}"),
    };
    assert!(matches!(receipt.outcome, StepOutcome::Committed { .. }));
    assert_eq!(std::fs::read(&fixture.path).unwrap(), b"before after\n");
    let receipt = StepReceipt {
        operation,
        outcome: StepOutcome::Unconfirmed {
            detail: "test lost the committed reply".into(),
            observed_destination: None,
            recovery: None,
            publication: None,
        },
        ..receipt
    };
    fixture
        .editor
        .io
        .container_writes
        .attempts
        .get(&fixture.document)
        .unwrap()
        .prepared
        .lock()
        .replace((namespace.clone(), receipt.operation.clone()));
    fixture.editor.container_write_done(Completion {
        ticket: ticket.clone(),
        outcome: Outcome::Success(ContainerWriteResult::StoreUncertain {
            detail: "lost reply".into(),
            receipt: Box::new(receipt.clone()),
        }),
    });
    assert!(fixture.editor.buf().is_dirty());
    assert!(fixture.editor.message.contains("unconfirmed"));

    fixture.editor.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
        Ok(serde_json::Value::Bool(true))
    }));
    let verification = store::verify(bound, &before, receipt, namespace, &token).unwrap();
    let store::WorkerVerification::Committed(verified) = verification else {
        panic!("verification did not commit")
    };
    fixture
        .editor
        .accept_container_receipt(ticket.key, *verified);
    assert!(
        !fixture.editor.buf().is_dirty(),
        "verified completion left dirty buffer: {}",
        fixture.editor.message
    );
    assert_eq!(fixture.editor.message, "written in container");
    assert_eq!(std::fs::read(&fixture.path).unwrap(), b"before after\n");
    fixture.worker.shutdown().unwrap();
}
