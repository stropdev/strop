use ropey::Rope;
use std::sync::mpsc::channel;
use std::sync::LazyLock;

fn main() {
    let (tx, rx) = channel();
    static CFG: LazyLock<strop_lsp::languages::Languages> = LazyLock::new(Default::default);
    let Some(spec) = strop_lsp::registry::for_extension(".rs", &CFG) else {
        eprintln!("no Rust language server configured");
        return;
    };
    let root = std::path::Path::new("/tmp/lsp-proj");
    // Client::spawn takes the caller's cancellation token; the example
    // has no worker, so one is scoped in purely to issue the token.
    let client = std::thread::scope(|scope| {
        let (tokens, issued) = channel();
        let _handle = strop_core::worker::spawn_scoped(
            scope,
            "probe-token",
            |_| {},
            move |token| {
                let _ = tokens.send(token);
                strop_core::worker::Outcome::Success(())
            },
        );
        let token = issued.recv().expect("worker issued token");
        strop_lsp::Client::spawn(
            &spec,
            strop_lsp::Workspace::Local {
                root: root.to_path_buf(),
            },
            tx,
            &token,
        )
    });
    let client = match client {
        Ok(client) => client,
        Err(error) => {
            println!("spawn failed: {error}");
            return;
        }
    };
    let path = root.join("src/main.rs");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("cannot read {}: {error}", path.display());
            return;
        }
    };
    let mut documents = strop_core::id::Arena::<strop_core::id::DocumentKind, ()>::default();
    let document = documents.try_insert(()).unwrap();
    let revision = strop_core::id::BufferRevision::new(0);
    client.did_open(document, revision, &path, "rust", Rope::from_str(&text));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(ev) => match &ev {
                // Request after capabilities arrive with initialize.
                strop_lsp::LspEvent::Ready { name, .. } => {
                    println!("READY {name}");
                    let line_text = match text.lines().nth(1) {
                        Some(line) => line.to_owned(),
                        None => String::new(),
                    };
                    let admitted = client.request(strop_lsp::RequestInput {
                        document,
                        revision,
                        path: path.clone(),
                        line: strop_core::id::LineIndex::new(1),
                        byte_col: strop_core::id::ByteColumn::new(8),
                        line_text: line_text.into(),
                        kind: strop_lsp::RequestKind::Hover,
                        rename_to: None,
                    });
                    if let Err(refusal) = admitted {
                        println!("REFUSED {refusal:?}");
                    }
                }
                strop_lsp::LspEvent::Failed { name, hint, .. } => println!("FAILED {name}: {hint}"),
                strop_lsp::LspEvent::Diagnostics { doc, diags, .. } => {
                    println!(
                        "DIAGS {}: {:?}",
                        doc.label(),
                        diags
                            .iter()
                            .map(|d| (d.line.get(), d.severity.char()))
                            .collect::<Vec<_>>()
                    );
                }
                strop_lsp::LspEvent::HoverText { text, .. } => {
                    println!("HOVER: {:?}", text.chars().take(80).collect::<String>());
                    client.did_close(document, &path);
                    return;
                }
                _ => {}
            },
            Err(_) => print!("."),
        }
    }
    println!("\ntimeout — no diagnostics");
}
