//! Seeded differential properties over random edits (0111 decision 10).
//! Hermetic (a fixed xorshift, no dependency) and bounded: a fixed number
//! of seeds and steps per language keeps `cargo test` fast. On a failure
//! the message carries the language, seed, text and edit log, so the case
//! replays exactly.
//!
//! - incremental: the long-lived highlighter, fed the buffer's journal,
//!   agrees with a fresh one on the same rope whenever the fresh layers
//!   parse without error (tree-sitter's error recovery may settle
//!   differently after an incremental reparse — a property of the parser,
//!   not of the journal);
//! - window: a fresh windowed highlight agrees with a fresh full-document
//!   highlight clipped to the window — a byte's colour does not depend on
//!   where the window starts (catches 0111 B3 and L1);
//! - repeat: asking twice returns identical spans.
//!
//! The alphabets leave out the non-`\n` line breaks ropey counts (VT, FF,
//! NEL, LS, PS, lone CR): journal points use ropey's line model while
//! tree-sitter counts `\n` (0111 §A L2), and that model is decided apart
//! from this plan.
use crate::{Class, Emphasis, Highlighter, Span};
use ropey::Rope;
use std::path::Path;
use strop_core::{id::BufferRevision, Buffer, Range};

const SEEDS: u64 = 24;
const STEPS: usize = 15;

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

struct Case {
    path: &'static str,
    seed: &'static str,
    tokens: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case {
        path: "a.rs",
        seed: "/// Doc *em* `code`\n/// ```rust\n/// let x = 1;\n/// ```\n/// ```python\n/// def f(): pass\n/// ```\nfn main() {\n    let s = \"hi\"; // c\n    /** block\n     * doc `x`\n     */\n    struct A;\n}\n//! inner\n",
        tokens: &[
            "/// ", "```", "```rust\n", "\n", "\"", "/*", "*/", "fn f() {}", " ", "//! ", "é",
            "let x = 1;", "{", "}", "/** ", "\r\n",
        ],
    },
    Case {
        path: "a.md",
        seed: "# Title\n\nSome *text* and `code` <b>bold</b>.\n\n```rust\nfn main() { let x = \"s\"; }\n```\n\n<div>\n\npara\n\n</div>\n\n```python\ndef f():\n    return 1\n```\n\n| a | b |\n|---|---|\n| `x` | *y* |\n\n````markdown\n```js\nlet a = 1;\n```\n````\n",
        tokens: &[
            "```", "```rust\n", "\n", "\n\n", "*", "`", "<div>", "</div>", "#", "~~~", "````",
            "é", " ", "js", "python", "| ", "\r\n", "#!/bin/bash\n",
        ],
    },
    Case {
        path: "a.html",
        seed: "<html><head><style>a { color: red; }</style><script>let x = 1; function f() {}</script></head><body><p>hi</p></body></html>\n",
        tokens: &[
            "<script>", "</script>", "<style>", "</style>", "\n", "let y = \"s\";", "<p>", "\"",
            " ", "é",
        ],
    },
    Case {
        path: "a.php",
        seed: "<html>\n<?php echo 1; // c\n$q = mysqli_query($c, \"select * from t\"); ?>\n<div><?php if ($x) { ?>\n<b>t</b>\n<?php } ?></div>\n</html>\n",
        tokens: &[
            "<?php ", " ?>", "\n", "<div>", "</div>", "\"", "echo 2;", " ", "/* ", " */",
        ],
    },
    Case {
        path: "a.py",
        seed: "def f():\n    \"\"\"Doc *x*.\n\n    ```python\n    y = 1\n    ```\n    \"\"\"\n    return 1\n",
        tokens: &[
            "\"\"\"", "\n", "    ", "```", "def g(): pass\n", "'", "é", " ",
        ],
    },
];

/// Spans clipped to `[first, last)`, adjacent equal styles merged: two
/// runs that paint the same cells compare equal however they were cut.
fn normal(spans: &[Span], first: usize, last: usize) -> Vec<(usize, usize, Class, Emphasis)> {
    let mut out: Vec<(usize, usize, Class, Emphasis)> = Vec::new();
    for span in spans {
        let (start, end) = (span.start.max(first), span.end.min(last));
        if start >= end {
            continue;
        }
        if let Some(previous) = out.last_mut().filter(|previous| {
            previous.1 == start && previous.2 == span.class && previous.3 == span.emphasis
        }) {
            previous.1 = end;
            continue;
        }
        out.push((start, end, span.class, span.emphasis));
    }
    out
}

/// A fresh highlight, and whether any layer it built holds a parse error.
fn fresh(path: &Path, rope: &Rope, first: usize, last: usize) -> (Vec<Span>, bool) {
    let mut hl = Highlighter::for_path(path, rope).expect("language");
    let spans = hl
        .highlight(rope, BufferRevision::new(0), first, last)
        .expect("fresh highlight");
    (spans, hl.has_parse_error())
}

/// One random replacement at char boundaries, from the case's alphabet.
fn edit(rng: &mut Rng, buf: &mut Buffer, tokens: &[&str], log: &mut Vec<String>) {
    let rope = buf.text();
    let len = rope.len_bytes();
    let start = rope.char_to_byte(rope.byte_to_char(rng.below(len + 1)));
    let end = if rng.below(3) == 0 {
        let end = (start + rng.below(12)).min(len);
        rope.char_to_byte(rope.byte_to_char(end))
    } else {
        start
    };
    let text = if rng.below(5) == 0 {
        ""
    } else {
        tokens[rng.below(tokens.len())]
    };
    if start == end && text.is_empty() {
        return;
    }
    log.push(format!("replace {start}..{end} with {text:?}"));
    buf.edit()
        .replace(Range::charwise(start, end), text)
        .expect("edit");
}

#[test]
fn incremental_window_and_repeat_properties_hold() {
    for case in CASES {
        let path = Path::new(case.path);
        let mut checked = 0;
        for seed in 1..=SEEDS {
            let mut rng = Rng::new(seed);
            let mut buf = Buffer::from_text(case.seed);
            let mut hl = Highlighter::for_path(path, buf.text()).expect("language");
            hl.highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
                .expect("warm");
            let mut log = Vec::new();
            for _ in 0..STEPS {
                for _ in 0..1 + rng.below(2) {
                    edit(&mut rng, &mut buf, case.tokens, &mut log);
                }
                for change in buf.changes() {
                    hl.apply_edits(std::slice::from_ref(&change.edit), change.revision);
                }
                buf.clear_changes();
                let rope = buf.text().clone();
                let len = rope.len_bytes();
                // line-granular windows, like the editor's viewport
                let lines = rope.len_lines();
                let top = rng.below(lines);
                let bottom = (top + 1 + rng.below(8)).min(lines);
                let (first, last) = if rng.below(3) == 0 {
                    (0, len)
                } else {
                    (rope.line_to_byte(top), rope.line_to_byte(bottom))
                };
                let context = || {
                    format!(
                        "{} seed {seed} window {first}..{last}\ntext {:?}\nlog {:?}",
                        case.path,
                        rope.to_string(),
                        log
                    )
                };
                let incremental = hl
                    .highlight(&rope, buf.revision(), first, last)
                    .unwrap_or_else(|error| panic!("{error}: {}", context()));
                let (windowed, erroneous) = fresh(path, &rope, first, last);
                // Error recovery may settle differently after an
                // incremental reparse; well-formed layers must agree.
                if !erroneous {
                    checked += 1;
                    assert_eq!(
                        normal(&incremental, first, last),
                        normal(&windowed, first, last),
                        "incremental != fresh: {}",
                        context()
                    );
                }
                let (full, _) = fresh(path, &rope, 0, len);
                assert_eq!(
                    normal(&windowed, first, last),
                    normal(&full, first, last),
                    "window != full, clipped: {}",
                    context()
                );
                let again = hl
                    .highlight(&rope, buf.revision(), first, last)
                    .unwrap_or_else(|error| panic!("{error}: {}", context()));
                assert_eq!(again, incremental, "repeat differs: {}", context());
            }
        }
        assert!(
            checked > 0,
            "{}: no well-formed step checked incrementally",
            case.path
        );
    }
}
