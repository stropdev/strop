#[test]
fn dbg_right_resolve() {
    let buf = strop_core::Buffer::from_text("café münchen\n");
    let cmd = match strop_grammar::parse("l") { strop_grammar::Parse::Complete(c) => c, _ => panic!() };
    let mut cur = 3;
    let r = strop_grammar::resolve(&buf, cur, &cmd).unwrap();
    eprintln!("range {:?} inclusive {}", r.range, r.inclusive);
    let after = strop_grammar::cursor_after(&buf, cur, &cmd, &r);
    eprintln!("cursor_after from {} = {}", cur, after);
}
