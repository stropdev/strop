//! Static grammar build (0002 §2.2): compiles the committed, generated
//! Sovel parser C into the crate. Regeneration happens only through the
//! pinned recipe in `grammars/sovel/README.md` — never here: no CLI, no
//! node, no downloads in the locked build path.
fn main() {
    let grammar = std::path::Path::new("grammars/sovel/src");
    cc::Build::new()
        .include(grammar)
        .file(grammar.join("parser.c"))
        .compile("tree-sitter-sovel");
    println!("cargo::rerun-if-changed=grammars/sovel/src/parser.c");
}
