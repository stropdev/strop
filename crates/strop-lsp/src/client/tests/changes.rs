use super::*;

#[test]
fn format_returns_edits_and_null_is_an_explicit_empty() {
    run(async {
        let (client, rx, mut wire) = Wire::production();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        client.caps.set(lt::ServerCapabilities {
            document_formatting_provider: Some(lt::OneOf::Left(true)),
            position_encoding: Some(lt::PositionEncodingKind::UTF8),
            ..Default::default()
        });
        client.finish_initialize().unwrap();
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("fn main() {}\n")
        ));
        let open = wire.next().await;
        assert_eq!(open["method"], "textDocument/didOpen");
        let wanted = client
            .format(document, BufferRevision::new(0), path.to_owned(), 4)
            .unwrap();
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/formatting");
        assert_eq!(request["params"]["options"]["tabSize"], 4);
        assert_eq!(request["params"]["options"]["insertSpaces"], true);
        wire.reply(
            &request,
            json!([
                { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 2 } }, "newText": "fn  " },
                { "range": { "start": { "line": 0, "character": 13 }, "end": { "line": 0, "character": 13 } }, "newText": "\n" }
            ]),
        )
        .await;
        let LspEvent::Edits { context, edits } = event(&rx).await else {
            panic!("edits")
        };
        assert_eq!(context.stamp, wanted);
        assert_eq!(context.kind, RequestKind::Format);
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].start.line, LineIndex::new(0));
        assert_eq!(edits[0].end.column, ServerColumn::new(2));
        assert_eq!(edits[0].new_text, "fn  ");
        assert_eq!(edits[1].new_text, "\n");
        // A null reply is an explicit empty edit set.
        assert!(client.did_change(
            document,
            BufferRevision::new(1),
            path,
            Rope::from_str("fn  main() {}\n")
        ));
        let _change = wire.next().await;
        let wanted = client
            .format(document, BufferRevision::new(1), path.to_owned(), 4)
            .unwrap();
        let request = wire.next().await;
        wire.reply(&request, Value::Null).await;
        let LspEvent::Edits { context, edits } = event(&rx).await else {
            panic!("edits")
        };
        assert_eq!(context.stamp, wanted);
        assert!(
            edits.is_empty(),
            "null formatting result is an explicit empty"
        );
        wire.stop().await;
    });
}

#[test]
fn rename_over_changes_map_and_document_changes_produce_workspace_edits() {
    run(async {
        let (client, rx, mut wire) = Wire::production();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        // No positionEncoding: the spec-default UTF-16 is exercised at
        // a non-ASCII boundary.
        client.caps.set(lt::ServerCapabilities {
            rename_provider: Some(lt::OneOf::Left(true)),
            ..Default::default()
        });
        client.finish_initialize().unwrap();
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("a😀z\n")
        ));
        let _open = wire.next().await;
        let input = |revision| RequestInput {
            document,
            revision: BufferRevision::new(revision),
            path: path.to_owned(),
            line: LineIndex::new(0),
            byte_col: ByteColumn::new(5),
            line_text: "a😀z".into(),
            kind: RequestKind::Rename,
            rename_to: None,
        };
        let changes_stamp = client.rename(input(0), "renamed").unwrap();
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/rename");
        // byte col 5 ('z' after a + emoji) is UTF-16 unit 3.
        assert_eq!(request["params"]["position"]["character"], 3);
        assert_eq!(request["params"]["newName"], "renamed");
        wire.reply(
            &request,
            json!({
                "changes": {
                    "file:///workspace/b.rs": [
                        { "range": { "start": { "line": 1, "character": 2 }, "end": { "line": 1, "character": 5 } }, "newText": "renamed" }
                    ],
                    "file:///workspace/a.rs": [
                        { "range": { "start": { "line": 0, "character": 1 }, "end": { "line": 0, "character": 3 } }, "newText": "renamed" }
                    ]
                }
            }),
        )
        .await;
        let LspEvent::WorkspaceEdits { context, edits } = event(&rx).await else {
            panic!("workspace edits")
        };
        assert_eq!(context.stamp, changes_stamp);
        assert_eq!(context.encoding, PositionEncoding::Utf16);
        // The changes map is delivered sorted by path, never hash order.
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].0.path, PathBuf::from("/workspace/a.rs"));
        assert_eq!(edits[0].0.filesystem, strop_workspace::Filesystem::Local);
        assert_eq!(edits[0].1[0].start.column, ServerColumn::new(1));
        assert_eq!(edits[1].0.path, PathBuf::from("/workspace/b.rs"));
        assert_eq!(edits[1].1[0].start.line, LineIndex::new(1));
        // documentChanges, versioned with the version this connection sent.
        assert!(client.did_change(
            document,
            BufferRevision::new(1),
            path,
            Rope::from_str("a😀z!\n")
        ));
        let change = wire.next().await;
        let version = change["params"]["textDocument"]["version"]
            .as_i64()
            .unwrap();
        let docs_stamp = client.rename(input(1), "renamed").unwrap();
        let request = wire.next().await;
        wire.reply(
            &request,
            json!({
                "documentChanges": [
                    {
                        "textDocument": { "uri": "file:///workspace/a.rs", "version": version },
                        "edits": [
                            { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }, "newText": "renamed" }
                        ]
                    }
                ]
            }),
        )
        .await;
        let LspEvent::WorkspaceEdits { context, edits } = event(&rx).await else {
            panic!("workspace edits")
        };
        assert_eq!(context.stamp, docs_stamp);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].0.path, PathBuf::from("/workspace/a.rs"));
        assert_eq!(edits[0].1[0].new_text, "renamed");
        wire.stop().await;
    });
}

#[test]
fn rename_with_resource_operations_or_a_foreign_version_is_a_refusal_note() {
    run(async {
        let (client, rx, mut wire) = Wire::production();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        client.caps.set(lt::ServerCapabilities {
            rename_provider: Some(lt::OneOf::Left(true)),
            position_encoding: Some(lt::PositionEncodingKind::UTF8),
            ..Default::default()
        });
        client.finish_initialize().unwrap();
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("x")
        ));
        let _open = wire.next().await;
        let input = || RequestInput {
            document,
            revision: BufferRevision::new(0),
            path: path.to_owned(),
            line: LineIndex::new(0),
            byte_col: ByteColumn::new(0),
            line_text: "x".into(),
            kind: RequestKind::Rename,
            rename_to: None,
        };
        // Resource operations: never a partial edit set.
        let ops_stamp = client.rename(input(), "new").unwrap();
        let request = wire.next().await;
        wire.reply(
            &request,
            json!({
                "documentChanges": [
                    { "kind": "create", "uri": "file:///workspace/new.rs" }
                ]
            }),
        )
        .await;
        let LspEvent::Note { context, text } = event(&rx).await else {
            panic!("note")
        };
        assert_eq!(context.stamp, ops_stamp);
        assert_eq!(
            text,
            "rename edit carries file operations (create/rename/delete) — refusing the whole edit"
        );
        // A version this connection never sent: stale or foreign.
        let foreign_stamp = client.rename(input(), "new").unwrap();
        let request = wire.next().await;
        wire.reply(
            &request,
            json!({
                "documentChanges": [
                    {
                        "textDocument": { "uri": "file:///workspace/a.rs", "version": 999 },
                        "edits": []
                    }
                ]
            }),
        )
        .await;
        let LspEvent::Note { context, text } = event(&rx).await else {
            panic!("note")
        };
        assert_eq!(context.stamp, foreign_stamp);
        assert_eq!(
            text,
            "rename targets a version of file:///workspace/a.rs this connection never sent — refusing the whole edit"
        );
        wire.stop().await;
    });
}

#[test]
fn code_actions_decode_edits_and_commands() {
    run(async {
        let (client, rx, mut wire) = Wire::production();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        client.caps.set(lt::ServerCapabilities {
            code_action_provider: Some(lt::CodeActionProviderCapability::Simple(true)),
            position_encoding: Some(lt::PositionEncodingKind::UTF8),
            ..Default::default()
        });
        client.finish_initialize().unwrap();
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("hello world\n")
        ));
        let _open = wire.next().await;
        let wanted = client
            .code_actions(RequestInput {
                document,
                revision: BufferRevision::new(0),
                path: path.to_owned(),
                line: LineIndex::new(0),
                byte_col: ByteColumn::new(6),
                line_text: "hello world".into(),
                kind: RequestKind::CodeAction,
                rename_to: None,
            })
            .unwrap();
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/codeAction");
        // A zero-width range at the cursor with an empty diagnostics
        // context.
        assert_eq!(request["params"]["range"]["start"]["character"], 6);
        assert_eq!(request["params"]["range"]["end"]["character"], 6);
        assert_eq!(request["params"]["range"]["start"]["line"], 0);
        assert_eq!(request["params"]["range"]["end"]["line"], 0);
        assert_eq!(request["params"]["context"]["diagnostics"], json!([]));
        wire.reply(
            &request,
            json!([
                {
                    "title": "extract function",
                    "kind": "refactor.extract",
                    "edit": {
                        "changes": {
                            "file:///workspace/a.rs": [
                                { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 5 } }, "newText": "greeting()" }
                            ]
                        }
                    }
                },
                { "title": "open docs", "command": "docs.open" },
                {
                    "title": "move module",
                    "kind": "refactor.move",
                    "edit": {
                        "documentChanges": [
                            { "kind": "rename", "oldUri": "file:///workspace/a.rs", "newUri": "file:///workspace/m.rs" }
                        ]
                    }
                }
            ]),
        )
        .await;
        let LspEvent::ActionList { context, actions } = event(&rx).await else {
            panic!("actions")
        };
        assert_eq!(context.stamp, wanted);
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].title, "extract function");
        let edits = actions[0].edits.as_ref().expect("applicable edits");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].0.path, PathBuf::from("/workspace/a.rs"));
        assert_eq!(edits[0].1[0].new_text, "greeting()");
        assert!(!actions[0].has_external_command);
        // A command-only action: listed, executable elsewhere.
        assert_eq!(actions[1].title, "open docs");
        assert!(actions[1].edits.is_none());
        assert!(actions[1].has_external_command);
        // File operations cannot be applied: listed by title, marked
        // inapplicable by edits: None with no external command.
        assert_eq!(actions[2].title, "move module");
        assert!(actions[2].edits.is_none());
        assert!(!actions[2].has_external_command);
        wire.stop().await;
    });
}

#[test]
fn format_rename_and_code_action_refuse_admission_without_providers() {
    run(async {
        let (client, rx, mut wire) = Wire::production();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        // Ready, but none of the edit providers were advertised.
        client.caps.set(lt::ServerCapabilities {
            hover_provider: Some(lt::HoverProviderCapability::Simple(true)),
            position_encoding: Some(lt::PositionEncodingKind::UTF8),
            ..Default::default()
        });
        client.finish_initialize().unwrap();
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("x")
        ));
        let _open = wire.next().await;
        let input = |kind| RequestInput {
            document,
            revision: BufferRevision::new(0),
            path: path.to_owned(),
            line: LineIndex::new(0),
            byte_col: ByteColumn::new(0),
            line_text: "x".into(),
            kind,
            rename_to: None,
        };
        assert_eq!(
            client.format(document, BufferRevision::new(0), path.to_owned(), 4),
            Err(RequestRefusal::Unsupported)
        );
        assert_eq!(
            client.rename(input(RequestKind::Rename), "new"),
            Err(RequestRefusal::Unsupported)
        );
        assert_eq!(
            client.code_actions(input(RequestKind::CodeAction)),
            Err(RequestRefusal::Unsupported)
        );
        // No frame and no event: refusals never touch the wire.
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        wire.stop().await;
    });
}
