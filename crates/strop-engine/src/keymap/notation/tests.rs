use super::TokenKind::{Chord, Command, Key, Named, Placeholder, Separator, Space, Tail};
use super::*;

fn joined<'a>(tokens: impl Iterator<Item = Token<'a>>) -> String {
    tokens.map(|token| token.text).collect()
}

fn lexed<'a>(tokens: impl Iterator<Item = Token<'a>>) -> Vec<(TokenKind, &'a str)> {
    tokens.map(|token| (token.kind, token.text)).collect()
}

/// Every cell the help prints, lexed, gives back its exact bytes and
/// never an empty token.
#[test]
fn the_lexers_are_lossless() {
    for binding in crate::keymap::BINDINGS {
        assert_eq!(joined(keys(binding.keys)), binding.keys);
        assert!(keys(binding.keys).all(|token| !token.text.is_empty()));
    }
    for family in crate::editor::ex::Family::ALL {
        for (usage, _) in crate::editor::ex::help_rows(family) {
            assert_eq!(joined(command(usage)), usage);
            assert!(command(usage).all(|token| !token.text.is_empty()));
        }
    }
    for field in strop_picker::query::schema::Field::ALL {
        let example = field.example();
        assert_eq!(joined(filter(example)), example);
        assert!(filter(example).all(|token| !token.text.is_empty()));
    }
}

#[test]
fn key_cells_lex_by_what_each_word_is() {
    let cases: &[(&str, &[(TokenKind, &str)])] = &[
        (
            "h j k l",
            &[
                (Key, "h"),
                (Space, " "),
                (Key, "j"),
                (Space, " "),
                (Key, "k"),
                (Space, " "),
                (Key, "l"),
            ],
        ),
        (
            "space w v / -",
            &[
                (Named, "space"),
                (Space, " "),
                (Key, "w"),
                (Space, " "),
                (Key, "v"),
                (Space, " "),
                (Separator, "/"),
                (Space, " "),
                (Key, "-"),
            ],
        ),
        ("/ ?", &[(Key, "/"), (Space, " "), (Key, "?")]),
        ("space /", &[(Named, "space"), (Space, " "), (Key, "/")]),
        (
            "ctrl-\\ ctrl-n",
            &[(Chord, "ctrl-\\"), (Space, " "), (Chord, "ctrl-n")],
        ),
        (
            "f<c> F<c>",
            &[
                (Key, "f"),
                (Placeholder, "<c>"),
                (Space, " "),
                (Key, "F"),
                (Placeholder, "<c>"),
            ],
        ),
        ("'<a>", &[(Key, "'"), (Placeholder, "<a>")]),
        (
            "d y c > < gU",
            &[
                (Key, "d"),
                (Space, " "),
                (Key, "y"),
                (Space, " "),
                (Key, "c"),
                (Space, " "),
                (Key, ">"),
                (Space, " "),
                (Key, "<"),
                (Space, " "),
                (Key, "gU"),
            ],
        ),
        (
            "ge gE { }",
            &[
                (Key, "ge"),
                (Space, " "),
                (Key, "gE"),
                (Space, " "),
                (Key, "{"),
                (Space, " "),
                (Key, "}"),
            ],
        ),
        (
            "ctrl-r {reg}",
            &[(Chord, "ctrl-r"), (Space, " "), (Placeholder, "{reg}")],
        ),
        (
            "up down left right tab s-tab",
            &[
                (Named, "up"),
                (Space, " "),
                (Named, "down"),
                (Space, " "),
                (Named, "left"),
                (Space, " "),
                (Named, "right"),
                (Space, " "),
                (Named, "tab"),
                (Space, " "),
                (Chord, "s-tab"),
            ],
        ),
        (
            ":terminal [cmd] :term",
            &[
                (Command, ":terminal"),
                (Space, " "),
                (Separator, "["),
                (Placeholder, "cmd"),
                (Separator, "]"),
                (Space, " "),
                (Command, ":term"),
            ],
        ),
        (
            "space o / :remote",
            &[
                (Named, "space"),
                (Space, " "),
                (Key, "o"),
                (Space, " "),
                (Separator, "/"),
                (Space, " "),
                (Command, ":remote"),
            ],
        ),
        (
            "iw i\" i{",
            &[
                (Key, "iw"),
                (Space, " "),
                (Key, "i\""),
                (Space, " "),
                (Key, "i{"),
            ],
        ),
    ];
    for (cell, expected) in cases {
        assert_eq!(lexed(keys(cell)), *expected, "{cell}");
    }
}

#[test]
fn usages_lex_names_tails_and_placeholders() {
    let cases: &[(&str, &[(TokenKind, &str)])] = &[
        (
            ":bd[elete][!] [number]",
            &[
                (Command, ":bd"),
                (Separator, "["),
                (Tail, "elete"),
                (Separator, "]"),
                (Separator, "["),
                (Tail, "!"),
                (Separator, "]"),
                (Space, " "),
                (Separator, "["),
                (Placeholder, "number"),
                (Separator, "]"),
            ],
        ),
        (
            ":s/pat/new/[flags]",
            &[
                (Command, ":s"),
                (Separator, "/"),
                (Placeholder, "pat"),
                (Separator, "/"),
                (Placeholder, "new"),
                (Separator, "/"),
                (Separator, "["),
                (Placeholder, "flags"),
                (Separator, "]"),
            ],
        ),
        (
            ":sor[t][!] [u][n][i]",
            &[
                (Command, ":sor"),
                (Separator, "["),
                (Tail, "t"),
                (Separator, "]"),
                (Separator, "["),
                (Tail, "!"),
                (Separator, "]"),
                (Space, " "),
                (Separator, "["),
                (Placeholder, "u"),
                (Separator, "]"),
                (Separator, "["),
                (Placeholder, "n"),
                (Separator, "]"),
                (Separator, "["),
                (Placeholder, "i"),
                (Separator, "]"),
            ],
        ),
        (
            ":[range]!cmd",
            &[
                (Command, ":"),
                (Separator, "["),
                (Placeholder, "range"),
                (Separator, "]"),
                (Placeholder, "!cmd"),
            ],
        ),
        (
            ":tab-size [N|auto]",
            &[
                (Command, ":tab-size"),
                (Space, " "),
                (Separator, "["),
                (Placeholder, "N"),
                (Separator, "|"),
                (Placeholder, "auto"),
                (Separator, "]"),
            ],
        ),
        (":w :e!", &[(Command, ":w"), (Space, " "), (Command, ":e!")]),
    ];
    for (cell, expected) in cases {
        assert_eq!(lexed(command(cell)), *expected, "{cell}");
    }
}

#[test]
fn filter_cells_split_key_and_value() {
    assert_eq!(
        lexed(filter("language:rust")),
        [(Command, "language:"), (Placeholder, "rust")]
    );
    assert_eq!(
        lexed(filter("-language: -kind: -path:")),
        [
            (Command, "-language:"),
            (Space, " "),
            (Command, "-kind:"),
            (Space, " "),
            (Command, "-path:"),
        ]
    );
    assert_eq!(lexed(filter("NOT")), [(Key, "NOT")]);
}

/// What the help inks as a separator is exactly what dispatch reads as
/// an alternative: every `/` word that is not a separator is a key of
/// one of the row's expanded sequences.
#[test]
fn separators_agree_with_dispatch() {
    for binding in crate::keymap::BINDINGS {
        let words = binding.keys.split(' ').filter(|word| *word == "/").count();
        // a `/` inside a word (`:/pat/`) is a usage's, not an alternative
        let tokens: Vec<Token<'_>> = keys(binding.keys).collect();
        let alone = |at: usize| tokens.get(at).is_none_or(|token| token.kind == Space);
        let separators = (0..tokens.len())
            .filter(|&at| {
                tokens[at].kind == Separator
                    && tokens[at].text == "/"
                    && (at == 0 || alone(at - 1))
                    && alone(at + 1)
            })
            .count();
        let typed: usize = crate::keymap::lookup::expand(binding.keys)
            .iter()
            .map(|sequence| sequence.iter().filter(|key| **key == "/").count())
            .sum();
        assert_eq!(words, separators + typed, "{}", binding.keys);
    }
}
