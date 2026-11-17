//! The command line's own keys (plan 0110 §3.2): editing, registers,
//! cancelling. Each case ends with the line run or closed, so the final
//! state is a buffer state Neovim can be asked for.

use super::Case;

/// Five lines for the line commands.
const FIVE: &str = "one\ntwo\n  three\nfour\nfive\n";
/// A number to yank and words to search for.
const WORDS: &str = "3 alpha beta-gamma\ntwo\nthree\nfour\nfive\n";

#[rustfmt::skip]
pub const CASES: &[Case] = &[
    // running
    ("enter runs", FIVE, ":2d<cr>"),
    ("ctrl-j runs", FIVE, ":2d<c-j>"),
    ("ctrl-m runs", FIVE, ":2d<c-m>"),
    // cancelling
    ("esc cancels at once", FIVE, ":2d<esc>"),
    ("esc then a normal key is a normal key", FIVE, ":2d<esc>x"),
    ("esc cancels a range from visual", FIVE, "jVj:<esc>x"),
    ("ctrl-c cancels", FIVE, "j:2d<c-c>"),
    ("backspace on an empty line cancels", FIVE, ":<bs>dd"),
    ("backspace back to empty cancels", FIVE, ":3<bs><bs>dd"),
    ("ctrl-h is backspace", FIVE, ":3x<c-h>d<cr>"),
    ("backspace edits", FIVE, ":2dx<bs><cr>"),
    ("search esc returns", FIVE, "/four<esc>x"),
    ("search ctrl-c returns", FIVE, "j/four<c-c>"),
    ("search backspace to empty cancels", FIVE, "/f<bs><bs>x"),
    // deleting
    ("ctrl-w deletes a word", FIVE, ":2d x<c-w><cr>"),
    ("ctrl-w deletes punctuation as a word", FIVE, ":2,3d<c-w><c-w><c-w><cr>"),
    ("ctrl-w skips blanks first", FIVE, ":3d  yy  <c-w><cr>"),
    ("ctrl-w twice", FIVE, ":3d a b<c-w><c-w><cr>"),
    ("ctrl-w at the start does nothing", FIVE, ":<c-w>2d<cr>"),
    ("ctrl-u deletes the line", FIVE, ":abc<c-u>2d<cr>"),
    ("ctrl-u keeps what follows", FIVE, ":junkd<left><c-u>4<cr>"),
    ("delete under the cursor", FIVE, ":2dd<left><delete><cr>"),
    ("delete at the end deletes before", FIVE, ":2dx<delete><cr>"),
    ("delete at the start", FIVE, ":x3d<home><delete><cr>"),
    // moving
    ("left then insert", FIVE, ":d<left>2<cr>"),
    ("left left right", FIVE, ":4d<left><left><right>,5<cr>"),
    ("ctrl-b then insert", FIVE, ":d<c-b>3<cr>"),
    ("home then insert", FIVE, ":d<home>3<cr>"),
    ("ctrl-e after ctrl-b", FIVE, ":2<c-b><c-e>d<cr>"),
    ("end after home", FIVE, ":2<home><end>,3d<cr>"),
    ("home and end with a count", FIVE, ":d<home>2<end> 2<cr>"),
    ("left past the start stays", FIVE, ":d<left><left><left>2<cr>"),
    ("insert in the middle of a range", FIVE, ":2,d<left>4<cr>"),
    // registers
    ("ctrl-r unnamed", WORDS, "yiw:<c-r>\"d<cr>"),
    ("ctrl-r named", WORDS, "\"ayiw:<c-r>ad<cr>"),
    ("ctrl-r zero", WORDS, "yiwjdd:<c-r>0d<cr>"),
    ("ctrl-r ctrl-w", WORDS, ":<c-r><c-w>d<cr>"),
    ("ctrl-r ctrl-w a later word", WORDS, "w:s/<c-r><c-w>/X/<cr>"),
    ("ctrl-r ctrl-a", WORDS, "ww:s/<c-r><c-a>/X/<cr>"),
    ("ctrl-r in search", WORDS, "jyiw/<c-r>\"<cr>x"),
    ("ctrl-r into the middle", WORDS, "yiw:d<left><c-r>\"<cr>"),
    ("ctrl-r then backspace", WORDS, "yiw:<c-r>\"4<bs>d<cr>"),
    // history
    ("up recalls the last line", FIVE, ":2d<cr>:<up><cr>"),
    ("up filters by the typed prefix", FIVE, ":1d<cr>:2y<cr>:1<up><cr>"),
    ("up twice", FIVE, ":1d<cr>:2d<cr>:<up><up><cr>"),
    ("up then down restores the typed text", FIVE, ":3d<cr>:2<up><down>d<cr>"),
    ("ctrl-p is unfiltered", FIVE, ":d<cr>:<c-p><cr>"),
    ("ctrl-p then ctrl-n", FIVE, ":1d<cr>:2d<cr>:<c-p><c-p><c-n><cr>"),
    ("search up", FIVE, "/four<cr>gg/<up><cr>x"),
    ("search history is its own", FIVE, "/two<cr>:3d<cr>/<up><cr>x"),
    ("a failed line is in history", FIVE, ":frob<cr>:<up><bs><bs><bs><bs>2d<cr>"),
    ("up after editing filters again", FIVE, ":1d<cr>:2y<cr>:<up><bs>d<up><cr>"),
    // edits after edits
    ("type delete retype", FIVE, ":3d<c-u>4d<cr>"),
    ("search edited mid-line", FIVE, "/fxur<home><right><delete>o<cr>x"),
];
