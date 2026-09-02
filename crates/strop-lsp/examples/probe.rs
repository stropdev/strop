use std::sync::mpsc::channel;

fn main() {
    let (tx, rx) = channel();
    let spec = strop_lsp::registry::for_extension(".rs").unwrap();
    let root = std::path::Path::new("/tmp/lsp-proj");
    let Some(client) = strop_lsp::Client::spawn(&spec, root, tx) else {
        println!("spawn failed");
        return;
    };
    client.did_open(
        &root.join("src/main.rs"),
        "rust",
        &std::fs::read_to_string("/tmp/lsp-proj/src/main.rs").unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    std::thread::sleep(std::time::Duration::from_millis(1200)); // let diagnostics settle first
    client.hover(&root.join("src/main.rs"), 1, 8);
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(ev) => match &ev {
                strop_lsp::LspEvent::Ready { server } => println!("READY {server}"),
                strop_lsp::LspEvent::Failed { server, hint } => println!("FAILED {server}: {hint}"),
                strop_lsp::LspEvent::Diagnostics { path, diags } => {
                    println!(
                        "DIAGS {}: {:?}",
                        path.display(),
                        diags
                            .iter()
                            .map(|d| (d.line, d.severity_char()))
                            .collect::<Vec<_>>()
                    );
                }
                strop_lsp::LspEvent::HoverText { text } => {
                    println!("HOVER: {:?}", text.chars().take(80).collect::<String>());
                    return;
                }
                _ => {}
            },
            Err(_) => print!("."),
        }
    }
    println!("\ntimeout — no diagnostics");
}
