use super::*;

fn hits(buf: &Buffer, pat: &str) -> Vec<(usize, usize)> {
    let q = CompiledQuery::compile(pat, false).expect("compiles");
    all(buf, &q)
        .expect("runs")
        .into_iter()
        .map(|m| (m.start.get(), m.end.get()))
        .collect()
}

#[test]
fn literal_hits_carry_explicit_ends() {
    let buf = Buffer::from_text("ab cd ab\n");
    assert_eq!(hits(&buf, "ab"), vec![(0, 2), (6, 8)]);
}

#[test]
fn regex_hits_carry_explicit_ends() {
    let buf = Buffer::from_text("a ab abc\n");
    assert_eq!(hits(&buf, "a\\+"), vec![(0, 1), (2, 3), (5, 6)]);
}

#[test]
fn captures_ride_each_match_and_an_unused_group_is_none() {
    let buf = Buffer::from_text("ab cd\n");
    let q = CompiledQuery::compile("\\(a\\)\\(x\\)\\?b\\|\\(c\\)d", false).expect("compiles");
    let mut seen = Vec::new();
    visit_captures(&buf, &q, 0, |hit, groups| {
        let groups: Vec<_> = groups
            .iter()
            .map(|group| group.map(|group| (group.start.get(), group.end.get())))
            .collect();
        seen.push(((hit.start.get(), hit.end.get()), groups));
        std::ops::ControlFlow::Continue(())
    })
    .expect("runs");
    assert_eq!(
        seen,
        vec![
            ((0, 2), vec![Some((0, 1)), None, None]),
            ((3, 5), vec![None, None, Some((3, 4))]),
        ]
    );
}

#[test]
fn empty_matches_walk_but_never_past_eof() {
    // vim counts 3 matches for x* on "aaa": one per position, none
    // after the final char
    let buf = Buffer::from_text("bbb\n");
    assert_eq!(hits(&buf, "x*"), vec![(0, 0), (1, 1), (2, 2), (3, 3)]);
}

/// The bounded engine's answer for the same literal: a capture group
/// keeps the program off the scanner without changing its matches or
/// its case mode.
fn engine_twin(pat: &str, whole_word: bool) -> CompiledQuery {
    let twin = CompiledQuery::compile(&format!("\\({pat}\\)"), whole_word).expect("compiles");
    assert!(twin.program().literal.is_none());
    twin
}

fn spans(hits: Vec<SearchMatch>) -> Vec<(usize, usize)> {
    hits.into_iter()
        .map(|m| (m.start.get(), m.end.get()))
        .collect()
}

/// Every entry point answers as the engine does for `pat` on `text`.
fn assert_same_as_engine(text: &str, pat: &str, whole_word: bool) {
    let buf = Buffer::from_text(text);
    let fast = CompiledQuery::compile(pat, whole_word).expect("compiles");
    assert!(
        fast.program().literal.is_some(),
        "{pat:?} takes the scanner"
    );
    let slow = engine_twin(pat, whole_word);
    let ctx = || format!("{pat:?} (whole word {whole_word}) on {text:?}");
    assert_eq!(
        spans(all(&buf, &fast).unwrap()),
        spans(all(&buf, &slow).unwrap()),
        "all: {}",
        ctx()
    );
    let mut cuts: Vec<usize> = (0..=buf.len_bytes())
        .filter(|at| text.is_char_boundary(*at))
        .collect();
    if cuts.len() > 64 {
        let step = cuts.len() / 32;
        cuts = cuts.into_iter().step_by(step).collect();
    }
    for at in cuts {
        assert_eq!(
            forward(&buf, at, &fast).unwrap(),
            forward(&buf, at, &slow).unwrap(),
            "forward from {at}: {}",
            ctx()
        );
        assert_eq!(
            backward(&buf, at, &fast).unwrap(),
            backward(&buf, at, &slow).unwrap(),
            "backward from {at}: {}",
            ctx()
        );
    }
}

#[test]
fn literal_shapes_take_the_scanner_and_the_rest_do_not() {
    let literal = |pat: &str, whole_word| {
        CompiledQuery::compile(pat, whole_word)
            .unwrap()
            .program()
            .literal
            .clone()
    };
    // smartcase: lower case folds, a capital is exact
    let folded = literal("needle", false).unwrap();
    assert!(folded.fold && !folded.word_before && !folded.word_after);
    assert!(!literal("Needle", false).unwrap().fold);
    assert!(literal("\\<needle\\>", false).unwrap().word_before);
    let star = literal("needle", true).unwrap();
    assert!(star.fold && star.word_before && star.word_after);
    // a folded non-ASCII needle, a non-word flank and real regex syntax
    // keep the engine
    for pat in [
        "café",
        "\\<-x",
        "x-\\>",
        "ne.dle",
        "\\zsneedle",
        "^needle",
        "a\\|b",
    ] {
        assert!(literal(pat, false).is_none(), "{pat:?}");
    }
    assert!(
        literal("CAFÉ", false).is_some(),
        "an exact needle may be any UTF-8"
    );
}

#[test]
fn self_overlapping_needles_walk_like_vim() {
    // nvim: /aa on "aaaa" counts 2 per line, `:s/aa/b/g` gives "bb"
    let buf = Buffer::from_text("aaaa\nAAAA\n");
    assert_eq!(hits(&buf, "aa"), vec![(0, 2), (2, 4), (5, 7), (7, 9)]);
    assert_eq!(hits(&buf, "AA"), vec![(5, 7), (7, 9)]);
    assert_same_as_engine("aaaa\nAAAA\n", "aa", false);
    assert_same_as_engine("aaaa\nAAAA\n", "AA", false);
}

#[test]
fn folded_and_whole_word_literals_match_the_engine() {
    let text = "Needle needle_x xneedle NEEDLE éneedle needleé needle\nneedle";
    for pat in [
        "needle",
        "\\<needle",
        "needle\\>",
        "\\<needle\\>",
        "Needle",
        "\\cNEEDLE",
    ] {
        assert_same_as_engine(text, pat, false);
    }
    assert_same_as_engine(text, "needle", true);
    assert_same_as_engine("a_b a-b A-B", "a-b", false);
    assert_same_as_engine("x\r\nx", "x", true);
}

#[test]
fn kelvin_sign_folds_to_k_and_hands_back_to_the_engine() {
    // U+212A lowercases to `k`: the engine matches it, so must search
    let text = "make MAKE ma\u{212A}e ok \u{212A} mak";
    let buf = Buffer::from_text(text);
    assert_eq!(hits(&buf, "make").len(), 3);
    for pat in ["make", "k", "ok", "\\<k\\>", "mak"] {
        assert_same_as_engine(text, pat, false);
    }
    assert_same_as_engine(text, "k", true);
}

#[test]
fn matches_straddling_scan_windows_and_rope_chunks_are_found() {
    // the scanner decides 64 KiB per window; put needles across every
    // offset around the first window edges, and near a KELVIN SIGN there
    const EDGE: usize = 64 * 1024;
    for shift in 0..12 {
        for (needle, pat) in [("needle", "needle"), ("Needle", "Needle"), ("ak", "ak")] {
            let mut text = "x".repeat(EDGE - shift);
            text.push_str(needle);
            text.push_str(&"y ".repeat(40));
            text.push_str(needle);
            let buf = Buffer::from_text(&text);
            let want = vec![
                (EDGE - shift, EDGE - shift + needle.len()),
                (text.len() - needle.len(), text.len()),
            ];
            assert_eq!(hits(&buf, pat), want, "{pat} at {shift}");
            let mut near = "x".repeat(EDGE - shift);
            near.push_str("a\u{212A} ak ");
            near.push_str(&"z".repeat(100));
            near.push_str("a\u{212A}");
            let slow = engine_twin("ak", false);
            let fast = CompiledQuery::compile("ak", false).unwrap();
            let buf = Buffer::from_text(&near);
            assert_eq!(
                spans(all(&buf, &fast).unwrap()),
                spans(all(&buf, &slow).unwrap()),
                "kelvin at {shift}"
            );
            assert_eq!(spans(all(&buf, &fast).unwrap()).len(), 3);
        }
    }
}

#[test]
fn random_ascii_texts_answer_as_the_engine_does() {
    // a deterministic generator: small alphabets make dense, overlapping
    // and flanked hits
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |bound: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % bound as u64) as usize
    };
    const TEXT: &[&str] = &["a", "A", "b", "k", "K", " ", "_", "\n", "é", "\u{212A}"];
    const NEEDLE: &[&str] = &["a", "A", "b", "k", "K", " ", "_"];
    for _ in 0..400 {
        let text: String = (0..next(80)).map(|_| TEXT[next(TEXT.len())]).collect();
        let needle: String = (0..1 + next(4))
            .map(|_| NEEDLE[next(NEEDLE.len())])
            .collect();
        let whole_word = next(4) == 0;
        let pat = match next(4) {
            0 => format!("\\<{needle}"),
            1 => format!("{needle}\\>"),
            _ => needle.clone(),
        };
        let first_word = needle.starts_with(|c: char| c.is_alphanumeric() || c == '_');
        let last_word = needle.ends_with(|c: char| c.is_alphanumeric() || c == '_');
        if (pat.starts_with("\\<") && !first_word) || (pat.ends_with("\\>") && !last_word) {
            continue;
        }
        assert_same_as_engine(&text, &pat, whole_word);
    }
}

#[test]
fn occurrence_needles_are_plain_bytes() {
    let buf = Buffer::from_text("a.b a*b a.b\n");
    assert_eq!(literal_from(&buf, 0, b"a.b"), Some((0, 3)));
    assert_eq!(literal_from(&buf, 1, b"a.b"), Some((8, 11)));
    assert_eq!(literal_from(&buf, 9, b"a.b"), None);
    assert_eq!(literal_from(&buf, 0, b""), None);
}
