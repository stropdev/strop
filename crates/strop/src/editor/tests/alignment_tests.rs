use super::*;

/// Opens, surfaces, and closes never panic and keep the document
/// set honest (pre-0.4.1 this pinned the parallel-vectors alignment;
/// the Document struct made that invariant the type system).
#[test]
fn document_set_stays_honest() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("align-a.rs");
    let b = dir.path().join("align-b.rs");
    std::fs::write(&a, "a\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    let mut e = Editor::new(Buffer::open(a.to_str().unwrap()).unwrap());
    e.open_fixture(&b).unwrap();
    assert_eq!(e.docs.len(), 2);
    e.fixture_git_context();
    e.open_diff_surface("delta", "f.rs", vec![], None);
    assert_eq!(e.docs.len(), 3);
    assert!(e.cur().surface_payload().is_some());
    e.close_buffer(true);
    assert_eq!(e.docs.len(), 2);
    e.close_buffer(true);
    e.close_buffer(true);
    assert!(e.should_quit, "closing the last document quits");
}
