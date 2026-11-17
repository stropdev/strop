//! Random keys against the real editor (plan 0108 D2). A fixed-seed
//! generator types grammar-shaped tokens — counts, registers, operators,
//! motions, text objects, Insert with cursor keys, Visual modes, paste,
//! macros, a safe list of `:` commands — over text the differential
//! fixtures never hold: multibyte, combining, CRLF, no final newline,
//! empty. After every token every cursor and selection end sits on a
//! character boundary inside the buffer; at the end, undo to the root
//! gives back the original text and as many redos the final one. A
//! panic is a failure too; the report names the seed and the script.

use super::*;

/// The generator the picker's property suite uses: no dependency.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const FIXTURES: &[&str] = &[
    "",
    "étude",
    "one two\nfour\n",
    "aé 日本 🎉x\n  é\tz\n",
    "e\u{301}cole cafe\u{301}\nnai\u{308}ve\n",
    "one\r\ntwo é\r\n\r\nthree",
    "fn (a, [b]) {\n\t\"q\" 'r'\n}\nno newline 日",
    "a\rb\u{2028}c\u{85}d\n",
];

/// Typed text: ASCII, 2-, 3- and 4-byte characters, a combining mark.
const TEXT: &[&str] = &["x", "é", "日", "🎉", "\u{301}", " ", "(", "\"", "a b"];
const MOTIONS: &[&str] = &[
    "h", "j", "k", "l", "w", "b", "e", "W", "B", "E", "ge", "0", "^", "$", "gg", "G", "%", "{",
    "}", "(", ")", "fé", "t ", "Fa", "T日", ";", ",", "<left>", "<right>", "<up>", "<down>",
];
const OBJECTS: &[&str] = &[
    "iw", "aw", "iW", "is", "ip", "ap", "i(", "a(", "i[", "i{", "i\"", "a\"", "i'", "it",
];
const OPERATORS: &[&str] = &["d", "c", "y", ">", "<", "g~", "gu", "gU", "gJ"];
const EDITS: &[&str] = &[
    "x", "X", "s", "S", "C", "D", "Y", "J", "~", "p", "P", "]p", "[p", "gp", "u", "<c-r>", ".",
    "re", "r日", "<c-a>", "<c-x>", "dd", "yy", "cc", "yyp", "ddp", "xp", "gi",
];
const INSERT_ENTRY: &[&str] = &["i", "a", "A", "I", "o", "O", "R", "gI"];
const INSERT_KEYS: &[&str] = &[
    "<left>", "<right>", "<up>", "<down>", "<bs>", "<c-w>", "<c-u>", "<cr>", "<tab>", "<c-t>",
    "<c-d>", "<c-r>\"",
];
const EX: &[&str] = &[
    ":s/é/E/g<cr>",
    ":%s/o/0/<cr>",
    ":g/e/d<cr>",
    ":g/./norm Aé<cr>",
    ":v/x/s/$/;/<cr>",
    ":%norm I日<cr>",
    ":2,$j<cr>",
    ":m0<cr>",
    ":t$<cr>",
    ":sort<cr>",
    ":%s/\\(.\\)/\\1\\1/<cr>",
];
const PASTES: &[&str] = &["é", "日\n", "🎉x\r\n", "a\nb", "\t"];

/// One grammar-shaped token: keys for `feed_text`, or a bracketed paste.
enum Token {
    Keys(String),
    Paste(&'static str),
}

fn token(rng: &mut Lcg) -> Token {
    let count = |rng: &mut Lcg| match rng.below(4) {
        0 => format!("{}", 1 + rng.below(3)),
        _ => String::new(),
    };
    let keys = match rng.below(12) {
        0 | 1 => format!("{}{}", count(rng), rng.pick(MOTIONS)),
        2 | 3 => {
            let target = if rng.below(2) == 0 {
                rng.pick(MOTIONS)
            } else {
                rng.pick(OBJECTS)
            };
            // `"b`: register `a` holds only recorded keys, so `@a` never
            // runs deleted text as keys (a space there is the leader)
            let register = if rng.below(4) == 0 { "\"b" } else { "" };
            let operator = rng.pick(OPERATORS);
            let mut keys = format!("{register}{}{operator}{target}", count(rng));
            if operator == "c" {
                keys.push_str(rng.pick(TEXT));
                keys.push_str("<esc>");
            }
            keys
        }
        4 | 5 => format!("{}{}", count(rng), rng.pick(EDITS)),
        6 | 7 => {
            let mut keys = rng.pick(INSERT_ENTRY).to_owned();
            for _ in 0..1 + rng.below(5) {
                keys.push_str(if rng.below(2) == 0 {
                    rng.pick(TEXT)
                } else {
                    rng.pick(INSERT_KEYS)
                });
            }
            keys.push_str("<esc>");
            keys
        }
        8 => {
            let mode = rng.pick(&["v", "V", "<c-v>"]);
            let motion = rng.pick(MOTIONS);
            let action = rng.pick(&[
                "d",
                "y",
                "c日<esc>",
                ">",
                "~",
                "u",
                "U",
                "J",
                "p",
                "Ié<esc>",
                "Ax<esc>",
                "o",
                "r é",
            ]);
            format!("{mode}{}{motion}{action}<esc>", count(rng))
        }
        9 => rng.pick(EX).to_owned(),
        10 => match rng.below(3) {
            0 => format!("qa{}{}q", rng.pick(MOTIONS), rng.pick(EDITS)),
            1 => format!("{}@a", count(rng)),
            _ => format!("/{}<cr>n", rng.pick(TEXT)),
        },
        _ => return Token::Paste(rng.pick(PASTES)),
    };
    Token::Keys(keys)
}

/// One token, then the keys a structural resolution held behind it run
/// (the headless driver's barrier).
fn apply(editor: &mut Editor, token: &Token) {
    match token {
        Token::Keys(keys) => editor.feed_text(keys),
        Token::Paste(text) => editor.paste_bracketed(text),
    }
    editor.wait_resolution();
}

fn describe(script: &[Token]) -> String {
    script
        .iter()
        .map(|token| match token {
            Token::Keys(keys) => format!("keys {keys}"),
            Token::Paste(text) => format!("paste {text:?}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every cursor and selection end is a character boundary in the buffer.
fn on_boundaries(editor: &Editor) -> Result<(), String> {
    let buf = editor.buf();
    let primary = editor.sels().primary();
    for selection in std::iter::once(&primary).chain(editor.sels().extra_heads()) {
        for end in [selection.anchor, selection.head] {
            if end > buf.len_bytes() || !buf.is_boundary(end) {
                return Err(format!("selection end {end} is not a character boundary"));
            }
        }
    }
    Ok(())
}

/// Undo to the root, then redo back to where the script ended.
fn history_round_trip(editor: &mut Editor, original: &str) -> Result<(), String> {
    editor.feed_text("<esc><esc>");
    editor.wait_resolution();
    let last = editor.buf().text().to_string();
    let mut steps = 0;
    while editor.buf().history().can_undo() {
        editor.feed_text("u");
        editor.wait_resolution();
        steps += 1;
        if steps > 10_000 {
            return Err("undo never reached the root".into());
        }
    }
    if editor.buf().text() != original {
        return Err(format!(
            "undo to the root gave {:?}",
            editor.buf().text().to_string()
        ));
    }
    // as many redos as undos walk back the same path to where the
    // script ended (it may itself have ended below the tip)
    for _ in 0..steps {
        editor.feed_text("<c-r>");
        editor.wait_resolution();
    }
    if *editor.buf().text() != last {
        return Err(format!(
            "redo back gave {:?}, not {last:?}",
            editor.buf().text().to_string()
        ));
    }
    Ok(())
}

/// One script on one fixture: the first failed oracle, with its step.
fn run(fixture: &str, script: &[Token]) -> Result<(), String> {
    let mut editor = Editor::new(Buffer::from_text(fixture));
    let document = editor.current();
    for (step, token) in script.iter().enumerate() {
        let applied = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply(&mut editor, token);
            on_boundaries(&editor)
        }));
        match applied {
            Ok(Ok(())) if editor.current() == document => {}
            Ok(Ok(())) => return Err(format!("step {step}: left the document")),
            Ok(Err(error)) => return Err(format!("step {step}: {error}")),
            Err(_) => return Err(format!("step {step}: panicked")),
        }
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        history_round_trip(&mut editor, fixture)
    }))
    .unwrap_or_else(|_| Err("undo/redo panicked".into()))
}

/// Each fixture runs on its own thread with its own generator, so the
/// test costs about one fixture's time and stays deterministic.
#[test]
fn random_keys_keep_the_editor_sound() {
    const SEED: u64 = 0x0001_08d2;
    const SCRIPTS: usize = 40;
    const STEPS: usize = 30;
    let failures: Vec<String> = std::thread::scope(|scope| {
        let runs: Vec<_> = FIXTURES
            .iter()
            .enumerate()
            .map(|(index, fixture)| {
                scope.spawn(move || {
                    let seed = SEED + index as u64;
                    let mut rng = Lcg(seed);
                    let mut failures = Vec::new();
                    for script in 0..SCRIPTS {
                        let tokens: Vec<Token> = (0..STEPS).map(|_| token(&mut rng)).collect();
                        if let Err(error) = run(fixture, &tokens) {
                            failures.push(format!(
                                "seed {seed:#x}, fixture {fixture:?}, script {script}: {error}\n{}",
                                describe(&tokens)
                            ));
                        }
                    }
                    failures
                })
            })
            .collect();
        runs.into_iter()
            .flat_map(|run| {
                run.join()
                    .unwrap_or_else(|_| vec!["a fixture's thread panicked".into()])
            })
            .collect()
    });
    assert!(
        failures.is_empty(),
        "{} failing scripts; the first:\n{}",
        failures.len(),
        failures[0]
    );
}
