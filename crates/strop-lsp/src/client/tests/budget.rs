//! The decode budget on a live connection (0101 §5.2): a reply over it fails
//! its own request and nothing else.
use super::*;

/// A `documentSymbol` reply body of `count` symbols in rust-analyzer's
/// shape: 43 structure tokens each.
fn symbols(id: &Value, count: usize) -> Vec<u8> {
    let mut body = format!(r#"{{"jsonrpc":"2.0","id":{id},"result":["#);
    for i in 0..count {
        if i > 0 {
            body.push(',');
        }
        let line = i * 3;
        body.push_str(&format!(
            r#"{{"name":"function_number_{i}","detail":"fn(&self, value: usize) -> usize","kind":12,"tags":[],"deprecated":false,"range":{{"start":{{"line":{line},"character":0}},"end":{{"line":{},"character":1}}}},"selectionRange":{{"start":{{"line":{line},"character":7}},"end":{{"line":{line},"character":24}}}},"children":[]}}"#,
            line + 2
        ));
    }
    body.push_str("]}");
    body.into_bytes()
}

async fn send(wire: &mut Wire, body: &[u8]) {
    wire.writer
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await
        .unwrap();
    wire.writer.write_all(body).await.unwrap();
    wire.writer.flush().await.unwrap();
}

/// A ready client with one open document; returns a way to ask.
async fn opened(client: &Client, wire: &mut Wire) -> strop_core::id::DocumentId {
    let mut docs = Documents::default();
    let document = docs.try_insert(()).unwrap();
    client.caps.set(lt::ServerCapabilities {
        document_symbol_provider: Some(lt::OneOf::Left(true)),
        hover_provider: Some(lt::HoverProviderCapability::Simple(true)),
        position_encoding: Some(lt::PositionEncodingKind::UTF8),
        ..Default::default()
    });
    client.finish_initialize().unwrap();
    assert!(client.did_open(
        document,
        BufferRevision::new(0),
        Path::new("/workspace/a.rs"),
        "rust",
        Rope::from_str("fn a() {}")
    ));
    let _open = wire.next().await;
    document
}

fn request(client: &Client, document: strop_core::id::DocumentId, kind: RequestKind) {
    client
        .request(RequestInput {
            document,
            revision: BufferRevision::new(0),
            path: PathBuf::from("/workspace/a.rs"),
            line: LineIndex::new(0),
            byte_col: ByteColumn::new(0),
            line_text: "fn a() {}".into(),
            kind,
            rename_to: None,
            rename_files: None,
        })
        .unwrap();
}

/// The next request is answered on the same connection, and nothing
/// failed or restarted along the way.
async fn still_serves(
    client: &Client,
    rx: &Receiver<LspEvent>,
    wire: &mut Wire,
    document: strop_core::id::DocumentId,
) {
    request(client, document, RequestKind::Hover);
    let hover = wire.next().await;
    assert_eq!(hover["method"], "textDocument/hover");
    wire.answer(&hover, "still here").await;
    let LspEvent::HoverText { text, .. } = event(rx).await else {
        panic!("the next request is answered")
    };
    assert_eq!(text, "still here");
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event, LspEvent::Failed { .. }),
            "the connection did not fail: {event:?}"
        );
    }
}

#[test]
fn the_largest_kept_symbol_list_is_decoded_and_delivered() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let document = opened(&client, &mut wire).await;
        request(&client, document, RequestKind::DocumentSymbols);
        let asked = wire.next().await;
        // the engine keeps at most 20,000 symbols per list (0099 §4.1)
        send(&mut wire, &symbols(&asked["id"], 20_000)).await;
        let LspEvent::Symbols { symbols, .. } = event(&rx).await else {
            panic!("symbols event")
        };
        assert_eq!(symbols.len(), 20_000);
        still_serves(&client, &rx, &mut wire, document).await;
        wire.stop().await;
    });
}

#[test]
fn a_reply_over_the_budget_fails_only_its_request() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let document = opened(&client, &mut wire).await;
        request(&client, document, RequestKind::DocumentSymbols);
        let asked = wire.next().await;
        let body = symbols(&asked["id"], 40_000);
        assert!(structure_tokens(&body) > MAX_DECODE_TOKENS);
        send(&mut wire, &body).await;
        let LspEvent::Note { context, text } = event(&rx).await else {
            panic!("the request ends in a note")
        };
        assert_eq!(context.kind, RequestKind::DocumentSymbols);
        assert!(
            text.contains("reply too large to decode") && text.contains("limit 1315860 (512 MiB)"),
            "{text}"
        );
        still_serves(&client, &rx, &mut wire, document).await;
        wire.stop().await;
    });
}

#[test]
fn a_notification_over_the_budget_is_dropped_and_said_once() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let document = opened(&client, &mut wire).await;
        let mut flood = br#"{"jsonrpc":"2.0","method":"$/progress","params":["#.to_vec();
        flood.extend(b"0,".repeat(MAX_DECODE_TOKENS));
        flood.extend_from_slice(b"0]}");
        send(&mut wire, &flood).await;
        send(&mut wire, &flood).await;
        let LspEvent::NotificationDropped { tokens, .. } = event(&rx).await else {
            panic!("the editor is told")
        };
        assert_eq!(tokens, structure_tokens(&flood));
        still_serves(&client, &rx, &mut wire, document).await;
        wire.stop().await;
    });
}

#[test]
fn a_server_request_over_the_budget_ends_the_connection() {
    run(async {
        let (client, _rx, mut wire) = Wire::new();
        let _document = opened(&client, &mut wire).await;
        let mut flood =
            br#"{"jsonrpc":"2.0","id":1,"method":"workspace/configuration","params":["#.to_vec();
        flood.extend(b"0,".repeat(MAX_DECODE_TOKENS));
        flood.extend_from_slice(b"0]}");
        send(&mut wire, &flood).await;
        let ended = (&mut wire.task).await;
        assert!(ended.is_ok(), "the mainloop returned on its own");
    });
}
