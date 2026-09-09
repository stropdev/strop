//! Contract tests: the grammar's observable behavior, pinned.

pub mod contract {
    use crate::*;
    use strop_core::Buffer;

    const SRC: &str = "fn f(xs: &[Item]) -> Edge {\n    let edge = hone(xs);\n}\n";

    pub fn cmd(keys: &str) -> Command {
        match parse(keys) {
            Parse::Complete(c) => c,
            other => panic!("{keys} parsed as {other:?}"),
        }
    }

    /// resolve, unwrapping both layers (Err = query failure, None =
    /// nothing resolves — both are bugs in these pins).
    fn r(buf: &Buffer, cursor: usize, keys: &str) -> Resolved {
        resolve(buf, cursor, &cmd(keys))
            .expect("query runs")
            .expect("resolvable")
    }

    fn resolve_str(buf: &Buffer, cursor: usize, keys: &str) -> String {
        buf.slice_string(r(buf, cursor, keys).range)
    }

    #[test]
    fn bracket_object_from_inside() {
        let buf = Buffer::from_text(SRC);
        assert_eq!(resolve_str(&buf, SRC.find("Item").unwrap(), "di["), "Item");
    }

    #[test]
    fn bracket_object_cursor_on_open() {
        let buf = Buffer::from_text(SRC);
        assert_eq!(resolve_str(&buf, SRC.find('[').unwrap(), "di["), "Item");
    }

    #[test]
    fn bracket_object_cursor_on_close() {
        let buf = Buffer::from_text(SRC);
        let close = SRC.find(']').unwrap();
        assert_eq!(resolve_str(&buf, close, "di["), "Item");
    }

    #[test]
    fn bracket_object_around_includes_delimiters() {
        let buf = Buffer::from_text(SRC);
        let inside = SRC.find("Item").unwrap();
        assert_eq!(resolve_str(&buf, inside, "da["), "[Item]");
    }

    #[test]
    fn bracket_object_nested() {
        let buf = Buffer::from_text("f(g[1], 2)\n");
        let inner = buf_text_index("f(g[1], 2)\n", "1");
        assert_eq!(resolve_str(&buf, inner, "di["), "1");
    }

    fn buf_text_index(hay: &str, needle: &str) -> usize {
        hay.find(needle).unwrap()
    }

    #[test]
    fn word_motions_and_objects() {
        let buf = Buffer::from_text("let edge = hone(xs);\n");
        assert_eq!(resolve_str(&buf, 0, "dw"), "let ");
        assert_eq!(resolve_str(&buf, 0, "de"), "let");
        assert_eq!(resolve_str(&buf, 4, "diw"), "edge");
    }

    #[test]
    fn big_word_objects_are_whitespace_delimited() {
        // ciW was an invalid command (0028 P2); the WORD object family
        // spans punctuation where the word object stops at it
        let text = "call foo(bar, baz) now\n";
        let buf = Buffer::from_text(text);
        let at = |needle: &str| text.find(needle).unwrap();
        assert_eq!(resolve_str(&buf, at("bar"), "diW"), "foo(bar,");
        assert_eq!(resolve_str(&buf, at("bar"), "diw"), "bar");
        assert_eq!(resolve_str(&buf, at("now"), "diW"), "now");
        // parse level: W admits both inner and around forms
        assert!(matches!(
            cmd("ciW").target,
            Target::Object {
                inner: true,
                obj: Object::BigWord
            }
        ));
        assert!(matches!(
            cmd("daW").target,
            Target::Object {
                inner: false,
                obj: Object::BigWord
            }
        ));
    }

    #[test]
    fn inner_word_object_on_blanks_selects_the_blank_run() {
        // nvim: diW/diw on whitespace deletes the run, no refusal
        let buf = Buffer::from_text("foo  bar\n");
        assert_eq!(resolve_str(&buf, 3, "diW"), "  ");
        assert_eq!(resolve_str(&buf, 3, "diw"), "  ");
    }

    #[test]
    fn word_motions_are_multibyte_honest() {
        let buf = Buffer::from_text("héllo wörld 🦀\n");
        assert_eq!(resolve_str(&buf, 0, "dw"), "héllo ", "é is a word char");
        let res = r(&buf, 0, "w");
        let pos = res.range.end.get(); // forward motions carry [cursor, target)
        assert!(buf.is_boundary(pos), "w lands on a boundary: {pos}");
        assert_eq!(buf.byte(pos), b'w');
        // emoji is not a word char: w from wörld lands on 🦀's start
        let res = r(&buf, "héllo ".len(), "w");
        assert_eq!(buf.byte(res.range.end), 0xF0, "on the emoji lead byte");
        let res = r(&buf, "héllo ".len(), "e");
        assert!(buf.is_boundary(buf.clamp_boundary(res.range.end)));
        // backward across the multibyte word
        let res = r(&buf, "héllo wörld ".len(), "b");
        assert_eq!(buf.byte(res.range.start), b'w');
    }

    #[test]
    fn doubled_operator_is_linewise() {
        let buf = Buffer::from_text(SRC);
        let res = r(&buf, 3, "dd");
        assert!(res.range.is_linewise());
        assert_eq!(buf.slice_string(res.range), "fn f(xs: &[Item]) -> Edge {\n");
    }

    #[test]
    fn find_and_till() {
        let buf = Buffer::from_text("edge.polish(Finish::Mirror)\n");
        assert_eq!(resolve_str(&buf, 0, "dt:"), "edge.polish(Finish");
    }

    #[test]
    fn search_motion_is_exclusive() {
        let buf = Buffer::from_text(SRC);
        let res = r(&buf, 0, "d/hone\r");
        assert_eq!(
            buf.slice_string(res.range),
            "fn f(xs: &[Item]) -> Edge {\n    let edge = "
        );
        assert!(!res.range.inclusive());
    }

    #[test]
    fn search_motion_lands_on_the_match_start() {
        let buf = Buffer::from_text("fn one() { two() }\n");
        // forward: cursor lands on the match start (range end)
        let c = cmd("/two\r");
        let res = resolve(&buf, 0, &c).unwrap().unwrap();
        assert_eq!(
            cursor_after(&buf, 0, &c, &res),
            "fn one() { ".len(),
            "cursor lands on the t, not pattern-len math"
        );
        // backward: lands on the match start via motion_target — an
        // explicit range, so a shorter match than the pattern still
        // lands exactly (the old code subtracted pattern.len()).
        let c = cmd("?two\r");
        let res = resolve(&buf, buf.len_bytes(), &c).unwrap().unwrap();
        assert_eq!(
            cursor_after(&buf, buf.len_bytes(), &c, &res),
            "fn one() { ".len()
        );
    }

    #[test]
    fn search_motion_supports_the_regex_dialect() {
        let buf = Buffer::from_text("foo bar baz\nqux\n");
        // d/ba\+ deletes through the "ba" match — the match range is
        // explicit; nothing assumes the pattern's own length
        let res = r(&buf, 0, "d/ba\\+\r");
        assert_eq!(buf.slice_string(res.range), "foo ");
    }

    #[test]
    fn counts_multiply() {
        let buf = Buffer::from_text("one two three four\n");
        assert_eq!(resolve_str(&buf, 0, "d2w"), "one two ");
    }

    #[test]
    fn spec_footer_names_the_target() {
        let buf = Buffer::from_text(SRC);
        let res = r(&buf, SRC.find("Item").unwrap(), "ci[");
        assert!(res.spec.contains("change"), "{:?}", res.spec);
        assert!(res.spec.contains("inner ["), "{:?}", res.spec);
        assert!(res.spec.contains("inclusive"), "{:?}", res.spec);
    }

    #[test]
    fn cw_changes_to_word_end_never_trailing_space() {
        // vim: cw behaves like ce — and at a word's last char it still
        // changes only that word (single-char words included)
        let buf = Buffer::from_text("x = 1\n");
        assert_eq!(resolve_str(&buf, 0, "cw"), "x");
        let buf = Buffer::from_text("alpha = 1\n");
        assert_eq!(resolve_str(&buf, 2, "cw"), "pha"); // on 'p'
        assert_eq!(resolve_str(&buf, 3, "cw"), "ha"); // on 'h'
                                                      // on whitespace, cw reaches like e
        let buf = Buffer::from_text("  beta = 1\n");
        assert_eq!(resolve_str(&buf, 0, "cw"), "  beta");
    }
}

mod zero_tests {
    use crate::*;
    use strop_core::Buffer;

    #[test]
    fn zero_is_line_start_not_count() {
        let buf = Buffer::from_text("    let edge = hone(xs);\n");
        let r = resolve(&buf, 10, &super::contract::cmd("d0"))
            .unwrap()
            .unwrap();
        assert_eq!(buf.slice_string(r.range), "    let ed");
        // and bare 0 is a complete motion, not a pending count
        assert!(matches!(parse("0"), Parse::Complete(_)));
        // counts still parse past the rule
        assert!(matches!(parse("10dd"), Parse::Complete(_)));
    }
}

mod extended {
    use crate::*;
    use strop_core::Buffer;

    const SRC: &str = "fn f(xs: &[Item]) -> Edge {\n    let edge = hone(xs);\n}\n";

    #[test]
    fn named_register_prefix() {
        match parse("\"adi[") {
            Parse::Complete(c) => assert_eq!(c.register, Some('a')),
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse("\"a"), Parse::Incomplete));
        assert!(matches!(parse("\"!"), Parse::Invalid));
    }

    fn r(buf: &Buffer, cursor: usize, keys: &str) -> Resolved {
        resolve(buf, cursor, &super::contract::cmd(keys))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn match_pair_both_sides() {
        let buf = Buffer::from_text(SRC);
        let open = SRC.find('[').unwrap();
        let close = SRC.find(']').unwrap();
        let res = r(&buf, open, "d%");
        assert_eq!(buf.slice_string(res.range), "[Item]");
        // from the close, the range spans back to the open
        let res = r(&buf, close, "d%");
        assert_eq!(buf.slice_string(res.range), "[Item]");
        // not on a bracket: scans the line for the first one
        let res = r(&buf, 0, "d%");
        assert!(buf.slice_string(res.range).contains("(xs: &[Item])"));
    }

    #[test]
    fn big_word_skips_punctuation() {
        let buf = Buffer::from_text("edge.polish(Finish::Mirror) tail\n");
        let res = r(&buf, 0, "dW");
        assert_eq!(buf.slice_string(res.range), "edge.polish(Finish::Mirror) ");
        // small w stops at the dot
        let res = r(&buf, 0, "dw");
        assert_eq!(buf.slice_string(res.range), "edge");
    }
}

mod cursor_moves {
    use super::contract;
    use crate::*;
    use strop_core::Buffer;

    #[test]
    fn l_moves_right_and_stops_at_line_end() {
        let buf = Buffer::from_text("ab\ncd\n");
        let c = contract::cmd("l");
        let r = resolve(&buf, 0, &c).unwrap().unwrap();
        assert_eq!(cursor_after(&buf, 0, &c, &r), 1);
        // at line end (col 1 = 'b'), l is a no-op — never crosses to line 2
        let r = resolve(&buf, 1, &c).unwrap().unwrap();
        assert_eq!(cursor_after(&buf, 1, &c, &r), 1);
    }

    #[test]
    fn h_stops_at_line_start() {
        let buf = Buffer::from_text("ab\ncd\n");
        let c = contract::cmd("h");
        let r = resolve(&buf, 1, &c).unwrap().unwrap();
        assert_eq!(cursor_after(&buf, 1, &c, &r), 0);
        let r = resolve(&buf, 0, &c).unwrap().unwrap();
        assert_eq!(cursor_after(&buf, 0, &c, &r), 0);
    }
}

/// The differential corpus: every row was produced by real vim 9.1
/// searching a real buffer (`searchpos`), so it pins buffer semantics
/// (`.` never crosses a line break, `^`/`$` are line anchors, `\n`
/// consumes one whole line break) — not `match()` string semantics.
mod query_corpus {
    use crate::*;
    use strop_core::Buffer;

    /// `pattern \t text \t start \t end`, with \t \n \r \\ escapes in
    /// the first two fields. -1/-1 = no match.
    const CORPUS: &str = include_str!("../corpora/vim-query-corpus.txt");

    fn unescape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('t') => out.push('\t'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('\\') => out.push('\\'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn dialect_matches_vim_on_the_corpus() {
        let mut checked = 0usize;
        let mut failures = Vec::new();
        for line in CORPUS
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let mut fields = line.split('\t');
            let (Some(pat), Some(text), Some(start), Some(end)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                failures.push(format!("malformed row: {line:?}"));
                continue;
            };
            let (pat, text) = (unescape(pat), unescape(text));
            let (want_start, want_end) = (
                start.parse::<isize>().unwrap(),
                end.parse::<isize>().unwrap(),
            );
            let buf = Buffer::from_text(&text);
            match CompiledQuery::compile(&pat, false) {
                // patterns vim runs but this dialect refuses are bugs
                // here — the corpus contains only supported syntax
                Err(e) => failures.push(format!("{pat:?} on {text:?}: refused: {e}")),
                Ok(q) => match search_forward(&buf, 0, &q) {
                    Err(e) => failures.push(format!("{pat:?} on {text:?}: engine: {e}")),
                    Ok(Some(m)) => {
                        let (s, e) = (m.start.get() as isize, m.end.get() as isize);
                        if (s, e) != (want_start, want_end) {
                            failures.push(format!(
                                "{pat:?} on {text:?}: got {s}..{e}, vim says {want_start}..{want_end}"
                            ));
                        }
                    }
                    Ok(None) => {
                        if want_start != -1 {
                            failures.push(format!(
                                "{pat:?} on {text:?}: no match, vim says {want_start}..{want_end}"
                            ));
                        }
                    }
                },
            }
            checked += 1;
        }
        assert!(checked > 150, "corpus must load (got {checked} rows)");
        assert!(
            failures.is_empty(),
            "{} of {checked} rows diverge:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}

/// Typed refusals and documented extensions beyond the corpus.
mod query_dialect {
    use crate::*;
    use strop_core::Buffer;

    fn err(pat: &str) -> QueryError {
        CompiledQuery::compile(pat, false).expect_err("must refuse")
    }

    #[test]
    fn unsupported_constructs_are_typed_errors() {
        for (pat, construct) in [
            ("a\\@=b", "\\@= (lookahead)"),
            ("\\@!", "\\@! (negative lookahead)"),
            ("x\\@<=y", "\\@<= (lookbehind)"),
            ("\\%Vx", "\\%V (visual-area match)"),
            ("\\%#x", "\\%# (cursor match)"),
            ("\\%23lx", "\\%l / \\%c / \\%v (line/column match)"),
            ("~x", "~ (last substitute pattern)"),
            ("\\Mab*", "\\M (nomagic)"),
            ("\\Vab*", "\\V (very nomagic)"),
            ("a\\v{2}", "magic-level prefix (only at pattern start)"),
            ("[a&&b]", "[a&&b] class intersection"),
        ] {
            let e = err(pat);
            match &e {
                QueryError::Unsupported { construct: got, .. } => {
                    assert_eq!(got, &construct, "{pat}: named the construct");
                }
                other => panic!("{pat}: expected Unsupported, got {other:?}"),
            }
        }
    }

    #[test]
    fn malformed_queries_are_typed_errors() {
        for (pat, want) in [
            ("\\(ab", QueryError::UnbalancedGroup { at: 0 }),
            ("ab\\)", QueryError::UnbalancedGroup { at: 2 }),
            ("[abc", QueryError::UnclosedClass { at: 0 }),
            ("a\\{x}", QueryError::BadRepeat { at: 1 }),
            ("a\\{2,3", QueryError::BadRepeat { at: 1 }),
            ("a\\", QueryError::TrailingBackslash { at: 1 }),
            ("a**", QueryError::BadRepeat { at: 2 }),
            ("\\v+?", QueryError::BadRepeat { at: 2 }),
            ("[z-a]", QueryError::BadClass { at: 0 }),
            ("[[:foo:]]", QueryError::BadClass { at: 0 }),
            ("\\%d", QueryError::BadCharCode { at: 0 }),
        ] {
            assert_eq!(err(pat), want, "{pat}");
        }
    }

    #[test]
    fn parse_refuses_uncompilable_search_motions() {
        match parse("d/\\(x\r") {
            Parse::QueryError(QueryError::UnbalancedGroup { .. }) => {}
            other => panic!("typed query error expected, got {other:?}"),
        }
        // and the supported pattern still completes with the query aboard
        match parse("/foo\\|bar\r") {
            Parse::Complete(Command {
                target: Target::Motion(Motion::Search(q)),
                ..
            }) => {
                assert_eq!(q.source(), "foo\\|bar");
                assert!(!q.whole_word());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn whole_word_folds_into_the_query() {
        let buf = Buffer::from_text("honed hone honed\n");
        let plain = CompiledQuery::compile("hone", false).unwrap();
        let word = CompiledQuery::compile("hone", true).unwrap();
        assert!(!plain.whole_word());
        assert!(word.whole_word());
        assert_eq!(
            search_forward(&buf, 0, &plain)
                .unwrap()
                .map(|m| m.start.get()),
            Some(0),
            "plain matches the prefix of `honed`"
        );
        assert_eq!(
            search_forward(&buf, 0, &word)
                .unwrap()
                .map(|m| m.start.get()),
            Some(6),
            "whole-word skips the `hon`-prefixed words"
        );
        // the filter rides every entry point — no caller re-checks
        assert_eq!(search_all(&buf, &word).unwrap().len(), 1);
    }

    #[test]
    fn crlf_line_breaks_are_one_unit() {
        let buf = Buffer::from_text("ab\r\ncd\r\n");
        let q = CompiledQuery::compile("b\\nc", false).unwrap();
        let m = search_forward(&buf, 0, &q)
            .unwrap()
            .expect("matches across CRLF");
        assert_eq!(
            (m.start.get(), m.end.get()),
            (1, 5),
            "consumes the \\r\\n pair"
        );
        // `.` still never crosses a CRLF break
        let q = CompiledQuery::compile("b.c", false).unwrap();
        assert!(search_forward(&buf, 0, &q).unwrap().is_none());
        // and a `^` anchor knows the break
        let q = CompiledQuery::compile("^cd", false).unwrap();
        let m = search_forward(&buf, 0, &q)
            .unwrap()
            .expect("anchors after CRLF");
        assert_eq!(m.start.get(), 4);
    }

    #[test]
    fn lone_cr_is_a_plain_char() {
        let buf = Buffer::from_text("a\rb\nc\r\n");
        let q = CompiledQuery::compile("a\\rb", false).unwrap();
        let m = search_forward(&buf, 0, &q)
            .unwrap()
            .expect("lone CR matches");
        assert_eq!((m.start.get(), m.end.get()), (0, 3));
    }

    #[test]
    fn bounded_engine_errors_on_pathological_patterns() {
        // \(a\|aa\)\+$ against a long non-matching tail explodes
        // combinatorially — the budget turns that into TooComplex
        let text = "a".repeat(60).to_string() + &"b".repeat(200);
        let buf = Buffer::from_text(&text);
        let q = CompiledQuery::compile("\\(a\\|aa\\)\\+$", false).unwrap();
        match search_all(&buf, &q) {
            Err(QueryError::TooComplex) => {}
            other => panic!("expected the step budget, got {other:?}"),
        }
    }

    #[test]
    fn unicode_word_boundaries_follow_the_editor_model() {
        // documented extension: é is a word char for \< \> (it is for
        // w/b/e and `*` too) — ASCII rows agree with vim in the corpus
        let buf = Buffer::from_text("un café ouvert\n");
        let q = CompiledQuery::compile("\\<caf", false).unwrap();
        let m = search_forward(&buf, 0, &q)
            .unwrap()
            .expect("boundary after space");
        assert_eq!(m.start.get(), 3);
        let q = CompiledQuery::compile("\\<af", false).unwrap();
        assert!(
            search_forward(&buf, 0, &q).unwrap().is_none(),
            "mid-word is not a boundary"
        );
    }

    #[test]
    fn resolve_many_resolves_every_cursor_independently() {
        let buf = Buffer::from_text("one two one two\n");
        let c = contract_cmd("/two\r");
        let out = resolve_many(&buf, &[0, 4], &c).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out[0].is_some() && out[1].is_some());
        let lands: Vec<usize> = out
            .iter()
            .zip([0usize, 4])
            .map(|(r, cur)| match r {
                Some(r) => cursor_after(&buf, cur, &c, r),
                None => unreachable!(),
            })
            .collect();
        assert_eq!(lands, vec![4, 12], "each cursor finds its own next match");
    }

    fn contract_cmd(keys: &str) -> Command {
        match parse(keys) {
            Parse::Complete(c) => c,
            other => panic!("{other:?}"),
        }
    }
}
