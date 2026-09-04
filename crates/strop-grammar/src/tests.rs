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

    fn resolve_str(buf: &Buffer, cursor: usize, keys: &str) -> String {
        let c = cmd(keys);
        let r = resolve(buf, cursor, &c).expect("resolvable");
        buf.slice_string(r.range)
    }

    #[test]
    fn bracket_object_from_inside() {
        let buf = Buffer::from_text(SRC);
        let cursor = SRC.find("Item").unwrap() + 2; // inside [Item]
        assert_eq!(resolve_str(&buf, cursor, "di["), "Item");
    }

    #[test]
    fn bracket_object_cursor_on_open() {
        let buf = Buffer::from_text(SRC);
        let cursor = SRC.find('[').unwrap();
        assert_eq!(resolve_str(&buf, cursor, "ci["), "Item");
    }

    #[test]
    fn bracket_object_cursor_on_close() {
        let buf = Buffer::from_text(SRC);
        let cursor = SRC.find(']').unwrap();
        assert_eq!(resolve_str(&buf, cursor, "di["), "Item");
    }

    #[test]
    fn bracket_object_around_includes_delimiters() {
        let buf = Buffer::from_text(SRC);
        let cursor = SRC.find("Item").unwrap();
        assert_eq!(resolve_str(&buf, cursor, "da["), "[Item]");
    }

    #[test]
    fn bracket_object_nested() {
        let buf = Buffer::from_text("f(a, g(b, c), d)\n");
        let cursor = 9; // 'b' — inside inner parens
        assert_eq!(resolve_str(&buf, cursor, "di("), "b, c");
        // from 'a', the enclosing pair is the outer one
        assert_eq!(resolve_str(&buf, 3, "di("), "a, g(b, c), d");
    }

    #[test]
    fn word_motions_and_objects() {
        let buf = Buffer::from_text("let edge = hone(xs);\n");
        assert_eq!(resolve_str(&buf, 0, "dw"), "let ");
        assert_eq!(resolve_str(&buf, 0, "de"), "let");
        assert_eq!(resolve_str(&buf, 4, "diw"), "edge");
    }

    #[test]
    fn word_motions_are_multibyte_honest() {
        // regression: byte-wise classes split héllo at the é boundary,
        // parked the cursor mid-char, and the next x panicked ropey
        let buf = Buffer::from_text("héllo wörld 🦀 em\n");
        assert_eq!(resolve_str(&buf, 0, "dw"), "héllo ", "é is a word char");
        let r = resolve(&buf, 0, &cmd("w")).unwrap();
        let pos = r.range.end; // forward motions carry [cursor, target)
        assert!(buf.is_boundary(pos), "w lands on a boundary: {pos}");
        assert_eq!(buf.byte(pos), b'w');
        // emoji is not a word char: w from wörld lands on 🦀's start
        let r = resolve(&buf, "héllo ".len(), &cmd("w")).unwrap();
        assert_eq!(buf.byte(r.range.end), 0xF0, "on the emoji lead byte");
        let r = resolve(&buf, "héllo ".len(), &cmd("e")).unwrap();
        assert!(buf.is_boundary(buf.clamp_boundary(r.range.end)));
        // backward across the multibyte word
        let r = resolve(&buf, "héllo wörld ".len(), &cmd("b")).unwrap();
        assert_eq!(buf.byte(r.range.start), b'w');
    }

    #[test]
    fn doubled_operator_is_linewise() {
        let buf = Buffer::from_text(SRC);
        let r = resolve(&buf, 3, &cmd("dd")).unwrap();
        assert!(r.range.linewise);
        assert_eq!(buf.slice_string(r.range), "fn f(xs: &[Item]) -> Edge {\n");
    }

    #[test]
    fn find_and_till() {
        let buf = Buffer::from_text("edge.polish(Finish::Mirror)\n");
        assert_eq!(resolve_str(&buf, 0, "df:"), "edge.polish(Finish:");
        assert_eq!(resolve_str(&buf, 0, "dt:"), "edge.polish(Finish");
    }

    #[test]
    fn search_motion_is_exclusive() {
        let buf = Buffer::from_text(SRC);
        let c = cmd("d/hone\r");
        let r = resolve(&buf, 0, &c).unwrap();
        assert_eq!(
            buf.slice_string(r.range),
            "fn f(xs: &[Item]) -> Edge {\n    let edge = "
        );
        assert!(!r.inclusive);
    }

    #[test]
    fn counts_multiply() {
        let buf = Buffer::from_text("one two three four\n");
        assert_eq!(resolve_str(&buf, 0, "d2w"), "one two ");
    }

    #[test]
    fn spec_footer_names_the_target() {
        let buf = Buffer::from_text(SRC);
        let r = resolve(&buf, SRC.find("Item").unwrap(), &cmd("ci[")).unwrap();
        assert!(r.spec.contains("change"), "{:?}", r.spec);
        assert!(r.spec.contains("inner ["), "{:?}", r.spec);
        assert!(r.spec.contains("inclusive"), "{:?}", r.spec);
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
        let r = resolve(&buf, 10, &super::contract::cmd("d0")).unwrap();
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

    #[test]
    fn match_pair_both_sides() {
        let buf = Buffer::from_text(SRC);
        let open = SRC.find('[').unwrap();
        let close = SRC.find(']').unwrap();
        let c = super::contract::cmd("d%");
        let r = resolve(&buf, open, &c).unwrap();
        assert_eq!(buf.slice_string(r.range), "[Item]");
        // from the close, the range spans back to the open
        let r = resolve(&buf, close, &c).unwrap();
        assert_eq!(buf.slice_string(r.range), "[Item]");
        // not on a bracket: scans the line for the first one
        let r = resolve(&buf, 0, &c).unwrap();
        assert!(buf.slice_string(r.range).contains("(xs: &[Item])"));
    }

    #[test]
    fn big_word_skips_punctuation() {
        let buf = Buffer::from_text("edge.polish(Finish::Mirror) tail\n");
        let r = resolve(&buf, 0, &super::contract::cmd("dW")).unwrap();
        assert_eq!(buf.slice_string(r.range), "edge.polish(Finish::Mirror) ");
        // small w stops at the dot
        let r = resolve(&buf, 0, &super::contract::cmd("dw")).unwrap();
        assert_eq!(buf.slice_string(r.range), "edge");
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
        let r = resolve(&buf, 0, &c).unwrap();
        assert_eq!(cursor_after(&buf, 0, &c, &r), 1);
        // at line end (col 1 = 'b'), l is a no-op — never crosses to line 2
        let r = resolve(&buf, 1, &c).unwrap();
        assert_eq!(cursor_after(&buf, 1, &c, &r), 1);
    }

    #[test]
    fn h_stops_at_line_start() {
        let buf = Buffer::from_text("ab\ncd\n");
        let c = contract::cmd("h");
        let r = resolve(&buf, 1, &c).unwrap();
        assert_eq!(cursor_after(&buf, 1, &c, &r), 0);
        let r = resolve(&buf, 0, &c).unwrap();
        assert_eq!(cursor_after(&buf, 0, &c, &r), 0);
    }
}
