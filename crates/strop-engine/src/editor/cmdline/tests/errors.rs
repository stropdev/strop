//! Errors that look like errors (plan 0110 step 6): a failed line says
//! so as a failure; a command that worked does not.

use super::editor;

#[test]
fn every_way_a_line_fails_is_a_failure() {
    for (keys, message) in [
        (":wrte<cr>", "not an editor command: wrte"),
        (":9d<cr>", "invalid range"),
        (":'qd<cr>", "mark not set"),
        (":2pwd<cr>", "no range allowed"),
        (":d x y<cr>", "trailing characters: y"),
        (":m9<cr>", "invalid range"),
        (":set frob<cr>", "unknown option: frob"),
        (":k<cr>", "argument required"),
        (":sort z<cr>", "unsupported :sort argument: z"),
        (":@q<cr>", "nothing in register q"),
        (":s/zzz/y/<cr>", "pattern not found: zzz"),
        (":b 99<cr>", "buffer 99 does not exist"),
        ("/zzz<cr>", "pattern not found"),
    ] {
        let mut e = editor("one\ntwo\nthree\n");
        e.feed_text(keys);
        assert!(e.message.starts_with(message), "{keys}: {:?}", e.message);
        assert!(
            e.message_failed(),
            "{keys}: {:?} is not a failure",
            e.message
        );
    }
}

#[test]
fn a_command_that_works_is_not_a_failure_and_clears_one() {
    let mut e = editor("one\ntwo\nthree\nfour\n");
    e.feed_text(":wrte<cr>");
    assert!(e.message_failed());
    e.feed_text(":1,3d<cr>");
    assert_eq!(e.message, "3 fewer lines");
    assert!(!e.message_failed());
    e.feed_text(":frob<cr>j");
    assert!(
        e.message.is_empty() && !e.message_failed(),
        "a key clears it"
    );
}

#[test]
fn messages_keeps_the_failure_kind() {
    let mut e = editor("one\n");
    e.feed_text(":wrte<cr>");
    e.log_echo_message();
    e.feed_text(":messages<cr>");
    let text = e.buf().text().to_string();
    assert!(text.contains("✗ not an editor command: wrte"), "{text}");
}
