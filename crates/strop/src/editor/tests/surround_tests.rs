use super::*;

#[test]
fn ds_deletes_pair() {
    let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
    e.feed_text("w"); // onto "hi"
    e.feed_text("ds\"");
    assert_eq!(e.buf().text().to_string(), "say hi now\n");
    // and undo restores the pair as one unit
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "say \"hi\" now\n");
}

#[test]
fn cs_changes_pair() {
    let mut e = Editor::new(Buffer::from_text("call(a, b)\n"));
    e.feed_text("f("); // onto the open paren (on-pair counts as inside)
    e.feed_text("cs(["); // change (…) to […]
    assert_eq!(e.buf().text().to_string(), "call[a, b]\n");
    // and undo restores as one unit
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "call(a, b)\n");
}

#[test]
fn ysiw_wraps_word() {
    let mut e = Editor::new(Buffer::from_text("make it sharp\n"));
    e.feed_text("w"); // onto "it"
    e.feed_text("ysiw\"");
    assert_eq!(e.buf().text().to_string(), "make \"it\" sharp\n");
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "make it sharp\n");
}

#[test]
fn visual_s_wraps_selection() {
    let mut e = Editor::new(Buffer::from_text("wrap me up\n"));
    e.feed_text("ve"); // select "wrap"
    e.feed_text("S(");
    assert_eq!(e.buf().text().to_string(), "(wrap) me up\n");
}
