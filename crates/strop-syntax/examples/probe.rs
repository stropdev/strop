use streaming_iterator::StreamingIterator;
use tree_sitter::QueryCursor;

fn probe(name: &str, lang: tree_sitter::Language, query_src: &str, src: &str) {
    let mut p = tree_sitter::Parser::new();
    p.set_language(&lang).unwrap();
    let tree = p.parse(src, None).unwrap();
    println!(
        "{name} sexp: {}",
        tree.root_node()
            .to_sexp()
            .chars()
            .take(120)
            .collect::<String>()
    );
    let q = tree_sitter::Query::new(&lang, query_src).unwrap();
    let mut c = QueryCursor::new();
    let mut m = c.matches(&q, tree.root_node(), src.as_bytes());
    let mut n = 0;
    while let Some(m_) = m.next() {
        for cap in m_.captures {
            println!(
                "  {name} cap: {} @ {:?}",
                q.capture_names()[cap.index as usize],
                &src[cap.node.start_byte()..cap.node.end_byte()]
            );
        }
        n += 1;
        if n > 5 {
            break;
        }
    }
    println!("{name} matches: {n}");
}

fn main() {
    probe(
        "cpp",
        tree_sitter_cpp::LANGUAGE.into(),
        tree_sitter_cpp::HIGHLIGHT_QUERY,
        "#include <vector>\nstruct Edge { int sharp; };\n",
    );
    probe(
        "ts",
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        tree_sitter_typescript::HIGHLIGHTS_QUERY,
        "const x: number = 1;\n",
    );
}
