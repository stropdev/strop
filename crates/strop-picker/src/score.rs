//! fzf-style subsequence scoring: matched-char columns for the
//! accent+bold render (0001 §4 — never background blocks on matches).

/// Score `query` against `text`. Returns (score, matched char indices)
/// or None when query isn't a subsequence. Scoring rewards: start of
/// string, boundaries (after `/._- `), case-matched camel humps, and
/// consecutive runs.
/// Score `query` against `text`. Returns (score, matched char indices)
/// or None when query isn't a subsequence. Backed by nucleo-matcher
/// (0022 §2 — the bench: 49.6ms → 11.85ms on a 100k-item refilter,
/// identical hit sets, match columns for the accent render).
pub fn fuzzy_score(query: &str, text: &str) -> Option<(i32, Vec<u32>)> {
    fuzzy_score_with_case(query, text, crate::query::CaseMode::default())
}

pub(crate) fn fuzzy_score_with_case(
    query: &str,
    text: &str,
    case: crate::query::CaseMode,
) -> Option<(i32, Vec<u32>)> {
    MATCHER.with(|m| fuzzy_with(&mut m.borrow_mut(), query, text, case))
}

thread_local! {
    static MATCHER: std::cell::RefCell<nucleo_matcher::Matcher> =
        std::cell::RefCell::new(nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT));
}

/// Score with a caller-held matcher (the picker reuses one per refilter
/// pass — a fresh Matcher per row would dominate the cost).
pub fn fuzzy_with(
    matcher: &mut nucleo_matcher::Matcher,
    query: &str,
    text: &str,
    case: crate::query::CaseMode,
) -> Option<(i32, Vec<u32>)> {
    if query.is_empty() {
        return Some((0, vec![]));
    }
    // 0023 P1: Ascii is for ASCII — anything else gets the checked
    // Unicode path (the review's Japanese-filename probe)
    let hay = if text.is_ascii() {
        nucleo_matcher::Utf32Str::Ascii(text.as_bytes())
    } else {
        nucleo_matcher::Utf32Str::Unicode(&text.chars().collect::<Vec<_>>())
    };
    let case = match case {
        crate::query::CaseMode::Smart => nucleo_matcher::pattern::CaseMatching::Smart,
        crate::query::CaseMode::Sensitive => nucleo_matcher::pattern::CaseMatching::Respect,
        crate::query::CaseMode::Ignore => nucleo_matcher::pattern::CaseMatching::Ignore,
    };
    let pat = nucleo_matcher::pattern::Pattern::parse(
        query,
        case,
        nucleo_matcher::pattern::Normalization::Smart,
    );
    let mut cols_u32: Vec<u32> = Vec::new();
    let score = pat.indices(hay, matcher, &mut cols_u32)?;
    // char-indexed columns for the renderer (nucleo reports them
    // already; text is ASCII-fast here — the picker's sources are
    // repo paths and grep rows)
    Some((score as i32, cols_u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_score("rd", "src/render.rs").is_some());
        assert!(fuzzy_score("xyz", "src/render.rs").is_none());
    }

    #[test]
    fn case_is_insensitive_by_default_with_sensitive_and_smart_overrides() {
        for needle in ["mixedcase", "MIXEDCASE", "MixedCase"] {
            assert!(
                fuzzy_score(needle, "src/MixedCase.rs").is_some(),
                "{needle}"
            );
            assert!(
                fuzzy_score(needle, "src/mixedcase.rs").is_some(),
                "{needle}"
            );
        }
        assert!(fuzzy_score_with_case(
            "MIXEDCASE",
            "src/mixedcase.rs",
            crate::query::CaseMode::Sensitive
        )
        .is_none());
        assert!(fuzzy_score_with_case(
            "MIXEDCASE",
            "src/MIXEDCASE.rs",
            crate::query::CaseMode::Sensitive
        )
        .is_some());
        assert!(fuzzy_score_with_case(
            "MIXEDCASE",
            "src/mixedcase.rs",
            crate::query::CaseMode::Smart
        )
        .is_none());
        assert!(fuzzy_score_with_case(
            "mixedcase",
            "src/MixedCase.rs",
            crate::query::CaseMode::Smart
        )
        .is_some());
    }

    #[test]
    fn boundaries_beat_middle() {
        let boundary = fuzzy_score("ren", "src/render.rs").unwrap().0;
        let middle = fuzzy_score("ren", "different.txt").unwrap().0;
        assert!(boundary > middle);
    }

    #[test]
    fn matched_columns_reported() {
        // nucleo's optimal path: boundary-heavy over earliest (0022 §2)
        let (_, cols) = fuzzy_score("rr", "src/render.rs").unwrap();
        assert_eq!(cols, vec![4, 11]);
    }
}
