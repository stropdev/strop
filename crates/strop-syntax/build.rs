//! Static grammar build (0002 §2.2): compiles the committed, generated
//! parser C into the crate. Regeneration happens only through the
//! pinned recipe in each grammar's README — never here: no CLI, no
//! node, no downloads in the locked build path.
fn main() {
    let sovel = std::path::Path::new("grammars/sovel/src");
    cc::Build::new()
        .include(sovel)
        .file(sovel.join("parser.c"))
        .compile("tree-sitter-sovel");
    println!("cargo::rerun-if-changed=grammars/sovel/src/parser.c");

    let weft = std::path::Path::new("grammars/weft/src");
    cc::Build::new()
        .include(weft)
        .file(weft.join("parser.c"))
        .compile("tree-sitter-weft");
    println!("cargo::rerun-if-changed=grammars/weft/src/parser.c");

    // Splicery adds the grammar's one external scanner (heredocs): a
    // dynamic terminator line cannot be a regex token.
    let splicery = std::path::Path::new("grammars/splicery/src");
    cc::Build::new()
        .include(splicery)
        .file(splicery.join("parser.c"))
        .file(splicery.join("scanner.c"))
        .compile("tree-sitter-splicery");
    println!("cargo::rerun-if-changed=grammars/splicery/src/parser.c");
    println!("cargo::rerun-if-changed=grammars/splicery/src/scanner.c");
}
