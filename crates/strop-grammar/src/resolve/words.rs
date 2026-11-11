//! Word motions and the word text object, ported from Vim's
//! `fwd_word` / `bck_word` / `end_word` / `bckend_word` / `current_word`
//! over the same cursor model (`walk`). Each returns `false` where Vim
//! returns FAIL: the walk ran out of buffer.

use strop_core::Buffer;

use super::walk::{Step, Walk, BLANK};

/// The keyword class shared by motions, text objects and buffer completion.
pub fn is_keyword(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// `w` / `W`. `eol` (an operator is pending) stops the last word at the
/// end of its line instead of running on to the next line's first word.
pub(crate) fn forward(walk: &mut Walk, count: usize, big: bool, eol: bool) -> bool {
    for remaining in (0..count).rev() {
        let last = remaining == 0;
        let class = walk.class(big);
        let on_last_line = walk.on_last_line();
        let step = walk.inc();
        if step == Step::Edge || (step.left_text() && on_last_line) {
            return false;
        }
        if step.left_text() && eol && last {
            return true;
        }
        if class != BLANK {
            while walk.class(big) == class {
                let step = walk.inc();
                if step == Step::Edge || (step.left_text() && eol && last) {
                    return true;
                }
            }
        }
        while walk.class(big) == BLANK {
            if walk.on_empty_line() {
                break;
            }
            let step = walk.inc();
            if step == Step::Edge || (step.left_text() && (on_last_line || (eol && last))) {
                return true;
            }
        }
    }
    true
}

/// `b` / `B`. `stop` keeps the first step inside the word under the
/// cursor (the text object's use).
pub(crate) fn backward(walk: &mut Walk, count: usize, big: bool, mut stop: bool) -> bool {
    'count: for _ in 0..count {
        let class = walk.class(big);
        if walk.dec() == Step::Edge {
            return false;
        }
        if !stop || class == walk.class(big) || class == BLANK {
            while walk.class(big) == BLANK {
                if walk.on_empty_line() {
                    stop = false;
                    continue 'count;
                }
                if walk.dec() == Step::Edge {
                    return true;
                }
            }
            let class = walk.class(big);
            while walk.class(big) == class {
                if walk.dec() == Step::Edge {
                    return true;
                }
            }
        }
        walk.inc();
        stop = false;
    }
    true
}

/// `e` / `E`. `stop` keeps the first step inside the word under the
/// cursor (`cw`); `empty` lets an empty line end the walk (text objects).
pub(crate) fn end(walk: &mut Walk, count: usize, big: bool, mut stop: bool, empty: bool) -> bool {
    'count: for _ in 0..count {
        let class = walk.class(big);
        if walk.inc() == Step::Edge {
            return false;
        }
        if walk.class(big) == class && class != BLANK {
            while walk.class(big) == class {
                if walk.inc() == Step::Edge {
                    return false;
                }
            }
        } else if !stop || class == BLANK {
            while walk.class(big) == BLANK {
                if empty && walk.on_empty_line() {
                    stop = false;
                    continue 'count;
                }
                if walk.inc() == Step::Edge {
                    return false;
                }
            }
            let class = walk.class(big);
            while walk.class(big) == class {
                if walk.inc() == Step::Edge {
                    return false;
                }
            }
        }
        walk.dec();
        stop = false;
    }
    true
}

/// `ge` / `gE`. `eol` stops at a line's end (the text object's use).
pub(crate) fn end_backward(walk: &mut Walk, count: usize, big: bool, eol: bool) -> bool {
    for _ in 0..count {
        let class = walk.class(big);
        let step = walk.dec();
        if step == Step::Edge {
            return false;
        }
        if eol && step == Step::Line {
            return true;
        }
        if class != BLANK {
            while walk.class(big) == class {
                let step = walk.dec();
                if step == Step::Edge || (eol && step == Step::Line) {
                    return true;
                }
            }
        }
        while walk.class(big) == BLANK {
            if walk.on_empty_line() {
                break;
            }
            let step = walk.dec();
            if step == Step::Edge || (eol && step == Step::Line) {
                return true;
            }
        }
    }
    true
}

/// Back to the start of the word or blank run under the cursor, within
/// the line (Vim's `back_in_line`).
fn back_in_line(walk: &mut Walk, big: bool) {
    let class = walk.class(big);
    while walk.col() > 0 {
        walk.dec();
        if walk.class(big) != class {
            walk.inc();
            break;
        }
    }
}

/// One column left; `false` at column 0 (Vim's `oneleft`).
fn one_left(walk: &mut Walk) -> bool {
    if walk.col() == 0 {
        return false;
    }
    walk.dec();
    true
}

/// A selected word object: `[start, end]` with `inclusive` telling
/// whether `end` itself belongs to it.
pub(crate) struct WordObject {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) inclusive: bool,
}

/// `iw` / `aw` / `iW` / `aW` with a count (Vim's `current_word`).
/// `selection` is a Visual selection of more than one character, as
/// `(anchor, head)`: the object then extends it instead of starting over.
pub(crate) fn object(
    buf: &Buffer,
    cursor: usize,
    selection: Option<(usize, usize)>,
    mut count: usize,
    around: bool,
    big: bool,
) -> Option<WordObject> {
    let mut walk = Walk::new(buf, cursor);
    let mut start = selection.map_or(cursor, |(anchor, _)| anchor);
    let mut around_white = false;
    let mut inclusive = true;
    let backward = selection.is_some_and(|(anchor, head)| head < anchor);
    if selection.is_none() {
        back_in_line(&mut walk, big);
        start = walk.pos;
        if (walk.class(big) == BLANK) == around {
            if !end(&mut walk, 1, big, true, true) {
                return None;
            }
        } else {
            forward(&mut walk, 1, big, true);
            if walk.col() == 0 {
                walk.decl();
            } else {
                one_left(&mut walk);
            }
            around_white = around;
        }
        count = count.saturating_sub(1);
    }
    while count > 0 {
        inclusive = true;
        if backward {
            if walk.decl() == Step::Edge {
                return None;
            }
            if around != (walk.class(big) != BLANK) {
                if !self::backward(&mut walk, 1, big, true) {
                    return None;
                }
            } else {
                if !end_backward(&mut walk, 1, big, true) {
                    return None;
                }
                walk.incl();
            }
        } else {
            if walk.incl() == Step::Edge {
                return None;
            }
            if around != (walk.class(big) == BLANK) {
                if !forward(&mut walk, 1, big, true) && count > 1 {
                    return None;
                }
                if !one_left(&mut walk) {
                    inclusive = false;
                }
            } else if !end(&mut walk, 1, big, true, true) {
                return None;
            }
        }
        count -= 1;
    }
    if around_white && (walk.class(big) != BLANK || (walk.col() == 0 && !inclusive)) {
        // No blanks after the word: take the ones before it instead,
        // unless they are the line's indent.
        let mut lead = Walk::new(buf, start);
        if one_left(&mut lead) {
            back_in_line(&mut lead, big);
            if lead.class(big) == BLANK && lead.col() > 0 {
                start = lead.pos;
            }
        }
    }
    Some(WordObject {
        start,
        end: walk.pos,
        inclusive,
    })
}

/// The occurrence-selection seed (0049 §7): the word or punctuation
/// run under `pos` by Vim's small-word classes — `gb` selects exactly
/// what `iw` classifies, and a caret on whitespace seeds nothing.
pub fn word_run(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
    if pos >= buf.len_bytes() {
        return None;
    }
    let mut walk = Walk::new(buf, pos);
    let class = walk.class(false);
    if class == BLANK {
        return None;
    }
    back_in_line(&mut walk, false);
    let start = walk.pos;
    while walk.class(false) == class {
        if walk.inc() != Step::Char {
            break;
        }
    }
    Some((start, walk.pos))
}
