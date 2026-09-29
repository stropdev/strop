use super::*;
use crate::editor::completion::context::{capture, Invocation};
use crate::editor::completion::model::{CompletionPayload, CompletionQuery, LanguageSuggestions};
use crate::editor::Editor;
use strop_core::frontend_input::Key;
use strop_core::Buffer;
use strop_lsp::completion::{CompletionEntry, CompletionItem, CompletionList};

fn query(request: u64) -> Arc<CompletionQuery> {
    let mut editor = Editor::new_in(Buffer::from_text("alpha al"), "/mailbox-fixture".into());
    editor.feed(Key::Char('i'));
    editor.set_head(8);
    Arc::new(CompletionQuery {
        request: WorkerId::new(request),
        context: Arc::new(capture(&editor, Invocation::ManualCombined, 0, None).unwrap()),
    })
}
fn payload(query: Arc<CompletionQuery>, label: &str) -> CompletionDelivery {
    let item: CompletionItem = serde_json::from_value(serde_json::json!({
        "label":label, "data":{"opaque":"x".repeat(32 * 1024)}
    }))
    .unwrap();
    let source = query.context.primary().unwrap();
    let origin = strop_lsp::ReplyContext {
        stamp: strop_lsp::RequestStamp {
            request: strop_lsp::RequestId::new(query.request.get()),
            server: strop_lsp::ServerId::new(1),
            document: source.source,
            revision: source.revision,
        },
        kind: strop_lsp::RequestKind::Completion,
        encoding: strop_lsp::PositionEncoding::Utf8,
    };
    let items = (0..8)
        .map(|ordinal| CompletionEntry {
            ordinal,
            item: Arc::new(item.clone()),
        })
        .collect();
    let owner = crate::editor::completion::context::LanguageContext::capture(
        &crate::editor::lsp::state::Binding {
            server: origin.stamp.server,
            path: "/mailbox-fixture/source".into(),
            root: "/mailbox-fixture".into(),
            language: "c".into(),
            target: strop_workspace::Filesystem::Local,
            revision: source.revision,
        },
        None,
    )
    .unwrap();
    CompletionDelivery {
        query,
        scope: DeliveryScope::Query,
        payload: CompletionPayload::Language(Outcome::Success(LanguageSuggestions {
            origin,
            owner,
            items: CompletionList {
                items,
                server_incomplete: false,
                client_truncated: false,
                omitted_oversized: 0,
            },
            resolve_required: false,
            prefix: Arc::from("al"),
            suffix: Arc::from(""),
        })),
    }
}
fn publish(mailbox: &Arc<CompletionMailbox>, delivery: CompletionDelivery) -> bool {
    let mailbox = mailbox.clone();
    std::thread::spawn(move || mailbox.publish(delivery))
        .join()
        .unwrap()
}
fn retire(mailbox: &Arc<CompletionMailbox>) {
    let mailbox = mailbox.clone();
    std::thread::spawn(move || mailbox.retire_native())
        .join()
        .unwrap();
}

#[test]
fn ui_releasing_a_large_candidate_snapshot_is_not_its_final_owner() {
    let mailbox = Arc::new(CompletionMailbox::default());
    let query = query(1);
    mailbox.query(Some(query.request));
    assert!(publish(&mailbox, payload(query, "alpha")));
    let drain = mailbox.take();
    let weak = Arc::downgrade(&drain.deliveries[0]);
    mailbox.query(None);
    drop(drain);
    assert_eq!(
        weak.strong_count(),
        1,
        "native ownership survives the last UI lease"
    );
    retire(&mailbox);
    assert!(
        weak.upgrade().is_none(),
        "the native retirement step releases the payload"
    );
    assert_eq!(mailbox.take().usage.charged_bytes, 0);
}

#[test]
fn an_obsolete_producer_cannot_remove_an_unobserved_newest_snapshot() {
    let mailbox = Arc::new(CompletionMailbox::default());
    let old = query(1);
    let newest = query(2);
    mailbox.query(Some(newest.request));
    assert!(publish(&mailbox, payload(newest.clone(), "alpha_newest")));
    assert!(!publish(&mailbox, payload(old, "alpha_obsolete")));
    let drain = mailbox.take();
    assert_eq!(drain.deliveries[0].query.request, newest.request);
    let CompletionPayload::Language(Outcome::Success(language)) = &drain.deliveries[0].payload
    else {
        panic!("language snapshot")
    };
    assert_eq!(language.items.items[0].item.label(), "alpha_newest");
    mailbox.query(None);
    drop(drain);
    retire(&mailbox);
}

#[test]
fn cancellation_crosses_a_full_retirement_budget_without_waiting_for_ui_leases() {
    let mailbox = Arc::new(CompletionMailbox::default());
    let mut held = Vec::new();
    for request in 1..=4 {
        let query = query(request);
        mailbox.query(Some(query.request));
        assert!(publish(&mailbox, payload(query, "alpha")));
        held.extend(mailbox.take().deliveries);
    }
    assert_eq!(mailbox.take().usage.charged_bytes, MAX_RETAINED_BYTES);
    let query = query(5);
    mailbox.query(Some(query.request));
    let (started, ready) = std::sync::mpsc::channel();
    let (done, result) = std::sync::mpsc::channel();
    let native = mailbox.clone();
    let producer = std::thread::spawn(move || {
        started.send(()).unwrap();
        done.send(native.publish(payload(query, "alpha_waiting")))
            .unwrap();
    });
    ready.recv().unwrap();
    mailbox.query(None);
    assert!(!result
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap());
    producer.join().unwrap();
    assert_eq!(held.len(), 4);
    assert!(mailbox.take().usage.charged_bytes <= MAX_RETAINED_BYTES);
    drop(held);
    mailbox.released();
    retire(&mailbox);
    assert_eq!(mailbox.take().usage.charged_bytes, 0);
}
