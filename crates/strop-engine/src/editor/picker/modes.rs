//! The search field's toggle keys and the row that reports them (0093
//! §3.7). One table owns which chord means what on which surface; the
//! status row is built from the same state the source request reads, so
//! the row cannot disagree with the search.

use strop_picker::query::{SearchModes, SearchQuery};
use strop_picker::{Kind, SelectionPolicy};
use strop_workspace::ResourceLocation;

/// What one Alt chord flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Toggle {
    MatchCase,
    WholeWord,
    Regex,
    Hidden,
    Ignored,
}

impl Toggle {
    /// The chord table. Case, word and regex are VS Code's Alt-C/W/R;
    /// hidden and ignored are fzf-lua's Alt-H/I. The expression toggles
    /// exist where an expression is searched; the two visibility toggles
    /// wherever the tree is walked.
    pub(crate) fn bound(kind: Kind, letter: char) -> Option<Self> {
        match (letter, kind) {
            ('c', Kind::Search) => Some(Self::MatchCase),
            ('w', Kind::Search) => Some(Self::WholeWord),
            ('r', Kind::Search) => Some(Self::Regex),
            ('h', Kind::Files | Kind::Search) => Some(Self::Hidden),
            ('i', Kind::Files | Kind::Search) => Some(Self::Ignored),
            _ => None,
        }
    }

    pub(crate) fn flip(self, modes: &mut SearchModes) {
        let flag = match self {
            Self::MatchCase => &mut modes.match_case,
            Self::WholeWord => &mut modes.whole_word,
            Self::Regex => &mut modes.regex,
            Self::Hidden => &mut modes.flip_hidden,
            Self::Ignored => &mut modes.flip_ignored,
        };
        *flag = !*flag;
    }
}

/// The session's visibility policy: the configuration, flipped where a
/// toggle says so.
pub(crate) fn policy(config: &crate::config::Config, modes: SearchModes) -> SelectionPolicy {
    SelectionPolicy {
        hidden: config.search_show_hidden != modes.flip_hidden,
        respect_ignore: config.search_respect_ignore != modes.flip_ignored,
    }
}

/// Where a picker is looking and how it reads its field.
pub(crate) struct Scope<'a> {
    pub kind: Kind,
    /// The captured root, when it is not the working directory.
    pub elsewhere: Option<&'a ResourceLocation>,
    pub config: &'a crate::config::Config,
    pub modes: SearchModes,
    pub query: &'a SearchQuery,
}

/// The status row under the field: only what differs from the default
/// (0088 rule 4), in words. Empty when nothing does — then no row is
/// drawn. A qualifier typed in the query counts like a toggle: the row
/// reports what the search does, not where the instruction came from.
pub(crate) fn status_row(scope: &Scope<'_>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(root) = scope.elsewhere {
        parts.push(root.label());
    }
    if scope.kind == Kind::Search {
        parts.extend(scope.modes.expression_labels().map(str::to_owned));
    }
    let session = policy(scope.config, scope.modes);
    let hidden = scope.query.hidden.unwrap_or(session.hidden);
    if hidden != scope.config.search_show_hidden {
        parts.push(visibility("hidden", hidden));
    }
    let ignored = scope.query.ignored.unwrap_or(!session.respect_ignore);
    if ignored == scope.config.search_respect_ignore {
        // i.e. it differs from the configured default, which is the
        // negation of `search_respect_ignore`
        parts.push(visibility("ignored", ignored));
    }
    parts.join(" · ")
}

fn visibility(what: &str, shown: bool) -> String {
    format!("{what} files {}", if shown { "shown" } else { "skipped" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: Kind, modes: SearchModes, query: &str) -> String {
        status_row(&Scope {
            kind,
            elsewhere: None,
            config: &crate::config::Config::default(),
            modes,
            query: &SearchQuery::parse(query),
        })
    }

    #[test]
    fn the_default_state_has_no_row() {
        assert_eq!(row(Kind::Search, SearchModes::default(), "needle"), "");
        assert_eq!(row(Kind::Files, SearchModes::default(), ""), "");
        // a qualifier that restates the default changes nothing
        assert_eq!(
            row(Kind::Search, SearchModes::default(), "hidden:include x"),
            ""
        );
    }

    #[test]
    fn each_toggle_is_named_in_words_and_only_while_it_is_on() {
        let mut modes = SearchModes::default();
        for (letter, expected) in [
            ('c', "match case"),
            ('w', "match case · whole word"),
            ('r', "match case · whole word · regex"),
            (
                'h',
                "match case · whole word · regex · hidden files skipped",
            ),
            (
                'i',
                "match case · whole word · regex · hidden files skipped · ignored files shown",
            ),
        ] {
            Toggle::bound(Kind::Search, letter)
                .unwrap()
                .flip(&mut modes);
            assert_eq!(row(Kind::Search, modes, "needle"), expected);
        }
        Toggle::bound(Kind::Search, 'c').unwrap().flip(&mut modes);
        assert!(!row(Kind::Search, modes, "needle").contains("match case"));
    }

    #[test]
    fn the_file_finder_binds_and_reports_only_the_visibility_toggles() {
        for letter in ['c', 'w', 'r'] {
            assert_eq!(Toggle::bound(Kind::Files, letter), None);
        }
        assert_eq!(Toggle::bound(Kind::Buffers, 'h'), None);
        let modes = SearchModes {
            match_case: true,
            flip_ignored: true,
            ..Default::default()
        };
        assert_eq!(row(Kind::Files, modes, ""), "ignored files shown");
    }

    #[test]
    fn a_query_qualifier_reads_like_a_toggle_and_overrides_one() {
        assert_eq!(
            row(Kind::Search, SearchModes::default(), "ignored:include x"),
            "ignored files shown"
        );
        let flipped = SearchModes {
            flip_hidden: true,
            ..Default::default()
        };
        assert_eq!(row(Kind::Files, flipped, "hidden:include"), "");
        assert!(policy(&crate::config::Config::default(), flipped).respect_ignore);
        assert!(!policy(&crate::config::Config::default(), flipped).hidden);
    }
}
