use super::*;

#[test]
fn vi_paren_selects_inner() {
    let mut e = Editor::new(Buffer::from_text("call(a, b)\n"));
    e.feed_text("f("); // onto the open paren
    e.feed_text("vi(");
    let r = e.visual_range().expect("visual range");
    assert_eq!(e.buf().slice_string(r), "a, b");
    // and operators consume it
    e.feed_text("d");
    assert_eq!(e.buf().text().to_string(), "call()\n");
}

#[test]
fn va_quote_includes_quotes() {
    let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
    e.feed_text("w"); // onto "hi"
    e.feed_text("va\"");
    let r = e.visual_range().expect("visual range");
    assert_eq!(e.buf().slice_string(r), "\"hi\"");
}
