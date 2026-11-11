//! The generated corpus (0095 §3): each operator × each motion or text
//! object × {no count, count} × {Normal, Visual, dot-repeat}, from fixed
//! starting positions in two fixtures. A case's name is its own
//! reproduction: `gen/prose@2:7/normal/d2aw`.

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

pub fn cases() -> Vec<Run> {
    let mut runs = Vec::new();
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
