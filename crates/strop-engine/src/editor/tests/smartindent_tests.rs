use super::*;

#[test]
fn closer_dedents_on_indent_only_line() {
    // open a line inside fn f() { } — auto-indented, then '}' dedents
    let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
    e.feed_text("o"); // indented to one level
    assert_eq!(e.buf().line_text(1), "    ");
    e.feed_text("}"); // closer on the indent-only line → dedent first
                      // the new line sits at col 0; the file's own closing brace is untouched
    assert_eq!(e.buf().text().to_string(), "fn f() {\n}\n}\n");
}

#[test]
fn closer_noop_with_real_text_before() {
    let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
    e.feed_text("o"); // indented one level
    e.feed_text("let x = 1;"); // real text on the line
    e.feed_text("}"); // closer after text: no dedent
    assert_eq!(e.buf().line_text(1), "    let x = 1;}");
}
