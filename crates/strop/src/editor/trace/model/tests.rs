use super::*;

fn document() -> Document {
    let mut ids = strop_core::id::Arena::<strop_core::id::DocumentKind, ()>::default();
    Document {
        id: ids.insert(()),
        revision: BufferRevision::new(0),
        text: "aé\n".into(),
    }
}

#[test]
fn kept_mutants_violate_independent_correspondence_checks() {
    let doc = document();
    // Stale delivery: publication bumped the revision, the request's owner
    // is no longer fresh — publishing it must fail.
    let mut model = Model::default();
    model.step(Event::Open(doc.clone())).unwrap();
    let owner = Owner {
        document: Some(doc.id),
        revision: Some(doc.revision),
        scope: serde_json::json!({"surface":1}),
    };
    let request = Request::Worker(1);
    model
        .step(Event::Admit {
            request: request.clone(),
            owner: owner.clone(),
        })
        .unwrap();
    model
        .step(Event::Publish {
            document: doc.id,
            base: doc.revision,
            next: BufferRevision::new(1),
            geometry: Geometry::PreEdit,
            edits: vec![Edit {
                start: 0,
                end: 1,
                text: "b".into(),
            }],
            text: "bé\n".into(),
        })
        .unwrap();
    assert!(model
        .step(Event::Deliver {
            request,
            owner,
            published: true,
            terminal: true
        })
        .is_err());

    // Closing a document a pane still references must fail.
    let mut model = Model::default();
    model.step(Event::Open(doc.clone())).unwrap();
    assert!(model
        .step(Event::Close {
            document: doc.id,
            panes: vec![doc.id]
        })
        .is_err());

    // A "refused" edit that changed bytes must fail.
    let mut model = Model::default();
    model.step(Event::Open(doc.clone())).unwrap();
    let mut changed = doc.clone();
    changed.text = "partial".into();
    assert!(model
        .step(Event::Refused {
            before: doc.clone(),
            after: changed,
            history_before: serde_json::json!([]),
            history_after: serde_json::json!([]),
        })
        .is_err());

    // A publication whose revision disagrees with the edit count must fail
    // even at revision zero.
    let mut model = Model::default();
    model.step(Event::Open(doc.clone())).unwrap();
    assert!(model
        .step(Event::Publish {
            document: doc.id,
            base: doc.revision,
            next: BufferRevision::new(0),
            geometry: Geometry::PreEdit,
            edits: vec![Edit {
                start: 0,
                end: 1,
                text: "b".into()
            }],
            text: "bé\n".into(),
        })
        .is_err());
}

#[test]
fn adjacent_ranges_are_valid_but_overlap_and_utf8_splits_are_not() {
    let edits = vec![
        Edit {
            start: 0,
            end: 1,
            text: "A".into(),
        },
        Edit {
            start: 1,
            end: 3,
            text: "E".into(),
        },
    ];
    assert_eq!(
        apply_text("aé\n", Geometry::PreEdit, &edits).unwrap(),
        "AE\n"
    );
    // 1..2 splits the two-byte é.
    assert!(apply_text(
        "aé\n",
        Geometry::PreEdit,
        &[Edit {
            start: 1,
            end: 2,
            text: "x".into()
        }]
    )
    .is_err());
    assert!(apply_text(
        "abcd",
        Geometry::PreEdit,
        &[
            Edit {
                start: 0,
                end: 3,
                text: "x".into()
            },
            Edit {
                start: 2,
                end: 4,
                text: "y".into()
            },
        ]
    )
    .is_err());
    // Duplicate insertion at the same start conflicts, including empties.
    assert!(apply_text(
        "ab",
        Geometry::PreEdit,
        &[
            Edit {
                start: 1,
                end: 1,
                text: "x".into()
            },
            Edit {
                start: 1,
                end: 1,
                text: "y".into()
            },
        ]
    )
    .is_err());
}

#[test]
fn observe_catches_an_unreported_mutation() {
    let doc = document();
    let mut model = Model::default();
    model.step(Event::Open(doc.clone())).unwrap();
    // A mutation that skipped its Publish hook shows up at the next Observe.
    let mut mutated = doc.clone();
    mutated.text = "aé\nsneaky".into();
    assert!(model
        .step(Event::Observe {
            documents: vec![mutated],
            panes: vec![doc.id]
        })
        .is_err());
    // And the honest observation passes.
    model
        .step(Event::Observe {
            documents: vec![doc.clone()],
            panes: vec![doc.id],
        })
        .unwrap();
}
