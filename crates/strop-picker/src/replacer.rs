//! What With puts in place of one hit (0094 §3.2). The picker's `-`/`+`
//! rows and the review are built from these two functions, so the preview
//! cannot disagree with the result.
use std::borrow::Cow;

/// The clamped, char-boundary-safe byte span of a match on its line.
pub fn replace_span(line_text: &str, col: usize, match_len: usize) -> (usize, usize) {
    let mut start = col.saturating_sub(1).min(line_text.len());
    while start > 0 && !line_text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = start.saturating_add(match_len).min(line_text.len());
    while end > start && !line_text.is_char_boundary(end) {
        end -= 1;
    }

    (start, end)
}

/// With, read against the term it replaces. A term with capture groups
/// (only a `regex:` term has any) makes With a template: `$1`, `${name}`
/// and `\1` expand from each hit's own groups, `$0` and `\0` are the whole
/// hit, `$$` is a dollar and `\\` a backslash. Any other term inserts With
/// exactly as typed.
#[derive(Debug, Clone)]
pub struct Replacer {
    template: Option<(regex::Regex, String)>,
    with: String,
}

impl Replacer {
    pub fn new(target: Option<&regex::Regex>, with: &str) -> Self {
        Self {
            template: target
                .filter(|regex| regex.captures_len() > 1)
                .map(|regex| (regex.clone(), dollar_template(with))),
            with: with.to_owned(),
        }
    }

    /// The text replacing `matched`, the hit's own bytes. `None`: the term
    /// no longer matches exactly this text, so the hit is stale.
    pub fn text<'a>(&'a self, matched: &str) -> Option<Cow<'a, str>> {
        let Some((regex, template)) = &self.template else {
            return Some(Cow::Borrowed(&self.with));
        };
        let captures = regex.captures(matched)?;
        let whole = captures.get(0)?;
        if whole.start() != 0 || whole.end() != matched.len() {
            return None;
        }
        let mut text = String::new();
        captures.expand(template, &mut text);
        Some(Cow::Owned(text))
    }
}

/// With in the `regex` crate's template spelling. Numbered groups are braced
/// (`$1_` is group 1 then an underscore, not a group named `1_`), vim's `\1`
/// is `${1}`, and `\\` is one backslash.
fn dollar_template(with: &str) -> String {
    let mut template = String::with_capacity(with.len());
    let mut chars = with.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek().copied()) {
            ('$', Some('$')) => {
                chars.next();
                template.push_str("$$");
            }
            ('$', Some(digit)) if digit.is_ascii_digit() => {
                template.push_str("${");
                while let Some(digit) = chars.next_if(char::is_ascii_digit) {
                    template.push(digit);
                }
                template.push('}');
            }
            ('\\', Some(digit)) if digit.is_ascii_digit() => {
                chars.next();
                template.push_str("${");
                template.push(digit);
                template.push('}');
            }
            ('\\', Some('\\')) => {
                chars.next();
                template.push('\\');
            }
            _ => template.push(c),
        }
    }
    template
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replaced(pattern: &str, with: &str, matched: &str) -> Option<String> {
        let regex = regex::Regex::new(pattern).unwrap();
        Replacer::new(Some(&regex), with)
            .text(matched)
            .map(Cow::into_owned)
    }

    #[test]
    fn groups_expand_in_both_spellings() {
        assert_eq!(
            replaced(r"get_(\w+)", r"fetch_$1_x", "get_alpha").as_deref(),
            Some("fetch_alpha_x")
        );
        assert_eq!(
            replaced(r"(\w+)=(\w+)", r"\2=\1", "key=value").as_deref(),
            Some("value=key")
        );
        assert_eq!(
            replaced(r"(?P<name>\w+)!", "${name}?", "stop!").as_deref(),
            Some("stop?")
        );
        assert_eq!(
            replaced(r"(a)b", r"[$0|\0] $$1 \\1", "ab").as_deref(),
            Some(r"[ab|ab] $1 \1")
        );
    }

    #[test]
    fn a_term_without_groups_keeps_with_literal() {
        assert_eq!(replaced(r"foo", r"$1 \1", "foo").as_deref(), Some(r"$1 \1"));
        assert_eq!(
            Replacer::new(None, "$1").text("anything").as_deref(),
            Some("$1")
        );
    }

    #[test]
    fn a_hit_the_term_no_longer_matches_is_stale() {
        assert_eq!(replaced(r"get_(\w+)", "x", "set_alpha"), None);
        assert_eq!(replaced(r"(a)", "x", "ab"), None);
    }

    #[test]
    fn spans_clamp_to_the_line_and_to_char_boundaries() {
        assert_eq!(replace_span("héllo", 3, 2), (1, 3));
        assert_eq!(replace_span("abc", 9, 4), (3, 3));
    }
}
