//! The generated corpus (0095 §3): each operator × each motion or text
//! object × {no count, count} × {Normal, Visual, dot-repeat}, from fixed
//! starting positions in two fixtures; `d` over every target also runs on
//! multibyte, CRLF and unterminated text (plan 0108 D2). A case's name
//! is its own reproduction: `gen/prose@2:7/normal/d2aw`.

use super::Run;

/// Prose: sentences, paragraphs, an indented block, trailing blanks,
/// punctuation runs, a last line with no blank line after it.
const PROSE: &str = concat!(
    "The quick brown fox. It jumps over the lazy dog!  Then it rests.\n",
    "A second line (with some parens) and \"double quotes\" here.\n",
    "\n",
    "    indented paragraph, line one\n",
    "    indented line two with trailing blanks  \n",
    "\n",
    "\n",
    "last paragraph: x = y + 42; foo_bar(baz, qux)\n",
    "final line\n",
);

/// Code: nesting, brackets of every kind, quotes, a tag pair, numbers.
const CODE: &str = concat!(
    "fn main() {\n",
    "    let v = vec![1, 2, 3];\n",
    "    if v.len() > 2 {\n",
    "        call(a, \"b c\", 'd');\n",
    "    }\n",
    "    <div class=\"x\"><b>bold</b> text</div>\n",
    "}\n",
);

/// Two-, three- and four-byte characters, a combining mark, wide
/// characters inside brackets and quotes.
const MULTIBYTE: &str = concat!(
    "plain words café über 日本語 text, déjà vu.\n",
    "a plain naïve e\u{301}cole (日本) \"引用 é\" end\n",
    "\n",
    "  🎉 party 🎉 time\n",
    "last 日 line\n",
);

/// Lines that end in CRLF: the `\r` is never text a motion lands on.
const CRLF: &str = concat!(
    "first line (with parens) here\r\n",
    "second line, \"quoted\" words\r\n",
    "\r\n",
    "    indented line\r\n",
    "last line\r\n",
);

/// A last line with no line break after it.
const UNTERMINATED: &str = "alpha beta (gamma)\n\n  delta, epsilon\nlast line no break";

/// A fixture and the positions cases start from (1-based line, column).
struct Fixture {
    name: &'static str,
    text: &'static str,
    positions: &'static [(usize, usize)],
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "prose",
        text: PROSE,
        positions: &[(1, 1), (1, 12), (2, 16), (3, 1), (5, 9), (8, 32)],
    },
    Fixture {
        name: "code",
        text: CODE,
        positions: &[(2, 5), (2, 17), (4, 14), (4, 19), (6, 22), (7, 1)],
    },
    Fixture {
        name: "multibyte",
        text: MULTIBYTE,
        positions: &[(1, 7), (2, 3), (4, 3)],
    },
    Fixture {
        name: "crlf",
        text: CRLF,
        positions: &[(1, 13), (5, 1)],
    },
    Fixture {
        name: "unterminated",
        text: UNTERMINATED,
        positions: &[(3, 4), (4, 6)],
    },
];

/// A motion or text object, by its keys.
struct Target {
    keys: &'static str,
}

const fn motion(keys: &'static str) -> Target {
    Target { keys }
}
const fn object(keys: &'static str) -> Target {
    Target { keys }
}

/// Every target of plan 0095 §2.1 that needs no second buffer. The mark
/// `a` is set on line 2, column 9 before each case; searches use words
/// that occur in one case only, so no case depends on case folding.
const TARGETS: &[Target] = &[
    motion("h"),
    motion("l"),
    motion("j"),
    motion("k"),
    motion("w"),
    motion("b"),
    motion("e"),
    motion("W"),
    motion("B"),
    motion("E"),
    motion("ge"),
    motion("gE"),
    motion("0"),
    motion("^"),
    motion("$"),
    motion("_"),
    motion("g_"),
    motion("-"),
    motion("+"),
    motion("gg"),
    motion("G"),
    motion("{"),
    motion("}"),
    motion("("),
    motion(")"),
    motion("%"),
    motion("fe"),
    motion("Fe"),
    motion("te"),
    motion("Te"),
    motion("[("),
    motion("[{"),
    motion("])"),
    motion("]}"),
    motion("gj"),
    motion("gk"),
    motion("'a"),
    motion("`a"),
    motion("/in<cr>"),
    motion("?in<cr>"),
    motion("H"),
    motion("L"),
    object("iw"),
    object("aw"),
    object("iW"),
    object("aW"),
    object("ip"),
    object("ap"),
    object("i("),
    object("a("),
    object("ib"),
    object("i["),
    object("a["),
    object("i{"),
    object("a{"),
    object("iB"),
    object("i<lt>"),
    object("a<lt>"),
    object("i\""),
    object("a\""),
    object("i'"),
    object("a'"),
    object("it"),
    object("at"),
];

/// (operator keys in Normal, keys in Visual, what follows it).
const OPERATORS: &[(&str, &str, &str)] = &[
    ("d", "d", ""),
    ("c", "c", "X<esc>"),
    ("y", "y", ""),
    (">", ">", ""),
    ("<lt>", "<lt>", ""),
    ("gU", "U", ""),
    ("gu", "u", ""),
    ("g~", "~", ""),
    ("g?", "g?", ""),
];

/// `|` is the column motion, which takes its count; `%` with a count is
/// another command; a search or a mark takes none that changes it.
fn countable(target: &Target) -> bool {
    !matches!(target.keys, "0" | "^" | "%" | "'a" | "`a" | "H" | "L")
}

fn name_of(keys: &str) -> String {
    keys.replace("<lt>", "<").replace("<cr>", "⏎")
}

/// Fixed display-column witnesses. Cursor columns in the names below are
/// Neovim's *one-based byte columns*, not counts of characters or cells.
fn display_cases() -> Vec<Run> {
    let ragged = "abc\na\nab\n";
    let interior = "abc\nabcdef\na\nuntouched\n";
    let cases = [
        ("column/ascii-byte3", "abcz\n", "3|"),
        ("column/multibyte-byte3", "éab\n", "2|"),
        ("column/combining-byte4", "e\u{301}ab\n", "2|"),
        ("column/wide-inside-byte2", "a界b\n", "3|"),
        ("column/wide-after-byte5", "a界b\n", "4|"),
        ("column/tab-inside-byte2", "a\tbc\n", "3|"),
        ("column/tab-after-byte3", "a\tbc\n", "5|"),
        ("column/short-byte2", "aé\n", "99|"),
        ("column/empty-byte1", "\nabc\n", "99|"),
        ("column/short-then-j-byte6", "x\n0123456789\n", "6|j"),
        ("column/tab-then-j-byte3", "a\tbc\n0123456789\n", "3|j"),
        ("column/wide-then-gj-byte3", "a界b\n0123456789\n", "3|gj"),
        ("column/operator-forward", "éab\n", "d2|"),
        ("column/operator-backward", "éab\n", "3|d1|"),
        ("column/visual-inclusive", "éab\n", "v2|d"),
        ("column/visual-past-end", "aé\nb\n", "v99|y"),
        ("column/operator-gj", "0123456789\na界b\n", "4|dgj"),
        ("column/operator-gk", "a界b\n0123456789\n", "2G4|dgk"),
        ("block/eol-delete", ragged, "<c-v>$jd"),
        ("block/eol-delete-put", ragged, "<c-v>$jdp"),
        ("block/eol-yank-put", ragged, "<c-v>$jyp"),
        ("block/eol-change-put", ragged, "<c-v>$jcX<esc>p"),
        (
            "block/eol-change-through-empty",
            "abc\n\nab\n",
            "<c-v>$jcX<esc>",
        ),
        (
            "block/eol-long-interior-counted-put",
            interior,
            "<c-v>$2jy2p",
        ),
        ("block/eol-up", interior, "3G<c-v>$2kd"),
        (
            "block/eol-nonzero-left",
            "abcdef\nx\nabcdefghijk\nab\n",
            "4|<c-v>$3jcX<esc>",
        ),
        (
            "block/eol-clipped-tab-wide-middle",
            "aZq\n\t界xy\nab\n",
            "3|<c-v>$2jy",
        ),
        ("block/horizontal-clears-eol", ragged, "<c-v>$jhkd"),
    ];
    cases
        .into_iter()
        .map(|(name, text, keys)| Run {
            name: format!("display/{name}"),
            text: text.to_owned(),
            keys: keys.to_owned(),
        })
        .collect()
}

pub fn cases() -> Vec<Run> {
    let mut runs = display_cases();
    for (fixture_index, fixture) in FIXTURES.iter().enumerate() {
        let (text, positions, fixture) = (fixture.text, fixture.positions, fixture.name);
        for (index, (line, column)) in positions.iter().enumerate() {
            let place = format!("2G9|ma{line}G{column}|");
            let mut push = |mode: &str, label: String, keys: String| {
                runs.push(Run {
                    name: format!("gen/{fixture}@{line}:{column}/{mode}/{label}"),
                    text: text.to_owned(),
                    keys: format!("{place}{keys}"),
                });
            };
            for (op_index, (op, visual_op, after)) in OPERATORS.iter().enumerate() {
                // `d` runs from every position (with counts from every
                // other one), `c` and `y` from one per fixture, the rest
                // from one position in one fixture: they share the
                // resolver, so more positions would test `d` again.
                let one_fixture = op_index % 2 == fixture_index;
                let (normal, counted) = match op_index {
                    0 => (true, index % 2 == 0),
                    1 | 2 => (index == op_index, true),
                    _ => (one_fixture && index == op_index % 6, false),
                };
                let visual = match op_index {
                    0 => index % 3 == 0,
                    _ => one_fixture && index == (op_index + 3) % 6,
                };
                let dot = *op != "y" && one_fixture && index == (op_index + 1) % 6;
                for target in TARGETS {
                    let keys = target.keys;
                    if *op == "g?" && keys.starts_with('?') {
                        // `g??` is the doubled operator, not a search
                        continue;
                    }
                    for count in ["", "2"] {
                        if !count.is_empty() && !countable(target) {
                            continue;
                        }
                        let label = format!("{}{count}{}", name_of(op), name_of(keys));
                        if normal && (counted || count.is_empty()) {
                            push("normal", label, format!("{op}{count}{keys}{after}"));
                        }
                        if visual && (op_index == 0 || count.is_empty()) {
                            push(
                                "visual",
                                format!("v{count}{}{}", name_of(keys), name_of(visual_op)),
                                format!("v{count}{keys}{visual_op}{after}"),
                            );
                        }
                    }
                    // (a mark the first command deleted or joined away is
                    // not the same mark the second time)
                    if dot && !matches!(keys, "'a" | "`a") {
                        let label = format!("{}{}", name_of(op), name_of(keys));
                        push("dot", label, format!("{op}{keys}{after}j."));
                    }
                }
            }
        }
    }
    runs
}
