use super::*;
use crate::{FrozenLine, PositionEncoding, RequestInput, RequestKind, ServerColumn};
use serde_json::json;
use strop_core::id::{Arena, BufferRevision, ByteColumn, DocumentKind, LineIndex};

fn input(text: &str, caret: usize) -> RequestInput {
    let mut documents: Arena<DocumentKind, ()> = Arena::default();
    RequestInput {
        document: documents.try_insert(()).unwrap(),
        revision: BufferRevision::new(0),
        path: "/fixture/source.rs".into(),
        line: LineIndex::new(0),
        byte_col: ByteColumn::new(caret),
        line_text: FrozenLine::from(text),
        kind: RequestKind::Hover,
        rename_to: None,
        rename_files: None,
    }
}

#[test]
fn filter_text_and_server_sort_order_determine_the_retained_candidates() {
    let request = input("req", 3);
    let filter = CompletionFilter::new(
        &request,
        ByteColumn::new(0),
        ServerColumn::new(3),
        PositionEncoding::Utf8,
    )
    .unwrap();
    let result = decode_list(
        json!([
            {"label":"late-label", "filterText":"request_one", "sortText":"2"},
            {"label":"request_but_wrong_filter", "filterText":"unrelated", "sortText":"0"},
            {"label":"first-label", "filterText":"request_two", "sortText":"1"}
        ]),
        &filter,
        || false,
    )
    .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|entry| entry.item.label())
            .collect::<Vec<_>>(),
        ["first-label", "late-label"]
    );
    assert!(!result.server_incomplete);
    assert!(!result.client_truncated);
}

#[test]
fn explicit_edit_start_controls_filtering_instead_of_a_guessed_word_prefix() {
    let request = input("obj.re", 6);
    let filter = CompletionFilter::new(
        &request,
        ByteColumn::new(4),
        ServerColumn::new(6),
        PositionEncoding::Utf8,
    )
    .unwrap();
    let edit = json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":6}},"newText":"replacement"});
    let result = decode_list(
        json!([
            {"label":"replacement", "filterText":"obj.replace", "textEdit":edit},
            {"label":"replace", "filterText":"replace", "textEdit":edit}
        ]),
        &filter,
        || false,
    )
    .unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].item.label(), "replacement");
}

#[test]
fn utf16_filter_positions_reject_half_surrogates_instead_of_clamping() {
    let request = input("a😀re", 7);
    let filter = CompletionFilter::new(
        &request,
        ByteColumn::new(5),
        ServerColumn::new(5),
        PositionEncoding::Utf16,
    )
    .unwrap();
    let valid = json!([{ "label":"result", "textEdit":{
        "range":{"start":{"line":0,"character":3},"end":{"line":0,"character":5}},"newText":"result"
    }}]);
    assert_eq!(
        decode_list(valid, &filter, || false).unwrap().items[0]
            .item
            .label(),
        "result"
    );
    let invalid = json!([{ "label":"result", "textEdit":{
        "range":{"start":{"line":0,"character":2},"end":{"line":0,"character":5}},"newText":"result"
    }}]);
    assert_eq!(
        decode_list(invalid, &filter, || false),
        Err(CompletionDecodeError::FilterPosition)
    );
}

#[test]
fn filtering_precedes_the_window_limit_and_late_priority_is_respected() {
    let request = input("needle", 6);
    let filter = CompletionFilter::new(
        &request,
        ByteColumn::new(0),
        ServerColumn::new(6),
        PositionEncoding::Utf8,
    )
    .unwrap();
    let mut values: Vec<_> = (0..MAX_COMPLETION_ITEMS)
        .map(|n| json!({"label":format!("other_{n}")}))
        .collect();
    values.push(json!({"label":"needle_present"}));
    let result = decode_list(json!(values), &filter, || false).unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].ordinal, MAX_COMPLETION_ITEMS as u32);
    assert!(!result.client_truncated);

    let mut values: Vec<_> = (0..MAX_COMPLETION_ITEMS)
        .map(|n| {
            json!({
                "label":format!("needle_{n}"), "sortText":"9"
            })
        })
        .collect();
    values.push(json!({"label":"needle_preferred", "sortText":"0"}));
    let result = decode_list(
        json!({"isIncomplete":false,"items":values}),
        &filter,
        || false,
    )
    .unwrap();
    assert_eq!(result.items[0].item.label(), "needle_preferred");
    assert!(result.client_truncated);
    assert!(!result.server_incomplete);
}

#[test]
fn null_is_empty_and_server_incompleteness_is_not_client_truncation() {
    let request = input("re", 2);
    let filter = CompletionFilter::new(
        &request,
        ByteColumn::new(0),
        ServerColumn::new(2),
        PositionEncoding::Utf8,
    )
    .unwrap();
    let empty = decode_list(serde_json::Value::Null, &filter, || false).unwrap();
    assert_eq!(empty.items, vec![]);
    assert!(!empty.server_incomplete);
    let incomplete = decode_list(
        json!({"isIncomplete":true,"items":[{"label":"result"}]}),
        &filter,
        || false,
    )
    .unwrap();
    assert!(incomplete.server_incomplete);
    assert!(!incomplete.client_truncated);
    assert_eq!(
        decode_list(json!({"items":[]}), &filter, || false),
        Err(CompletionDecodeError::List)
    );
    assert_eq!(
        decode_list(
            json!({"isIncomplete":false,"itemDefaults":{"editRange":{}},"items":[]}),
            &filter,
            || false
        ),
        Err(CompletionDecodeError::ListDefaults)
    );
}

#[test]
fn identical_labels_with_different_operations_keep_distinct_identity() {
    let request = input("re", 2);
    let filter = CompletionFilter::new(
        &request,
        ByteColumn::new(0),
        ServerColumn::new(2),
        PositionEncoding::Utf8,
    )
    .unwrap();
    let result = decode_list(
        json!([
            {"label":"result", "insertText":"result_from_one"},
            {"label":"result", "insertText":"result_from_two"}
        ]),
        &filter,
        || false,
    )
    .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|entry| (entry.ordinal, entry.item.protocol.insert_text.as_deref()))
            .collect::<Vec<_>>(),
        [(0, Some("result_from_one")), (1, Some("result_from_two"))]
    );
}

#[test]
fn oversize_data_is_not_stripped_from_an_otherwise_accepted_item() {
    let request = input("re", 2);
    let filter = CompletionFilter::new(
        &request,
        ByteColumn::new(0),
        ServerColumn::new(2),
        PositionEncoding::Utf8,
    )
    .unwrap();
    let result = decode_list(
        json!([
            {"label":"result_needing_data", "data":"private".repeat(MAX_COMPLETION_ITEM_BYTES)},
            {"label":"result_usable"}
        ]),
        &filter,
        || false,
    )
    .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|entry| entry.item.label())
            .collect::<Vec<_>>(),
        ["result_usable"]
    );
    assert_eq!(result.omitted_oversized, 1);
    assert!(result.client_truncated);
}

#[test]
fn resolve_cannot_replace_the_chosen_operation_or_introduce_a_command() {
    let original: CompletionItem =
        serde_json::from_value(json!({"label":"result", "insertText":"result"})).unwrap();
    assert_eq!(
        decode_resolved(
            json!({"label":"result", "insertText":"different"}),
            &original
        ),
        Err(CompletionDecodeError::ResolveChangedOperation)
    );
    assert_eq!(
        decode_resolved(
            json!({"label":"result", "command":{"title":"run","command":"external"}}),
            &original
        ),
        Err(CompletionDecodeError::ResolveChangedOperation)
    );
    assert_eq!(
        decode_resolved(
            json!({"label":"result", "additionalTextEdits":"not an edit list"}),
            &original
        ),
        Err(CompletionDecodeError::Item)
    );
}

#[test]
fn direct_item_construction_cannot_bypass_resolve_payload_bounds() {
    for oversized in [
        json!({"label":"result", "data":"x".repeat(MAX_COMPLETION_ITEM_BYTES)}),
        json!({"label":"result", "data":vec![0; MAX_COMPLETION_ITEM_NODES]}),
        json!({"label":"result", "vendorExtension":{"opaque":"x".repeat(MAX_COMPLETION_ITEM_BYTES)}}),
    ] {
        assert!(serde_json::from_value::<CompletionItem>(oversized).is_err());
    }
}

#[test]
fn cancelled_decoding_stops_before_an_unvisited_malformed_tail() {
    let request = input("re", 2);
    let filter = CompletionFilter::new(
        &request,
        ByteColumn::new(0),
        ServerColumn::new(2),
        PositionEncoding::Utf8,
    )
    .unwrap();
    let mut observations = 0;
    let mut values: Vec<_> = (0..200)
        .map(|n| json!({"label":format!("result_{n}")}))
        .collect();
    values.push(json!(false)); // Must never reach this malformed tail after cancellation.
    assert_eq!(
        decode_list(json!(values), &filter, || {
            observations += 1;
            observations > 2
        }),
        Err(CompletionDecodeError::Cancelled)
    );
}
