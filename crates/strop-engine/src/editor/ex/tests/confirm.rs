//! `:s///c`: the question, its answers, and who owns the keys meanwhile.

use strop_core::frontend_input::Key;
use strop_core::Range;

use super::{editor, text};
use crate::editor::InputOwner;

const TEXT: &str = "a a\nb\na a\n";

fn asking(answers: &str) -> crate::editor::Editor {
    let mut e = editor(TEXT);
    e.feed_text(":%s/a/X/gc<cr>");
    e.feed_text(answers);
    e
}

#[test]
fn the_question_owns_input_and_points_at_the_match() {
    let e = asking("");
    assert_eq!(e.input_owner(), InputOwner::SubstituteConfirm);
    assert_eq!(e.message(), "replace with X (y/n/a/q/l)?");
    assert_eq!(e.confirm_range(), Some(Range::charwise(0, 1)));
    assert_eq!(e.head(), 0);
    assert_eq!(text(&e), TEXT, "nothing changes before an answer");
}

#[test]
fn yes_replaces_and_moves_to_the_next_match() {
    let e = asking("y");
    assert_eq!(text(&e), "X a\nb\na a\n");
    assert_eq!(e.confirm_range(), Some(Range::charwise(2, 3)));
    assert_eq!(e.input_owner(), InputOwner::SubstituteConfirm);
}

#[test]
fn no_skips_the_match() {
    let e = asking("ny");
    assert_eq!(text(&e), "a X\nb\na a\n");
    assert_eq!(e.confirm_range(), Some(Range::charwise(6, 7)));
}

#[test]
fn all_replaces_the_rest_and_ends() {
    let e = asking("na");
    assert_eq!(text(&e), "a X\nb\nX X\n");
    assert_eq!(e.input_owner(), InputOwner::Document);
    assert_eq!(e.confirm_range(), None);
    assert_eq!(e.message(), "3 substitutions");
}

#[test]
fn last_replaces_this_one_and_ends() {
    let e = asking("nl");
    assert_eq!(text(&e), "a X\nb\na a\n");
    assert_eq!(e.input_owner(), InputOwner::Document);
    assert_eq!(e.message(), "1 substitution");
}

#[test]
fn quit_and_escape_end_keeping_what_was_replaced() {
    for stop in ["q", "<esc>"] {
        let e = asking(&format!("y{stop}"));
        assert_eq!(text(&e), "X a\nb\na a\n", "{stop}");
        assert_eq!(e.input_owner(), InputOwner::Document, "{stop}");
    }
}

#[test]
fn any_other_key_asks_again_and_is_not_typed() {
    let mut e = asking("x");
    assert_eq!(text(&e), TEXT);
    assert_eq!(e.input_owner(), InputOwner::SubstituteConfirm);
    assert_eq!(e.message(), "replace with X (y/n/a/q/l)?");
    e.feed(Key::Enter);
    assert_eq!(e.confirm_range(), Some(Range::charwise(0, 1)));
}

#[test]
fn answering_every_match_ends_the_question() {
    let e = asking("ynyn");
    assert_eq!(text(&e), "X a\nb\nX a\n");
    assert_eq!(e.input_owner(), InputOwner::Document);
    assert_eq!(e.message(), "2 substitutions");
}

#[test]
fn the_whole_confirmation_is_one_undo_step() {
    let mut e = asking("yyyy");
    assert_eq!(text(&e), "X X\nb\nX X\n");
    e.feed_text("u");
    assert_eq!(text(&e), TEXT);
}

#[test]
fn the_replacement_asked_about_is_the_expanded_one() {
    let mut e = editor("key=value\n");
    e.feed_text(":s/\\(key\\)=\\(value\\)/\\2=\\1/c<cr>");
    assert_eq!(e.message(), "replace with value=key (y/n/a/q/l)?");
    e.feed_text("y");
    assert_eq!(text(&e), "value=key\n");
}

#[test]
fn the_question_is_refused_inside_a_compound_command() {
    let mut e = editor(TEXT);
    e.feed_text(":g/a/s/a/X/c<cr>");
    assert_eq!(text(&e), TEXT);
    assert_eq!(e.input_owner(), InputOwner::Document);
    assert_eq!(e.message(), "the c flag cannot ask inside :g or :normal");
}

#[test]
fn a_buffer_that_changed_under_the_question_stops_it() {
    let mut e = asking("");
    e.buf_mut().edit().insert(0, "external ").unwrap();
    e.feed_text("y");
    assert_eq!(text(&e), format!("external {TEXT}"));
    assert_eq!(e.input_owner(), InputOwner::Document);
    assert_eq!(e.message(), "substitute stopped: the buffer changed");
}
