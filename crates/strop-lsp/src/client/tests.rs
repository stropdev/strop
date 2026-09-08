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
        let (tx, rx) = channel();
        let (mainloop, socket) = async_lsp::MainLoop::new_client(|_| Router::new(()));
        let (client_io, peer_io) = tokio::io::duplex(65536);
        let (input, output) = tokio::io::split(client_io);
        let task = tokio::spawn(async move {
            let _ = mainloop
                .run_buffered(input.compat(), output.compat_write())
                .await;
        });
        let caps = ServerCaps::default();
        let sync = Arc::new(parking_lot::Mutex::new(sync::SyncState::default()));
        let id = ServerId::allocate();
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
fn preinit_close_cancels_queued_requests_with_terminal_notes() {
    run(async {
        let (client, rx, wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("x")
        ));
        let wanted = ask(&client, document, 0);
        client.did_close(document, path);
        let LspEvent::Note { context, text } = event(&rx).await else {
            panic!("note")
        };
        assert_eq!(context.stamp, wanted);
        assert_eq!(text, "cancelled — the document closed");
        initialize(&client);
        // Nothing was ever framed: the open was cancelled pre-init.
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        wire.stop().await;
    });
}
