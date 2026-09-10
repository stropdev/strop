use super::*;
use crate::protocol::*;
use async_lsp::{lsp_types as lt, router::Router};
use ropey::Rope;
use serde_json::{json, Value};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{
    mpsc::{channel, Receiver, TryRecvError},
    Arc,
};
use strop_core::id::{Arena, BufferRevision, ByteColumn, DocumentKind, LineIndex};
use tokio::io::{
    AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf,
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

type Documents = Arena<DocumentKind, ()>;

struct Wire {
    reader: BufReader<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
    task: tokio::task::JoinHandle<()>,
}

impl Wire {
    /// An in-memory client: real mainloop, real wire queue, duplex peer.
    fn new() -> (Client, Receiver<LspEvent>, Self) {
        Self::with_router(|_, _, _, _, _| Router::new(()))
    }

    /// The PRODUCTION router (spawn.rs's handlers), so notification
    /// tolerance is tested exactly as shipped.
    fn production() -> (Client, Receiver<LspEvent>, Self) {
        Self::with_router(|_, tx, id, caps, sync| {
            super::spawn::client_router(
                tx,
                id,
                caps,
                sync,
                crate::target::Workspace::Local {
                    root: PathBuf::from("/workspace"),
                },
                "prod".into(),
                None,
            )
        })
    }

    /// The builder receives the parts the client will own, so a custom
    /// router's state and the client share one identity.
    fn with_router<St: Send + 'static>(
        build: impl FnOnce(
            async_lsp::ServerSocket,
            Sender<LspEvent>,
            ServerId,
            ServerCaps,
            Arc<parking_lot::Mutex<sync::SyncState>>,
        ) -> Router<St>,
    ) -> (Client, Receiver<LspEvent>, Self) {
        let (tx, rx) = channel();
        let id = ServerId::allocate();
        let caps = ServerCaps::default();
        let sync = Arc::new(parking_lot::Mutex::new(sync::SyncState::default()));
        let (mainloop, socket) = async_lsp::MainLoop::new_client({
            let tx = tx.clone();
            let caps = caps.clone();
            let sync = sync.clone();
            move |server| build(server, tx, id, caps, sync)
        });
        let (client_io, peer_io) = tokio::io::duplex(65536);
        let (input, output) = tokio::io::split(client_io);
        let task = tokio::spawn(async move {
            let _ = mainloop
                .run_buffered(input.compat(), output.compat_write())
                .await;
        });
        let client = Client {
            id,
            next_request: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            sync: sync.clone(),
            socket: socket.clone(),
            handle: tokio::runtime::Handle::current(),
            tx: tx.clone(),
            workspace: crate::target::Workspace::Local {
                root: PathBuf::from("/workspace"),
            },
            caps: caps.clone(),
            quitting: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            thread: Arc::new(std::sync::Mutex::new(None)),
            stop: Arc::new(ServiceStop(parking_lot::Mutex::new(None))),
            queue: queue::start(queue::WireEnv {
                id,
                name: "in-memory".into(),
                hint: String::new(),
                socket,
                handle: tokio::runtime::Handle::current(),
                tx,
                caps,
                workspace: crate::target::Workspace::Local {
                    root: PathBuf::from("/workspace"),
                },
                sync,
                quitting: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            })
            .expect("wire worker"),
        };
        let (reader, writer) = tokio::io::split(peer_io);
        (
            client,
            rx,
            Self {
                reader: BufReader::new(reader),
                writer,
                task,
            },
        )
    }

    async fn next(&mut self) -> Value {
        let mut length = None;
        loop {
            let mut line = String::new();
            assert_ne!(
                self.reader.read_line(&mut line).await.unwrap(),
                0,
                "unexpected EOF"
            );
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.expect("Content-Length")];
        self.reader.read_exact(&mut body).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn reply(&mut self, request: &Value, result: Value) {
        self.respond(request["id"].clone(), json!({ "result": result }))
            .await;
    }

    async fn reply_error(&mut self, request: &Value, code: i64, message: &str) {
        self.respond(
            request["id"].clone(),
            json!({ "error": { "code": code, "message": message } }),
        )
        .await;
    }

    async fn answer(&mut self, request: &Value, text: &str) {
        self.reply(request, json!({ "contents": text })).await;
    }

    async fn respond(&mut self, id: Value, body: Value) {
        let mut payload = json!({ "jsonrpc": "2.0", "id": id });
        if let (Value::Object(payload), Value::Object(body)) = (&mut payload, body) {
            for (key, value) in body {
                payload.insert(key, value);
            }
        }
        let bytes = serde_json::to_vec(&payload).unwrap();
        self.writer
            .write_all(format!("Content-Length: {}\r\n\r\n", bytes.len()).as_bytes())
            .await
            .unwrap();
        self.writer.write_all(&bytes).await.unwrap();
        self.writer.flush().await.unwrap();
    }

    /// A server-to-client notification frame (no id).
    async fn notify(&mut self, method: &str, params: Value) {
        let payload = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let bytes = serde_json::to_vec(&payload).unwrap();
        self.writer
            .write_all(format!("Content-Length: {}\r\n\r\n", bytes.len()).as_bytes())
            .await
            .unwrap();
        self.writer.write_all(&bytes).await.unwrap();
        self.writer.flush().await.unwrap();
    }

    async fn stop(self) {
        self.task.abort();
        assert!(self.task.await.unwrap_err().is_cancelled());
    }
}

fn run(future: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), future)
            .await
            .expect("in-memory protocol stalled");
    });
}

fn initialize(client: &Client) {
    client.caps.set(lt::ServerCapabilities {
        hover_provider: Some(lt::HoverProviderCapability::Simple(true)),
        definition_provider: Some(lt::OneOf::Left(true)),
        references_provider: Some(lt::OneOf::Left(true)),
        position_encoding: Some(lt::PositionEncodingKind::UTF8),
        ..Default::default()
    });
    client.finish_initialize().unwrap();
}

fn ask(client: &Client, document: strop_core::id::DocumentId, revision: u64) -> RequestStamp {
    client
        .request(RequestInput {
            document,
            revision: BufferRevision::new(revision),
            path: PathBuf::from("/workspace/a.rs"),
            line: LineIndex::new(0),
            byte_col: ByteColumn::new(5),
            line_text: "a😀z".into(),
            kind: RequestKind::Hover,
            rename_to: None,
        })
        .unwrap()
}

async fn event(rx: &Receiver<LspEvent>) -> LspEvent {
    loop {
        match rx.try_recv() {
            Ok(event) => return event,
            Err(TryRecvError::Disconnected) => panic!("event sender disconnected"),
            Err(TryRecvError::Empty) => tokio::task::yield_now().await,
        }
    }
}

#[test]
fn preinit_close_discards_old_open_and_queued_requests_and_converts_after_negotiation() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let old = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        assert!(client.did_open(
            old,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("old")
        ));
        let cancelled = ask(&client, old, 0);
        assert!(client.did_change(
            old,
            BufferRevision::new(1),
            path,
            Rope::from_str("unsaved old")
        ));
        client.did_close(old, path);
        docs.remove(old);
        let reopened = docs.insert(());
        assert!(client.did_open(
            reopened,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("a😀z")
        ));
        let wanted = ask(&client, reopened, 0);
        initialize(&client);
        let open = wire.next().await;
        assert_eq!(open["method"], "textDocument/didOpen");
        assert_eq!(open["params"]["textDocument"]["text"], "a😀z");
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/hover");
        assert_eq!(request["params"]["position"]["character"], 5);
        wire.answer(&request, "new incarnation").await;
        let LspEvent::Note {
            context: cancelled_context,
            text: cancel_text,
        } = event(&rx).await
        else {
            panic!("cancel note for the closed incarnation")
        };
        assert_eq!(cancelled_context.stamp, cancelled);
        assert_eq!(cancel_text, "cancelled \u{2014} the document closed");
        let LspEvent::HoverText { context, text } = event(&rx).await else {
            panic!("hover")
        };
        assert_eq!(text, "new incarnation");
        assert_eq!(context.stamp, wanted);
        assert_eq!(context.encoding, PositionEncoding::Utf8);
        wire.stop().await;
    });
}

#[test]
fn preinit_change_is_coalesced_into_first_open() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("old")
        ));
        assert!(client.did_change(
            document,
            BufferRevision::new(1),
            path,
            Rope::from_str("rust")
        ));
        // A second pre-init change replaces the queued snapshot again.
        assert!(client.did_change(
            document,
            BufferRevision::new(2),
            path,
            Rope::from_str("a😀z")
        ));
        let wanted = ask(&client, document, 2);
        initialize(&client);
        let open = wire.next().await;
        assert_eq!(open["method"], "textDocument/didOpen");
        assert_eq!(open["params"]["textDocument"]["text"], "a😀z");
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/hover");
        wire.answer(&request, "changed before init").await;
        let LspEvent::HoverText { context, .. } = event(&rx).await else {
            panic!("hover")
        };
        assert_eq!(context.stamp, wanted);
        wire.stop().await;
    });
}

#[test]
fn live_close_reopen_sends_fresh_content_and_reordered_replies_keep_original_owners() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let old = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        initialize(&client);
        assert!(client.did_open(
            old,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("a😀z")
        ));
        let first_open = wire.next().await;
        let old_stamp = ask(&client, old, 0);
        let old_request = wire.next().await;
        client.did_close(old, path);
        docs.remove(old);
        let new = docs.insert(());
        assert!(client.did_open(
            new,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("externally changed")
        ));
        let close = wire.next().await;
        assert_eq!(close["method"], "textDocument/didClose");
        assert_eq!(
            close["params"]["textDocument"]["uri"],
            first_open["params"]["textDocument"]["uri"]
        );
        let reopened = wire.next().await;
        assert_eq!(reopened["method"], "textDocument/didOpen");
        assert_eq!(
            reopened["params"]["textDocument"]["text"],
            "externally changed"
        );
        assert!(
            reopened["params"]["textDocument"]["version"]
                .as_i64()
                .unwrap()
                > first_open["params"]["textDocument"]["version"]
                    .as_i64()
                    .unwrap()
        );
        let new_stamp = client
            .request(RequestInput {
                document: new,
                revision: BufferRevision::new(0),
                path: path.to_owned(),
                line: LineIndex::new(0),
                byte_col: ByteColumn::new(5),
                line_text: "externally changed".into(),
                kind: RequestKind::Hover,
                rename_to: None,
            })
            .unwrap();
        let new_request = wire.next().await;
        wire.answer(&new_request, "fresh").await;
        let LspEvent::HoverText {
            context: fresh,
            text,
        } = event(&rx).await
        else {
            panic!("hover")
        };
        assert_eq!(text, "fresh");
        assert_eq!(fresh.stamp, new_stamp);
        wire.answer(&old_request, "late").await;
        let LspEvent::HoverText {
            context: late,
            text,
        } = event(&rx).await
        else {
            panic!("hover")
        };
        assert_eq!(text, "late");
        assert_eq!(late.stamp, old_stamp);
        assert_ne!(fresh.stamp.document, late.stamp.document);
        assert_eq!(fresh.stamp.revision, late.stamp.revision);
        wire.stop().await;
    });
}

#[test]
fn wire_order_is_admission_order_change_before_request() {
    run(async {
        let (client, _rx, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        initialize(&client);
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("v0\n")
        ));
        assert!(client.did_change(
            document,
            BufferRevision::new(1),
            path,
            Rope::from_str("v1 full text\n")
        ));
        ask(&client, document, 1);
        let open = wire.next().await;
        assert_eq!(open["method"], "textDocument/didOpen");
        let change = wire.next().await;
        assert_eq!(change["method"], "textDocument/didChange");
        assert_eq!(
            change["params"]["contentChanges"][0]["text"],
            "v1 full text\n"
        );
        assert_eq!(
            change["params"]["textDocument"]["version"]
                .as_i64()
                .unwrap(),
            open["params"]["textDocument"]["version"].as_i64().unwrap() + 1
        );
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/hover");
        wire.stop().await;
    });
}

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

#[test]
fn unsupported_capability_refuses_admission_without_wire_traffic() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        // Ready, but hover was never advertised.
        client.caps.set(lt::ServerCapabilities {
            definition_provider: Some(lt::OneOf::Left(true)),
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
        let refused = client.request(RequestInput {
            document,
            revision: BufferRevision::new(0),
            path: path.to_owned(),
            line: LineIndex::new(0),
            byte_col: ByteColumn::new(0),
            line_text: "x".into(),
            kind: RequestKind::Hover,
            rename_to: None,
        });
        assert_eq!(refused, Err(RequestRefusal::Unsupported));
        // No frame and no event: refusals never touch the wire.
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        wire.stop().await;
    });
}

#[test]
fn stale_revision_and_unopened_documents_refuse_admission() {
    run(async {
        let (client, _rx, mut wire) = Wire::new();
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
        let stale = client.request(RequestInput {
            document,
            revision: BufferRevision::new(7),
            path: path.to_owned(),
            line: LineIndex::new(0),
            byte_col: ByteColumn::new(0),
            line_text: "x".into(),
            kind: RequestKind::Hover,
            rename_to: None,
        });
        assert_eq!(stale, Err(RequestRefusal::StaleRevision));
        let elsewhere = client.request(RequestInput {
            document,
            revision: BufferRevision::new(0),
            path: PathBuf::from("/workspace/b.rs"),
            line: LineIndex::new(0),
            byte_col: ByteColumn::new(0),
            line_text: "x".into(),
            kind: RequestKind::Hover,
            rename_to: None,
        });
        assert_eq!(elsewhere, Err(RequestRefusal::NotOpen));
        wire.stop().await;
    });
}

#[test]
fn default_negotiation_is_utf16_and_converts_columns() {
    run(async {
        let (client, rx, mut wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        // No positionEncoding in the server capabilities: spec default.
        client.caps.set(lt::ServerCapabilities {
            hover_provider: Some(lt::HoverProviderCapability::Simple(true)),
            ..Default::default()
        });
        client.finish_initialize().unwrap();
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("a😀z")
        ));
        let _open = wire.next().await;
        let wanted = ask(&client, document, 0);
        let request = wire.next().await;
        assert_eq!(request["method"], "textDocument/hover");
        // byte col 5 ('z' after a + emoji) is UTF-16 unit 3.
        assert_eq!(request["params"]["position"]["character"], 3);
        wire.answer(&request, "utf16 world").await;
        let LspEvent::HoverText { context, .. } = event(&rx).await else {
            panic!("hover")
        };
        assert_eq!(context.stamp, wanted);
        assert_eq!(context.encoding, PositionEncoding::Utf16);
        wire.stop().await;
    });
}

#[test]
fn content_modified_retries_once_and_keeps_the_original_context() {
    // The 800ms retry delay is the runtime retry policy itself; the
    // pause-free clock keeps this an honest end-to-end check.
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
        // First attempt: server still indexing. Retry policy kicks in,
        // then the same request succeeds with the same stamp.
        wire.reply_error(&request, -32801, "content modified").await;
        let retry = wire.next().await;
        assert_eq!(retry["method"], "textDocument/hover");
        assert_ne!(
            retry["id"], request["id"],
            "the wire retry is a new json-rpc request"
        );
        wire.answer(&retry, "after retry").await;
        let LspEvent::HoverText { context, text } = event(&rx).await else {
            panic!("hover")
        };
        assert_eq!(text, "after retry");
        assert_eq!(context.stamp, wanted);
        wire.stop().await;
    });
}

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

mod spawn;
mod startup;
