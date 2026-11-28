use super::{IndentContext, SyntaxTree, WINDOW};
use crate::Highlighter;
use ropey::Rope;
use strop_core::id::{BufferRevision, ByteOffset};

fn parsed(path: &str, text: &str) -> (Rope, SyntaxTree) {
    let rope = Rope::from_str(text);
    let mut highlighter = Highlighter::for_path(std::path::Path::new(path), &rope).unwrap();
    highlighter
        .indentation_contexts(&rope, BufferRevision::new(1), &[], || false)
        .unwrap();
    (rope, highlighter.syntax_tree().unwrap())
}

fn at(tree: &SyntaxTree, rope: &Rope, position: usize) -> IndentContext {
    tree.indentation_contexts(rope, &[ByteOffset::new(position)], usize::MAX)
        .unwrap()[0]
}

fn opener(context: IndentContext) -> Option<usize> {
    context.opener.map(|open| open.position.get())
}

#[test]
fn the_walk_is_bounded_by_depth_not_by_the_cursors_distance_from_the_top() {
    let mut text = String::new();
    for index in 0..100_000 {
        text.push_str(&format!("fn f{index}() {{\n    let x = {index};\n}}\n"));
    }
    text.push_str("fn last() {\n    if x {\n        call(1, 2");
    let (rope, tree) = parsed("big.rs", &text);
    let end = text.len();
    let mut depth = 0;
    let mut node = tree
        .tree
        .root_node()
        .descendant_for_byte_range(end - 1, end);
    while let Some(current) = node {
        depth += 1;
        node = current.parent();
    }
    let budget = depth * (2 * WINDOW + 2);
    let contexts = tree
        .indentation_contexts(&rope, &[ByteOffset::new(end)], budget)
        .expect("a lookup at the end of a 100k-function file fits depth × k");
    assert_eq!(opener(contexts[0]), text.rfind('('));
    assert!(contexts[0].code);
    // the same budget answers at the top and in the middle
    let middle = text.find("fn f50000").unwrap() + "fn f50000() {\n    let".len();
    let contexts = tree
        .indentation_contexts(&rope, &[ByteOffset::new(middle)], budget)
        .unwrap();
    assert_eq!(
        opener(contexts[0]),
        Some(text.find("fn f50000").unwrap() + 12)
    );
}

#[test]
fn a_long_block_finds_its_brace_past_the_examined_statements() {
    let mut text = String::from("fn main() {\n");
    for index in 0..1000 {
        text.push_str(&format!("    let v{index} = [{index}];\n"));
    }
    let (rope, tree) = parsed("block.rs", &text);
    let context = at(&tree, &rope, text.len() - 1);
    assert_eq!(opener(context), Some(10));
    assert!(context.code);
}

#[test]
fn literals_comments_and_their_ends() {
    let text = "fn f() {\n    let s = \"{\";\n    // {\n    /* { */\n}\n";
    let (rope, tree) = parsed("code.rs", text);
    let inside = text.find("\"{").unwrap() + 2;
    assert!(!at(&tree, &rope, inside).code);
    let after = text.find("\";").unwrap() + 1;
    let context = at(&tree, &rope, after);
    assert!(context.code);
    assert_eq!(opener(context), Some(7));
    let line_comment_end = text.find("// {").unwrap() + 4;
    assert!(!at(&tree, &rope, line_comment_end).code);
    let block_comment_end = text.find("*/").unwrap() + 2;
    assert!(at(&tree, &rope, block_comment_end).code);
}

#[test]
fn incomplete_code_reads_its_recovered_brackets() {
    // an unclosed call two statements up, then a complete statement
    let text = "fn f() {\n    call(a,\n        b);\n    let x = [1,\n";
    let (rope, tree) = parsed("code.rs", text);
    let context = at(&tree, &rope, text.len() - 1);
    assert_eq!(opener(context), text.rfind('['));
    let text = "    let text = r#\"{\n    {";
    let (rope, tree) = parsed("code.rs", text);
    let context = at(&tree, &rope, text.len());
    assert!(!context.code);
    assert_eq!(context.opener, None);
}

#[test]
fn positions_before_the_first_token_and_after_the_last() {
    let text = "\n\n{ x }\n\n";
    let (rope, tree) = parsed("code.json", text);
    let start = at(&tree, &rope, 0);
    assert!(start.code);
    assert_eq!(start.opener, None);
    let inside = at(&tree, &rope, 3);
    assert_eq!(opener(inside), Some(2));
    let end = at(&tree, &rope, text.len());
    assert!(end.code);
    assert_eq!(end.opener, None);
}
