//! The search field's toggles (0093 §3.7). They sit in front of the query
//! language: a toggle changes how the typed expression is read, and an
//! explicit qualifier in the query still wins over it.

use super::parser::{BooleanExpr, CaseMode, ContentExpr, SearchQuery};

/// Every field is "away from the default", so the default value is the
/// all-off state the status row stays silent for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SearchModes {
    /// Case decides a match even for an all-lowercase query.
    pub match_case: bool,
    /// The expression matches only between word boundaries.
    pub whole_word: bool,
    /// Bare text is a regular expression, not a literal.
    pub regex: bool,
    /// Dotfiles are treated the opposite way to the configuration.
    pub flip_hidden: bool,
    /// Ignore files are treated the opposite way to the configuration.
    pub flip_ignored: bool,
}

impl SearchModes {
    /// The expression toggles that are on, in the order the status row
    /// names them. The two visibility flips are named by the caller, who
    /// knows which way the configuration points.
    pub fn expression_labels(&self) -> impl Iterator<Item = &'static str> {
        [
            (self.match_case, "match case"),
            (self.whole_word, "whole word"),
            (self.regex, "regex"),
        ]
        .into_iter()
        .filter_map(|(on, label)| on.then_some(label))
    }
}

fn whole_word(pattern: &str) -> String {
    format!(r"\b(?:{pattern})\b")
}

fn reread(text: &mut String, regex: &mut bool, modes: SearchModes) {
    if modes.regex {
        *regex = true;
    }
    if modes.whole_word {
        *text = whole_word(&if *regex {
            std::mem::take(text)
        } else {
            regex::escape(text)
        });
        *regex = true;
    }
}

fn reread_atoms(expression: &mut BooleanExpr, modes: SearchModes) {
    match expression {
        BooleanExpr::And(operands) | BooleanExpr::Or(operands) => {
            for operand in operands {
                reread_atoms(operand, modes);
            }
        }
        BooleanExpr::Not(inner) => reread_atoms(inner, modes),
        BooleanExpr::Content(atom) => reread(&mut atom.text, &mut atom.regex, modes),
        BooleanExpr::Metadata(_) => {}
    }
}

impl SearchQuery {
    /// Parse a content-search field with its toggles applied. `case:` in
    /// the query overrides the case toggle; an explicit `regex:` is
    /// already a regex, and whole-word wraps whichever form resulted.
    pub fn parse_with(input: &str, modes: SearchModes) -> Self {
        let mut query = Self::parse(input);
        if modes.match_case && query.case.is_none() {
            query.case = Some(CaseMode::Sensitive);
        }
        if let Some(content) = query.content.take() {
            let (mut text, mut regex) = match content {
                ContentExpr::Literal(text) => (text, false),
                ContentExpr::Regex(pattern) => (pattern, true),
            };
            reread(&mut text, &mut regex, modes);
            query.content = Some(if regex {
                ContentExpr::Regex(text)
            } else {
                ContentExpr::Literal(text)
            });
        }
        if let Some(expression) = query.boolean.as_mut() {
            reread_atoms(expression, modes);
        }
        query
    }
}

#[cfg(test)]
mod tests {
    use super::super::ContentPlan;
    use super::*;

    fn admits(input: &str, modes: SearchModes, line: &str) -> bool {
        let query = SearchQuery::parse_with(input, modes);
        ContentPlan::compile(&query)
            .unwrap()
            .unwrap()
            .matches_line(line)
    }

    #[test]
    fn the_default_is_smart_case() {
        let off = SearchModes::default();
        assert!(admits("needle", off, "a NEEDLE"));
        assert!(admits("Needle", off, "a Needle"));
        assert!(!admits("Needle", off, "a needle"));
    }

    #[test]
    fn match_case_holds_a_lowercase_query_to_its_case_and_a_qualifier_wins() {
        let on = SearchModes {
            match_case: true,
            ..Default::default()
        };
        assert!(admits("needle", on, "a needle"));
        assert!(!admits("needle", on, "a NEEDLE"));
        assert!(admits("case:ignore needle", on, "a NEEDLE"));
    }

    #[test]
    fn whole_word_wraps_literals_regexes_and_boolean_atoms() {
        let on = SearchModes {
            whole_word: true,
            ..Default::default()
        };
        assert!(admits("edge", on, "the edge."));
        assert!(!admits("edge", on, "hedgerow edges"));
        // a literal stays literal inside the boundaries
        assert!(admits("a.b", on, "x a.b y"));
        assert!(!admits("a.b", on, "x aXb y"));
        assert!(admits("regex:\"ed|rim\"", on, "a rim"));
        assert!(!admits("regex:\"ed|rim\"", on, "trimmed"));
        assert!(admits("edge OR rim", on, "a rim"));
        assert!(!admits("edge OR rim", on, "trimmed hedges"));
    }

    #[test]
    fn regex_reads_bare_text_as_a_pattern() {
        let on = SearchModes {
            regex: true,
            ..Default::default()
        };
        assert!(admits("fo+", on, "foooo"));
        assert!(!admits("fo+", SearchModes::default(), "foooo"));
        assert!(admits("fo+ AND ba.", on, "foo bar"));
        let invalid = SearchQuery::parse_with("fo(", on);
        assert!(ContentPlan::compile(&invalid).is_err());
    }

    #[test]
    fn smart_case_reads_escapes_as_escapes_not_capitals() {
        let on = SearchModes {
            regex: true,
            ..Default::default()
        };
        assert!(admits(r"needle\S", on, "NEEDLEx"));
        assert!(!admits(r"Needle\S", on, "needlex"));
    }
}
