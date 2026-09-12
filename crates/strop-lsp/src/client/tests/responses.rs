use super::*;

#[test]
fn hover_error_receives_a_terminal_note_with_the_original_stamp() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        initialize(&client);
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("x")
        ));
        let _open = wire.next().await;
        let wanted = ask(&client, document, 0);
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/hover");
        wire.reply_error(&request, -32603, "boom").await;
        let LspEvent::Note { context, text } = event(&rx).await else {
            panic!("note")
        };
        assert_eq!(context.stamp, wanted);
        assert_eq!(text, "hover failed: boom (jsonrpc error -32603)");
        wire.stop().await;
    });
}

#[test]
fn hover_null_and_empty_results_are_explicit_empties() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        initialize(&client);
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("x")
        ));
        let _open = wire.next().await;
        let null_stamp = ask(&client, document, 0);
        let request = wire.next().await;
        wire.reply(&request, Value::Null).await;
        let LspEvent::Note { context, text } = event(&rx).await else {
            panic!("note")
        };
        assert_eq!(context.stamp, null_stamp);
        assert_eq!(text, "no hover information at the cursor");
        // An empty hover payload is the same explicit empty.
        assert!(client.did_change(document, BufferRevision::new(1), path, Rope::from_str("y")));
        let _change = wire.next().await;
        let empty_stamp = client
            .request(RequestInput {
                document,
                revision: BufferRevision::new(1),
                path: path.to_owned(),
                line: LineIndex::new(0),
                byte_col: ByteColumn::new(0),
                line_text: "y".into(),
                kind: RequestKind::Hover,
                rename_to: None,
            })
            .unwrap();
        let request = wire.next().await;
        wire.reply(
            &request,
            json!({ "contents": { "kind": "plaintext", "value": "" } }),
        )
        .await;
        let LspEvent::Note { context, text } = event(&rx).await else {
            panic!("note")
        };
        assert_eq!(context.stamp, empty_stamp);
        assert_eq!(text, "no hover information at the cursor");
        wire.stop().await;
    });
}

#[test]
fn goto_null_is_a_note_and_switch_header_null_is_a_note() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        initialize(&client);
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("x")
        ));
        let _open = wire.next().await;
        let goto_stamp = client
            .request(RequestInput {
                document,
                revision: BufferRevision::new(0),
                path: path.to_owned(),
                line: LineIndex::new(0),
                byte_col: ByteColumn::new(0),
                line_text: "x".into(),
                kind: RequestKind::Goto,
                rename_to: None,
            })
            .unwrap();
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/definition");
        wire.reply(&request, Value::Null).await;
        let LspEvent::Note { context, text } = event(&rx).await else {
            panic!("note")
        };
        assert_eq!(context.stamp, goto_stamp);
        assert_eq!(text, "no definition found");
        let switch_stamp = client
            .request(RequestInput {
                document,
                revision: BufferRevision::new(0),
                path: path.to_owned(),
                line: LineIndex::new(0),
                byte_col: ByteColumn::new(0),
                line_text: "x".into(),
                kind: RequestKind::SwitchHeader,
                rename_to: None,
            })
            .unwrap();
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/switchSourceHeader");
        wire.reply(&request, Value::Null).await;
        let LspEvent::Note { context, text } = event(&rx).await else {
            panic!("note")
        };
        assert_eq!(context.stamp, switch_stamp);
        assert_eq!(text, "no header/source counterpart");
        wire.stop().await;
    });
}

#[test]
fn document_symbols_flatten_both_reply_shapes() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        client.caps.set(lt::ServerCapabilities {
            document_symbol_provider: Some(lt::OneOf::Left(true)),
            position_encoding: Some(lt::PositionEncodingKind::UTF8),
            ..Default::default()
        });
        client.finish_initialize().unwrap();
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("struct Foo; impl Foo { fn bar() {} }")
        ));
        let _open = wire.next().await;
        let ask = |kind| {
            client
                .request(RequestInput {
                    document,
                    revision: BufferRevision::new(0),
                    path: path.to_owned(),
                    line: LineIndex::new(0),
                    byte_col: ByteColumn::new(0),
                    line_text: "x".into(),
                    kind,
                    rename_to: None,
                })
                .unwrap()
        };
        // Hierarchical reply: children flatten with the ancestor path,
        // and the jump position is the selection range's start.
        let _nested_stamp = ask(RequestKind::DocumentSymbols);
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/documentSymbol");
        wire.reply(
            &request,
            serde_json::json!([{
                "name": "Foo", "kind": 23,
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 30}},
                "selectionRange": {"start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 10}},
                "children": [{
                    "name": "bar", "kind": 6,
                    "range": {"start": {"line": 0, "character": 12}, "end": {"line": 0, "character": 29}},
                    "selectionRange": {"start": {"line": 0, "character": 25}, "end": {"line": 0, "character": 28}}
                }]
            }]),
        )
        .await;
        let LspEvent::Symbols { symbols, .. } = event(&rx).await else {
            panic!("symbols event")
        };
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "Foo");
        assert_eq!(symbols[0].kind, "Struct");
        assert_eq!(symbols[0].container, "");
        assert_eq!(symbols[0].location.position.column.get(), 7);
        assert_eq!(symbols[1].name, "bar");
        assert_eq!(symbols[1].container, "Foo");
        assert_eq!(symbols[1].kind, "Method");
        // Legacy flat reply: containerName carries, location decodes
        // through the workspace like any other location.
        let _flat_stamp = ask(RequestKind::DocumentSymbols);
        let request = wire.next().await;
        wire.reply(
            &request,
            serde_json::json!([{
                "name": "main", "kind": 12, "containerName": "crate",
                "location": {
                    "uri": "file:///workspace/a.rs",
                    "range": {"start": {"line": 3, "character": 4}, "end": {"line": 3, "character": 8}}
                }
            }]),
        )
        .await;
        let LspEvent::Symbols { symbols, .. } = event(&rx).await else {
            panic!("symbols event")
        };
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "main");
        assert_eq!(symbols[0].kind, "Function");
        assert_eq!(symbols[0].container, "crate");
        assert_eq!(symbols[0].location.position.line.get(), 3);
        assert_eq!(
            symbols[0].location.doc.path,
            PathBuf::from("/workspace/a.rs")
        );
        wire.stop().await;
    });
}

#[test]
fn locations_null_is_an_empty_list_and_errors_are_notes() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        initialize(&client);
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("x")
        ));
        let _open = wire.next().await;
        let null_stamp = client
            .request(RequestInput {
                document,
                revision: BufferRevision::new(0),
                path: path.to_owned(),
                line: LineIndex::new(0),
                byte_col: ByteColumn::new(0),
                line_text: "x".into(),
                kind: RequestKind::Locations(LocKind::References),
                rename_to: None,
            })
            .unwrap();
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/references");
        wire.reply(&request, Value::Null).await;
        let LspEvent::Locations {
            context,
            kind,
            items,
        } = event(&rx).await
        else {
            panic!("locations")
        };
        assert_eq!(context.stamp, null_stamp);
        assert_eq!(kind, LocKind::References);
        assert!(items.is_empty(), "null result is an explicit empty list");
        assert!(client.did_change(document, BufferRevision::new(1), path, Rope::from_str("y")));
        let _change = wire.next().await;
        let error_stamp = client
            .request(RequestInput {
                document,
                revision: BufferRevision::new(1),
                path: path.to_owned(),
                line: LineIndex::new(0),
                byte_col: ByteColumn::new(0),
                line_text: "y".into(),
                kind: RequestKind::Locations(LocKind::References),
                rename_to: None,
            })
            .unwrap();
        let request = wire.next().await;
        wire.reply_error(&request, -32603, "index gone").await;
        let LspEvent::Note { context, text } = event(&rx).await else {
            panic!("note")
        };
        assert_eq!(context.stamp, error_stamp);
        assert_eq!(text, "references failed: index gone (jsonrpc error -32603)");
        wire.stop().await;
    });
}
