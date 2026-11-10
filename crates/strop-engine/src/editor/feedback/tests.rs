use strop_core::Buffer;
use strop_workspace::RemoteEndpoint;

use super::model::{FeedbackEvent, FeedbackKind, TypedAction};
use super::store::{FeedbackStore, FEEDBACK_HISTORY_BOUND};
use super::{FeedbackLifecycle, FeedbackOwner, FeedbackSeverity};
use crate::editor::{Document, Editor};

fn failed_scratch_save(editor: &mut Editor) -> strop_core::id::DocumentId {
    let document = editor.current();
    assert!(
        !editor.request_save_document(document, None, false, false),
        "a scratch buffer without a target refuses save preparation"
    );
    assert!(editor.feedback.unresolved().any(|record| {
        record.owner()
            == &FeedbackOwner::Save {
                document,
                remote: false,
            }
    }));
    document
}

#[test]
fn failed_save_survives_keys_unrelated_success_and_explain() {
    let mut editor = Editor::new(Buffer::from_text("scratch\n"));
    let failed = failed_scratch_save(&mut editor);
    let expected_detail = editor.feedback()[0].detail().to_string();

    editor.feed_text("gg/missing-search<esc>hl");
    editor.message = "an unrelated ordinary confirmation".into();
    let other = editor
        .docs
        .try_insert(Document::scratch(Buffer::from_text("other\n")))
        .unwrap();
    let revision = editor.doc(other).buf.revision();
    editor.note_save_failure(
        other,
        false,
        revision,
        None,
        false,
        false,
        true,
        "other owner failed".into(),
    );
    editor.resolve_save_feedback(other, false);

    let failed_record = editor
        .feedback
        .unresolved()
        .find(|record| {
            record.owner()
                == &FeedbackOwner::Save {
                    document: failed,
                    remote: false,
                }
        })
        .unwrap();
    assert_eq!(failed_record.detail(), expected_detail);
    assert_eq!(
        editor.presented_feedback().unwrap().id(),
        failed_record.id()
    );

    editor.switch_to(failed);
    editor.open_explain();
    let text = editor.buf().text().to_string();
    assert!(text.contains("[feedback]"), "{text}");
    assert!(text.contains(&expected_detail), "{text}");
    assert!(text.contains("local save failed"), "{text}");
    assert!(text.contains("other owner failed"), "{text}");
    assert!(text.contains("resolved"), "{text}");
}

#[test]
fn concurrent_save_owners_resolve_independently() {
    let mut editor = Editor::new(Buffer::from_text("one\n"));
    let first = failed_scratch_save(&mut editor);
    let second = editor
        .docs
        .try_insert(Document::scratch(Buffer::from_text("two\n")))
        .unwrap();
    editor.switch_to(second);
    failed_scratch_save(&mut editor);
    assert_eq!(editor.feedback.unresolved().count(), 2);

    editor.resolve_save_feedback(second, false);
    assert!(editor.feedback.unresolved().all(|record| {
        record.owner()
            == &FeedbackOwner::Save {
                document: first,
                remote: false,
            }
    }));
    assert_eq!(
        editor.feedback().len(),
        2,
        "resolved result remains recent history"
    );
}

#[test]
fn dismissal_is_presentation_only_and_completion_still_resolves() {
    let mut editor = Editor::new(Buffer::from_text("dirty\n"));
    let document = editor.current();
    let revision = editor.buf().revision();
    editor.note_unconfirmed_save(document, false, revision, "lost local receipt".into());
    let id = editor.feedback()[0].id();

    editor.invoke_feedback_action(id, 1);
    assert_eq!(
        editor.feedback()[0].lifecycle(),
        FeedbackLifecycle::Dismissed
    );
    assert!(editor.presented_feedback().is_none());
    assert!(editor.feedback()[0].unresolved());

    editor.resolve_save_feedback(document, false);
    assert_eq!(
        editor.feedback()[0].lifecycle(),
        FeedbackLifecycle::Resolved
    );
    assert!(editor.feedback()[0].actions.is_empty());
}

#[test]
fn context_actions_drop_retry_after_revision_changes() {
    let mut editor = Editor::new(Buffer::from_text("scratch\n"));
    failed_scratch_save(&mut editor);
    editor.context_actions();
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert!(picker
        .items
        .iter()
        .any(|item| item.text.contains("Retry the failed save")));
    assert_eq!(picker.kind, strop_picker::Kind::Actions);
    editor.close_picker();

    editor.feed_text("ichanged<esc>");
    editor.context_actions();
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert!(!picker
        .items
        .iter()
        .any(|item| item.text.contains("Retry the failed save")));
    assert!(picker
        .items
        .iter()
        .any(|item| item.text.contains("Dismiss notice")));
}

#[test]
fn stale_document_action_cannot_affect_replacement() {
    let mut editor = Editor::new(Buffer::from_text("old\n"));
    let old = failed_scratch_save(&mut editor);
    let id = editor.feedback()[0].id();
    let retry = editor.feedback()[0]
        .actions
        .iter()
        .position(|action| matches!(action, TypedAction::RetrySave(_)))
        .unwrap();
    editor.docs.remove(old);
    let replacement = editor
        .docs
        .try_insert(Document::scratch(Buffer::from_text("replacement\n")))
        .unwrap();
    assert_ne!(old, replacement);
    editor.switch_to(replacement);

    editor.invoke_feedback_action(id, retry);
    assert_eq!(editor.buf().text(), "replacement\n");
    assert!(editor.message.contains("stale"));
    assert!(editor.io.store_attempt(replacement).is_none());
}

#[test]
fn remote_incarnation_change_stales_captured_recovery() {
    let mut editor = Editor::new(Buffer::from_text("scratch\n"));
    let endpoint = RemoteEndpoint::parse("ssh://fixture.example").unwrap();
    editor
        .workspaces
        .select_root(strop_workspace::ResourceLocation::remote(
            endpoint.clone(),
            "/work".into(),
        ))
        .unwrap();
    let operation = crate::editor::remote::RemoteControl::Connect(endpoint.clone());
    editor.note_remote_failure(&operation, "connection refused".into());
    let id = editor.feedback()[0].id();
    editor
        .workspaces
        .note_disconnect(&strop_workspace::Filesystem::Remote(endpoint));

    editor.invoke_feedback_action(id, 0);
    assert!(editor.message.contains("stale"));
    assert_eq!(editor.feedback()[0].lifecycle(), FeedbackLifecycle::Active);

    editor.note_remote_recovered(&operation);
    assert_eq!(
        editor.feedback()[0].lifecycle(),
        FeedbackLifecycle::Resolved
    );
}

fn connection_event(endpoint: RemoteEndpoint) -> FeedbackEvent {
    FeedbackEvent {
        severity: FeedbackSeverity::Failure,
        owner: FeedbackOwner::Remote {
            endpoint,
            incarnation: 0,
        },
        kind: FeedbackKind::Connection,
        summary: "connection failed".into(),
        detail: "full connection detail".into(),
        reference: None,
        actions: vec![TypedAction::Dismiss],
    }
}

#[test]
fn repeats_coalesce_and_unresolved_history_is_never_evicted() {
    let mut store = FeedbackStore::default();
    let first = RemoteEndpoint::parse("ssh://host0.example").unwrap();
    store.record(connection_event(first.clone())).unwrap();
    let duplicate = store.record(connection_event(first.clone())).unwrap();
    assert_eq!(store.records().len(), 1);
    assert_eq!(store.records()[0].occurrences(), 2);

    let mut owners = vec![first];
    for index in 1..=FEEDBACK_HISTORY_BOUND + 4 {
        let endpoint = RemoteEndpoint::parse(&format!("ssh://host{index}.example")).unwrap();
        store.record(connection_event(endpoint.clone())).unwrap();
        owners.push(endpoint);
    }
    assert_eq!(store.records().len(), FEEDBACK_HISTORY_BOUND + 5);
    assert_eq!(store.unresolved().count(), FEEDBACK_HISTORY_BOUND + 5);
    assert_eq!(store.get(duplicate).unwrap().occurrences(), 2);
    assert!(store.saturated());

    for owner in owners {
        store.resolve_remote(&owner, FeedbackKind::Connection);
    }
    assert_eq!(store.records().len(), FEEDBACK_HISTORY_BOUND);
    assert!(store.unresolved().next().is_none());
    assert!(!store.saturated());
}

fn messages_text(editor: &mut Editor) -> String {
    editor.feed_text(":messages\r");
    assert_eq!(editor.buf().name.as_deref(), Some("messages"));
    assert!(editor.buf().readonly, "the history is a read-only surface");
    editor.buf().text().to_string()
}

/// B5: `:messages` keeps what the echo row said — refusals, ex errors and
/// feedback with its severity — in order, the same refusal once per key.
#[test]
fn messages_retains_refusals_errors_and_feedback() {
    let mut editor = Editor::new(Buffer::from_text("scratch\n"));
    editor.feed_text(":nonsense\r");
    let refusal = editor.message().to_owned();
    assert!(!refusal.is_empty());
    editor.feed_text("j");
    editor.feed_text(":nonsense\r");
    failed_scratch_save(&mut editor);
    let save = editor.feedback()[0].summary().to_owned();
    let text = messages_text(&mut editor);
    assert_eq!(
        text.matches(&format!("› {refusal}")).count(),
        2,
        "each refusal is logged when it is said: {text}"
    );
    assert!(text.contains(&format!("✗ {save}")), "{text}");
    assert!(
        text.find(&refusal).unwrap() < text.find(&save).unwrap(),
        "oldest first: {text}"
    );
    // q restores the buffer the user came from
    editor.feed_text("q");
    assert_eq!(editor.buf().text().to_string(), "scratch\n");
}

#[test]
fn messages_keeps_the_last_fifty() {
    let mut editor = Editor::new(Buffer::from_text("scratch\n"));
    for n in 0..60 {
        editor.feed_text("j");
        editor.message = format!("message number {n}");
        editor.log_echo_message();
    }
    let text = messages_text(&mut editor);
    assert!(!text.contains("message number 9\n"), "{text}");
    assert!(text.contains("message number 10\n"), "{text}");
    assert!(text.contains("message number 59\n"), "{text}");
    assert_eq!(text.matches("message number").count(), 50);
}
