use std::sync::mpsc::channel;
use std::sync::LazyLock;

fn main() {
    let (tx, rx) = channel();
    static CFG: LazyLock<strop_lsp::languages::Languages> = LazyLock::new(Default::default);
    let spec = strop_lsp::registry::for_extension(".rs", &CFG).unwrap();
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
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(ev) => match &ev {
                // capabilities arrive with initialize — hover after Ready
                // or the capability gate (0009 §2.5) drops it
                strop_lsp::LspEvent::Ready { server } => {
                    println!("READY {server}");
                    client.hover(&root.join("src/main.rs"), 1, 8);
                }
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
