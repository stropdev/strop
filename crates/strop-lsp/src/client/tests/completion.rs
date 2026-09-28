use super::*;
use crate::completion::{
    CompletionCall, CompletionReply, CompletionResult, CompletionSink, CompletionTransportEvent,
    CompletionTrigger, PendingCompletion,
};
use strop_core::id::DocumentId;
use strop_core::worker::{CancelReason, FailureKind, Outcome};

fn ready(client: &Client, document: DocumentId, text: &str) {
    client.caps.set(lt::ServerCapabilities {
        completion_provider: Some(lt::CompletionOptions {
            resolve_provider: Some(true),
            trigger_characters: Some(vec![".".into()]),
            ..Default::default()
        }),
        position_encoding: Some(lt::PositionEncodingKind::UTF16),
        ..Default::default()
    });
    client.finish_initialize().unwrap();
    assert!(client.did_open(
        document,
        BufferRevision::new(0),
        Path::new("/workspace/a.rs"),
        "rust",
        Rope::from_str(text)
    ));
}

fn query(
    client: &Client,
    document: DocumentId,
    revision: u64,
    text: &str,
    start: usize,
) -> PendingCompletion {
    client
        .prepare_completion(
            RequestInput {
                document,
                revision: BufferRevision::new(revision),
                path: "/workspace/a.rs".into(),
                line: LineIndex::new(0),
                byte_col: ByteColumn::new(text.len()),
                line_text: text.into(),
                kind: RequestKind::Completion,
                rename_to: None,
            },
            CompletionCall::Query {
                trigger: CompletionTrigger::Invoked,
                prefix_start: ByteColumn::new(start),
            },
        )
        .unwrap()
}

fn observer() -> (CompletionSink, Receiver<CompletionTransportEvent>) {
    let (tx, rx) = channel();
    (
        Arc::new(move |event| {
            let _ = tx.send(event);
        }),
        rx,
    )
}

async fn next(rx: &Receiver<CompletionTransportEvent>) -> CompletionTransportEvent {
    loop {
        match rx.try_recv() {
            Ok(event) => return event,
            Err(TryRecvError::Disconnected) => panic!("completion observer disconnected"),
            Err(TryRecvError::Empty) => tokio::task::yield_now().await,
        }
    }
}
async fn reply(rx: &Receiver<CompletionTransportEvent>) -> CompletionReply {
    let CompletionTransportEvent::Reply(reply) = next(rx).await else {
        panic!("terminal completion reply expected")
    };
    reply
}
async fn capacity(rx: &Receiver<CompletionTransportEvent>, client: &Client) {
    let CompletionTransportEvent::Capacity { server } = next(rx).await else {
        panic!("physical request retirement expected")
    };
    assert_eq!(server, client.id());
}

#[test]
fn query_and_resolve_preserve_negotiated_positions_and_opaque_item_identity() {
    run(async {
        let (client, _, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.try_insert(()).unwrap();
        ready(&client, document, "a😀re");
        let (sink, rx) = observer();
        let pending = query(&client, document, 0, "a😀re", 5);
        let stamp = pending.stamp;
        client.launch_completion(pending, &sink).unwrap();
        let open = wire.next().await;
        assert_eq!(open["method"], "textDocument/didOpen");
        assert_eq!(open["params"]["textDocument"]["text"], "a😀re");
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/completion");
        assert_eq!(
            request["params"]["position"],
            json!({"line":0,"character":5})
        );
        let original = json!({"label":"result", "data":{"private":"opaque", "identity":[17,42]},
            "vendorField":{"meaning":"kept"}, "tags":[1],
            "textEdit":{"range":{"start":{"line":0,"character":3},"end":{"line":0,"character":5}}, "newText":"result"}});
        wire.reply(
            &request,
            json!({"isIncomplete":true,"items":[original.clone()]}),
        )
        .await;
        let answer = reply(&rx).await;
        assert_eq!(answer.context.stamp, stamp);
        assert_eq!(answer.context.encoding, PositionEncoding::Utf16);
        let Outcome::Success(CompletionResult::List(list)) = answer.outcome else {
            panic!("completion list")
        };
        assert!(list.server_incomplete);
        assert!(!list.client_truncated);
        assert_eq!(list.items[0].item.label(), "result");
        capacity(&rx, &client).await;
        let item = list.items[0].item.clone();
        let pending = client
            .prepare_completion(
                query(&client, document, 0, "a😀re", 5).input,
                CompletionCall::Resolve { item },
            )
            .unwrap();
        let stamp = pending.stamp;
        client.launch_completion(pending, &sink).unwrap();
        let resolving = wire.next().await;
        assert_eq!(resolving["method"], "completionItem/resolve");
        assert_eq!(resolving["params"], original);
        wire.reply(&resolving, json!({"label":"result", "documentation":{"kind":"markdown","value":"**result** docs"},
            "additionalTextEdits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},
                "newText":"use result;\n"}]})).await;
        let resolved = reply(&rx).await;
        assert_eq!(resolved.context.stamp, stamp);
        let Outcome::Success(CompletionResult::Resolved(item)) = resolved.outcome else {
            panic!("resolved item")
        };
        assert_eq!(
            item.protocol().additional_text_edits.as_ref().unwrap()[0].new_text,
            "use result;\n"
        );
        capacity(&rx, &client).await;
        assert_eq!(client.completion_usage().physical_requests, 0);
        wire.stop().await;
    });
}

#[test]
fn ignored_cancellation_retains_physical_slots_until_late_responses() {
    run(async {
        let (client, _, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.try_insert(()).unwrap();
        ready(&client, document, "re");
        wire.next().await;
        let (sink, rx) = observer();
        let mut sent = Vec::new();
        // Preparing without launching consumes a logical identity, not a wire
        // identity. Cancellation must use the latter after they diverge.
        drop(query(&client, document, 0, "re", 0));
        for _ in 0..2 {
            let pending = query(&client, document, 0, "re", 0);
            let stamp = pending.stamp;
            client.launch_completion(pending, &sink).unwrap();
            let request = wire.next().await;
            assert_ne!(request["id"].as_u64(), Some(stamp.request.get()));
            client.cancel_completion(stamp, CancelReason::Superseded);
            let cancelled = wire.next().await;
            assert_eq!(cancelled["method"], "$/cancelRequest");
            assert_eq!(cancelled["params"]["id"], request["id"]);
            let terminal = reply(&rx).await;
            assert_eq!(terminal.context.stamp, stamp);
            assert_eq!(
                terminal.outcome,
                Outcome::Cancelled(CancelReason::Superseded)
            );
            sent.push(request);
        }
        for _ in 0..100 {
            assert_eq!(
                client.launch_completion(query(&client, document, 0, "re", 0), &sink),
                Err(RequestRefusal::Overloaded)
            );
        }
        assert_eq!(client.completion_usage().physical_requests, 2);
        wire.reply(&sent[0], json!([{"label":"result_stale"}]))
            .await;
        capacity(&rx, &client).await; // No second terminal reply for the cancelled request.
        assert_eq!(client.completion_usage().physical_requests, 1);
        let newest = query(&client, document, 0, "re", 0);
        let stamp = newest.stamp;
        client.launch_completion(newest, &sink).unwrap();
        let current = wire.next().await;
        assert_eq!(current["method"], "textDocument/completion");
        wire.reply(&current, json!([{"label":"result_current"}]))
            .await;
        let terminal = reply(&rx).await;
        assert_eq!(terminal.context.stamp, stamp);
        let Outcome::Success(CompletionResult::List(items)) = terminal.outcome else {
            panic!("newest query")
        };
        assert_eq!(items.items[0].item.label(), "result_current");
        capacity(&rx, &client).await;
        wire.reply(&sent[1], Value::Null).await;
        capacity(&rx, &client).await;
        assert_eq!(client.completion_usage(), Default::default());
        wire.stop().await;
    });
}

#[test]
fn cancellation_before_task_poll_releases_without_sending_an_obsolete_request() {
    run(async {
        let (client, _, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.try_insert(()).unwrap();
        ready(&client, document, "re");
        let (sink, rx) = observer();
        let old = query(&client, document, 0, "re", 0);
        let stamp = old.stamp;
        client.launch_completion(old, &sink).unwrap();
        client.cancel_completion(stamp, CancelReason::Dismissed);
        assert_eq!(
            reply(&rx).await.outcome,
            Outcome::Cancelled(CancelReason::Dismissed)
        );
        capacity(&rx, &client).await;
        assert_eq!(client.completion_usage(), Default::default());
        assert_eq!(wire.next().await["method"], "textDocument/didOpen");
        let current = query(&client, document, 0, "re", 0);
        let current_stamp = current.stamp;
        client.launch_completion(current, &sink).unwrap();
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/completion");
        wire.reply(&request, Value::Null).await;
        assert_eq!(reply(&rx).await.context.stamp, current_stamp);
        capacity(&rx, &client).await;
        wire.stop().await;
    });
}

#[test]
fn deadline_settles_logical_ownership_without_freeing_an_unanswered_request() {
    run(async {
        let (client, _, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.try_insert(()).unwrap();
        ready(&client, document, "re");
        wire.next().await;
        let (sink, rx) = observer();
        client
            .launch_completion(query(&client, document, 0, "re", 0), &sink)
            .unwrap();
        let request = wire.next().await;
        tokio::time::pause();
        // Cross the deadline and the timer wheel's millisecond rounding;
        // virtual time must not depend on the real sub-millisecond start tick.
        tokio::time::advance(std::time::Duration::from_secs(3)).await;
        let terminal = reply(&rx).await;
        assert!(
            matches!(terminal.outcome, Outcome::Failed { failure, .. } if failure.kind == FailureKind::Unavailable)
        );
        let cancel = wire.next().await;
        assert_eq!(cancel["params"]["id"], request["id"]);
        assert_eq!(client.completion_usage().physical_requests, 1);
        wire.reply(&request, Value::Null).await;
        capacity(&rx, &client).await;
        assert_eq!(client.completion_usage(), Default::default());
        wire.stop().await;
    });
}

#[test]
fn newer_document_revision_revokes_queued_request_and_orders_its_successor() {
    run(async {
        let (client, _, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.try_insert(()).unwrap();
        ready(&client, document, "re");
        let (sink, rx) = observer();
        let old = query(&client, document, 0, "re", 0);
        client.launch_completion(old, &sink).unwrap();
        assert!(client.did_change(
            document,
            BufferRevision::new(1),
            Path::new("/workspace/a.rs"),
            Rope::from_str("res")
        ));
        let terminal = reply(&rx).await;
        assert_eq!(
            terminal.outcome,
            Outcome::Cancelled(CancelReason::Superseded)
        );
        capacity(&rx, &client).await;
        assert_eq!(wire.next().await["method"], "textDocument/didOpen");
        let changed = wire.next().await;
        assert_eq!(changed["method"], "textDocument/didChange");
        assert_eq!(changed["params"]["contentChanges"][0]["text"], "res");
        client
            .launch_completion(query(&client, document, 1, "res", 0), &sink)
            .unwrap();
        let request = wire.next().await;
        assert_eq!(request["params"]["position"]["character"], 3);
        wire.reply(&request, Value::Null).await;
        assert_eq!(
            reply(&rx).await.context.stamp.revision,
            BufferRevision::new(1)
        );
        capacity(&rx, &client).await;
        wire.stop().await;
    });
}

#[test]
fn connection_close_settles_sent_owners_and_refuses_resurrection() {
    run(async {
        let (client, _, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.try_insert(()).unwrap();
        ready(&client, document, "re");
        wire.next().await;
        let (sink, rx) = observer();
        client
            .launch_completion(query(&client, document, 0, "re", 0), &sink)
            .unwrap();
        wire.next().await;
        wire.stop().await;
        assert!(
            matches!(reply(&rx).await.outcome, Outcome::Failed { failure, .. }
            if failure.kind == FailureKind::Disconnected)
        );
        assert!(
            matches!(next(&rx).await, CompletionTransportEvent::Closed { server } if server == client.id())
        );
        assert_eq!(client.completion_usage(), Default::default());
        assert_eq!(
            client.launch_completion(query(&client, document, 0, "re", 0), &sink),
            Err(RequestRefusal::NotReady)
        );
    });
}

#[test]
fn captured_line_bytes_backpressure_before_the_physical_count_limit() {
    run(async {
        let (client, _, wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.try_insert(()).unwrap();
        let text = "a".repeat(5 * 1024 * 1024);
        ready(&client, document, &text);
        let (sink, rx) = observer();
        let first = query(&client, document, 0, &text, text.len());
        let stamp = first.stamp;
        client.launch_completion(first, &sink).unwrap();
        assert_eq!(client.completion_usage().physical_requests, 1);
        assert!(client.completion_usage().captured_bytes >= text.len());
        assert_eq!(
            client.launch_completion(query(&client, document, 0, &text, text.len()), &sink),
            Err(RequestRefusal::Overloaded)
        );
        client.cancel_completion(stamp, CancelReason::Dismissed);
        assert_eq!(
            reply(&rx).await.outcome,
            Outcome::Cancelled(CancelReason::Dismissed)
        );
        capacity(&rx, &client).await;
        assert_eq!(client.completion_usage(), Default::default());
        wire.stop().await;
    });
}

#[test]
fn a_new_owner_waiting_on_old_requests_gets_capacity_without_reviving_a_dropped_owner() {
    run(async {
        let (client, _, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.try_insert(()).unwrap();
        ready(&client, document, "re");
        wire.next().await;
        let (old_sink, old_rx) = observer();
        let mut sent = Vec::new();
        for _ in 0..2 {
            let pending = query(&client, document, 0, "re", 0);
            let stamp = pending.stamp;
            client.launch_completion(pending, &old_sink).unwrap();
            sent.push(wire.next().await);
            client.cancel_completion(stamp, CancelReason::Dismissed);
            wire.next().await;
            assert_eq!(
                reply(&old_rx).await.outcome,
                Outcome::Cancelled(CancelReason::Dismissed)
            );
        }
        drop(old_sink);
        drop(old_rx);
        let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = wakes.clone();
        let wake: crate::completion::CompletionWake = Arc::new(move || {
            observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });
        let (new_sink, new_rx) = observer();
        client.wake_completion_when_available(&wake);
        assert_eq!(
            client.launch_completion(query(&client, document, 0, "re", 0), &new_sink),
            Err(RequestRefusal::Overloaded)
        );
        wire.reply(&sent[0], Value::Null).await;
        while wakes.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            tokio::task::yield_now().await;
        }
        client
            .launch_completion(query(&client, document, 0, "re", 0), &new_sink)
            .unwrap();
        let current = wire.next().await;
        wire.reply(&current, Value::Null).await;
        assert!(matches!(
            reply(&new_rx).await.outcome,
            Outcome::Success(CompletionResult::List(_))
        ));
        capacity(&new_rx, &client).await;
        client.wake_completion_when_available(&wake);
        drop(wake);
        wire.reply(&sent[1], Value::Null).await;
        while client.completion_usage().physical_requests != 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 1);
        wire.stop().await;
    });
}
