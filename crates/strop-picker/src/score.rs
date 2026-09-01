//! fzf-style subsequence scoring: matched-char columns for the
//! accent+bold render (0001 §4 — never background blocks on matches).

/// Score `query` against `text`. Returns (score, matched char indices)
/// or None when query isn't a subsequence. Scoring rewards: start of
/// string, boundaries (after `/._- `), case-matched camel humps, and
/// consecutive runs.
pub fn fuzzy_score(query: &str, text: &str) -> Option<(i32, Vec<u32>)> {
    if query.is_empty() {
        return Some((0, vec![]));
    }
    let q: Vec<char> = query.chars().map(|c| c.to_ascii_lowercase()).collect();
    let t: Vec<char> = text.chars().collect();

    // forward pass: prove subsequence, find the earliest viable end
    let mut qi = 0;
    let mut end = t.len();
    for (ti, &tc) in t.iter().enumerate() {
        if qi < q.len() && tc.to_ascii_lowercase() == q[qi] {
            qi += 1;
            if qi == q.len() {
                end = ti;
                break;
            }
        }
    }
    if qi < q.len() {
        return None;
    }

    // backward pass from that end: snap each match to the latest viable
    // position — boundaries and runs earn their bonuses (fzf does the
    // same tighten step)
    let mut cols = vec![0u32; q.len()];
    let mut qi = q.len();
    let mut ti = end + 1;
    while qi > 0 {
        ti -= 1;
        if t[ti].to_ascii_lowercase() == q[qi - 1] {
            qi -= 1;
            cols[qi] = ti as u32;
        }
        if ti == 0 && qi > 0 {
            return None; // unreachable after a successful forward pass
        }
    }

    let mut score = 0i32;
    for (k, &c) in cols.iter().enumerate() {
        let ci = c as usize;
        let boundary = ci == 0 || matches!(t[ci - 1], '/' | '.' | '_' | '-' | ' ');
        let camel = ci > 0 && t[ci - 1].is_lowercase() && t[ci].is_uppercase();
        score += 1;
        if boundary || camel {
            score += 6;
        }
        if k > 0 && cols[k - 1] as usize + 1 == ci {
            score += 4; // consecutive run
        }
    }
    // shorter texts win ties; earlier first-match wins
    score -= (t.len() as i32) / 8;
    score -= cols.first().copied().unwrap_or(0) as i32 / 16;
    Some((score, cols))
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
    fn boundaries_beat_middle() {
        let boundary = fuzzy_score("ren", "src/render.rs").unwrap().0;
        let middle = fuzzy_score("ren", "different.txt").unwrap().0;
        assert!(boundary > middle);
    }

    #[test]
    fn matched_columns_reported() {
        let (_, cols) = fuzzy_score("rr", "src/render.rs").unwrap();
        assert_eq!(cols, vec![1, 4]);
    }
}
