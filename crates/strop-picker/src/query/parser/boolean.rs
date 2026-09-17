//! The Boolean grammar (0063 §3): recursive descent over the token
//! stream and the canonical `to_query_string` formatting. The AST types
//! themselves live in `super` next to `SearchQuery`; this module owns
//! how tokens become a tree and how a tree becomes text again.

use super::super::lexer::{self, TokenKind};
use super::{BooleanExpr, ContentAtom, MetadataAtom, SearchQuery};

/// Recursive descent over the token stream (0063 §3): precedence
/// NOT > AND > OR, juxtaposition of units is an implicit AND, and a run
/// of bare words is one phrase atom. Located diagnostics for dangling
/// operators and unbalanced groups; the parse recovers instead of
/// stopping at the first error.
pub(super) fn parse_boolean(
    tokens: &[lexer::Token],
    query: &mut SearchQuery,
) -> Option<BooleanExpr> {
    let mut parser = BooleanParser { tokens, at: 0 };
    let expr = parser.or_level(query);
    while parser.at < tokens.len() {
        let token = &tokens[parser.at];
        match &token.kind {
            TokenKind::Paren { open: false } => query.fail(
                &token.range,
                "unbalanced )".into(),
                Some("remove it or open the group with (".into()),
            ),
            TokenKind::Operator(operator) => query.fail(
                &token.range,
                format!("{} without a right operand", operator.as_str()),
                Some("add an operand after it".into()),
            ),
            _ => {}
        }
        parser.at += 1;
    }
    expr
}

struct BooleanParser<'a> {
    tokens: &'a [lexer::Token],
    at: usize,
}

impl<'a> BooleanParser<'a> {
    /// The next significant token. Query-wide options
    /// (`case:`/`hidden:`/`ignored:`) are applied in the flat pass and
    /// never become branch predicates (0063 §3), so the descent skips
    /// them; an operator next to an option diagnoses as dangling, never
    /// silently matches.
    fn peek(&mut self) -> Option<&'a lexer::Token> {
        while matches!(
            self.tokens.get(self.at).map(|token| &token.kind),
            Some(TokenKind::Qualifier { key, .. })
                if matches!(key.as_str(), "case" | "hidden" | "ignored")
        ) {
            self.at += 1;
        }
        self.tokens.get(self.at)
    }

    fn or_level(&mut self, query: &mut SearchQuery) -> Option<BooleanExpr> {
        let first = self.and_level(query)?;
        let mut branches = vec![first];
        while matches!(
            self.peek().map(|token| &token.kind),
            Some(TokenKind::Operator(lexer::Operator::Or))
        ) {
            let range = self.peek().map(|token| token.range.clone());
            self.at += 1;
            match self.and_level(query) {
                Some(branch) => branches.push(branch),
                None => {
                    if let Some(range) = range {
                        query.fail(
                            &range,
                            "OR without a right operand".into(),
                            Some("add a term after OR".into()),
                        );
                    }
                    break;
                }
            }
        }
        match branches.len() {
            1 => branches.pop(),
            _ => Some(BooleanExpr::Or(branches)),
        }
    }

    fn and_level(&mut self, query: &mut SearchQuery) -> Option<BooleanExpr> {
        let mut operands = Vec::new();
        loop {
            let joined = matches!(
                self.peek().map(|token| &token.kind),
                Some(TokenKind::Operator(lexer::Operator::And))
            );
            if joined {
                self.at += 1;
            }
            if operands.is_empty() && joined {
                // AND at operand position: a dangling operator.
                if let Some(token) = self.peek() {
                    query.fail(
                        &token.range.clone(),
                        "AND without a left operand".into(),
                        Some("start with a term".into()),
                    );
                }
            }
            let before = self.at;
            match self.unit(query) {
                Some(operand) => operands.push(operand),
                None => {
                    if joined && before < self.tokens.len() {
                        // Consumed AND, no operand followed.
                    } else if joined {
                        if let Some(token) = self.tokens.get(before.saturating_sub(1)) {
                            query.fail(
                                &token.range.clone(),
                                "AND without a right operand".into(),
                                Some("add a term after AND".into()),
                            );
                        }
                    }
                    break;
                }
            }
        }
        match operands.len() {
            0 => None,
            1 => operands.pop(),
            _ => Some(BooleanExpr::And(operands)),
        }
    }

    /// One operand: NOT-unit, group, qualifier atom, or a bare-word
    /// phrase run (adjacent words stay one phrase, 0063 §3).
    fn unit(&mut self, query: &mut SearchQuery) -> Option<BooleanExpr> {
        let token = self.peek()?;
        match &token.kind {
            TokenKind::Operator(lexer::Operator::Not) => {
                let range = token.range.clone();
                self.at += 1;
                match self.unit(query) {
                    Some(operand) => Some(BooleanExpr::Not(Box::new(operand))),
                    None => {
                        query.fail(
                            &range,
                            "NOT without an operand".into(),
                            Some("add a term after NOT".into()),
                        );
                        None
                    }
                }
            }
            TokenKind::Paren { open: true } => {
                let range = token.range.clone();
                self.at += 1;
                let inner = self.or_level(query);
                match self.peek().map(|token| &token.kind) {
                    Some(TokenKind::Paren { open: false }) => {
                        self.at += 1;
                    }
                    _ => query.fail(
                        &range,
                        "unclosed group".into(),
                        Some("close it with )".into()),
                    ),
                }
                inner
            }
            TokenKind::Paren { open: false }
            | TokenKind::Operator(lexer::Operator::And)
            | TokenKind::Operator(lexer::Operator::Or) => None,
            TokenKind::UnclosedQuote => None,
            TokenKind::Qualifier {
                key,
                value,
                negated,
                ..
            } => {
                self.at += 1;
                if value.is_empty() {
                    return None;
                }
                // `type:` is the documented input alias of `kind:` (0063 §3).
                let key = if key == "type" { "kind" } else { key };
                if key == "text" {
                    Some(BooleanExpr::Content(ContentAtom {
                        regex: false,
                        text: value.clone(),
                    }))
                } else if key == "regex" {
                    Some(BooleanExpr::Content(ContentAtom {
                        regex: true,
                        text: value.clone(),
                    }))
                } else {
                    Some(BooleanExpr::Metadata(MetadataAtom {
                        key: key.to_string(),
                        value: value.clone(),
                        negated: *negated,
                    }))
                }
            }
            TokenKind::Word { .. } => {
                let mut words = Vec::new();
                while let Some(token) = self.peek() {
                    match &token.kind {
                        TokenKind::Word { text, .. } => {
                            words.push(text.clone());
                            self.at += 1;
                        }
                        _ => break,
                    }
                }
                Some(BooleanExpr::Content(ContentAtom {
                    regex: false,
                    text: words.join(" "),
                }))
            }
        }
    }
}

impl BooleanExpr {
    /// Canonical fully-parenthesized formatting. Always re-parses to the
    /// same tree (parse-format-parse property, 0063 §6.1).
    pub fn to_query_string(&self) -> String {
        match self {
            Self::Content(atom) => {
                let prefix = if atom.regex { "regex:" } else { "" };
                // Quote anything the lexer would re-shape: whitespace,
                // quotes, parens, backslashes, colons (a bare
                // `kind:…` inside content would lex as a qualifier)
                // and the operator words themselves.
                let hostile = atom.text.is_empty()
                    || atom.text.starts_with('-')
                    || atom.text.chars().any(|c| {
                        c.is_whitespace() || matches!(c, '"' | '\'' | '(' | ')' | '\\' | ':')
                    })
                    || lexer::Operator::from_word(&atom.text).is_some();
                if hostile {
                    let escaped = atom.text.replace('\\', "\\\\").replace('"', "\\\"");
                    format!("{prefix}\"{escaped}\"")
                } else {
                    format!("{prefix}{}", atom.text)
                }
            }
            Self::Metadata(atom) => {
                let negation = if atom.negated { "-" } else { "" };
                let hostile = atom.value.is_empty()
                    || atom.value.starts_with('-')
                    || atom.value.chars().any(|c| {
                        c.is_whitespace() || matches!(c, '"' | '\'' | '(' | ')' | '\\' | ':')
                    });
                if hostile {
                    let escaped = atom.value.replace('\\', "\\\\").replace('"', "\\\"");
                    format!("{negation}{}:\"{escaped}\"", atom.key)
                } else {
                    format!("{negation}{}:{}", atom.key, atom.value)
                }
            }
            Self::Not(inner) => format!("NOT ({})", inner.to_query_string()),
            Self::And(operands) => format!(
                "({})",
                operands
                    .iter()
                    .map(BooleanExpr::to_query_string)
                    .collect::<Vec<_>>()
                    .join(" AND ")
            ),
            Self::Or(branches) => format!(
                "({})",
                branches
                    .iter()
                    .map(BooleanExpr::to_query_string)
                    .collect::<Vec<_>>()
                    .join(" OR ")
            ),
        }
    }
}
