//! `:substitute`: one parsed specification and one pure plan for preview and apply.
//!
//! Patterns use the buffer-search Vim-magic grammar. Replacement text is literal:
//! `&`, `$1`, and backreferences such as `\1` have no special meaning. Within the
//! replacement, an escaped delimiter inserts that delimiter and `\\` inserts one
//! backslash; every other backslash is preserved. In patterns only an escaped
//! delimiter loses its backslash — all other escapes belong to the query grammar.

use strop_core::{Buffer, Range, Replacement};
use strop_grammar::{CompiledQuery, SearchMatch};

use crate::editor::changes::review::render;
use crate::editor::{Editor, ReviewRow};

use super::super::resolution::ResolutionPurpose;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubstituteError {
    Incomplete(String),
    Invalid(String),
}
impl SubstituteError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Incomplete(message) | Self::Invalid(message) => message,
        }
    }
    fn is_incomplete(&self) -> bool {
        matches!(self, Self::Incomplete(_))
    }
}
impl std::fmt::Display for SubstituteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SubstituteSpec {
    pub range: (usize, usize),
    pub query: CompiledQuery,
    pub replacement: String,
    pub global: bool,
    /// Byte spans a match must lie inside; empty means anywhere. A
    /// collection substitutes inside its excerpts, never in a card border
    /// that happens to spell the pattern (0094 §3.1).
    pub within: Vec<(usize, usize)>,
}

/// Everything apply needs, plus the immutable changed-text presentation. The
/// source revision is stamped by the surrounding resolution key, not recomputed.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SubstitutionPlan {
    pub edits: Vec<Replacement>,
    pub diff: String,
    pub rows: Vec<ReviewRow>,
    pub changed_lines: Vec<usize>,
    pub scope: (usize, usize),
}

/// A ready, revision-stamped substitution preview. This borrows the prepared
/// plan: rendering never rebuilds a diff or copies all of its changed text.
pub struct SubstitutionPreview<'a> {
    pub(crate) plan: &'a SubstitutionPlan,
    pub(crate) offscreen: usize,
}
impl SubstitutionPreview<'_> {
    pub fn ranges(&self) -> impl Iterator<Item = Range> + '_ {
        self.plan.edits.iter().map(|edit| edit.range)
    }
    pub fn diff_text(&self) -> &str {
        &self.plan.diff
    }
    pub fn diff_rows(&self) -> &[ReviewRow] {
        &self.plan.rows
    }
    pub fn count(&self) -> usize {
        self.plan.edits.len()
    }
    pub fn changed_line_count(&self) -> usize {
        self.plan.changed_lines.len()
    }
    pub fn offscreen_count(&self) -> usize {
        self.offscreen
    }
    /// Inclusive, zero-based source line bounds.
    pub fn scope(&self) -> (usize, usize) {
        self.plan.scope
    }
    pub fn summary(&self) -> String {
        let count = self.count();
        let lines = self.changed_line_count();
        let (first, last) = self.scope();
        let scope = if first == last {
            format!("line {}", first + 1)
        } else {
            format!("lines {}–{}", first + 1, last + 1)
        };
        let offscreen = self.offscreen;
        format!(
            "{count} substitution{} · {offscreen} offscreen · {lines} changed line{} · {scope}",
            if count == 1 { "" } else { "s" },
            if lines == 1 { "" } else { "s" }
        )
    }
}

/// Live Ex composition is explicit about work which is not a ready plan. In
/// particular, a bad edit can never expose the preceding valid command's diff.
pub enum SubstitutePreviewState<'a> {
    Ready(SubstitutionPreview<'a>),
    Resolving,
    NoTarget,
    Incomplete(String),
    Invalid(String),
    Unavailable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingFieldMode {
    Insert,
    Normal,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingFieldEscape {
    EnterNormal,
    CancelPrompt,
}
/// Modal ownership of an Ex/search field, including an operator outside it.
pub struct PendingFieldState {
    pub mode: PendingFieldMode,
    pub outer_operator: Option<strop_grammar::Op>,
    pub next_escape: PendingFieldEscape,
}

/// Parsed address range and the remaining Ex command text.
pub(super) type ExRangeParse<'a> = (Option<(usize, usize)>, &'a str);

/// Parse the existing Ex address grammar. No address means `(None, cmdline)`;
/// malformed and backward ranges are errors rather than a different command.
pub(crate) fn parse_ex_range<'a>(
    buf: &Buffer,
    head: usize,
    cmdline: &'a str,
) -> Result<ExRangeParse<'a>, String> {
    let last = buf.last_content_line();
    let cur = buf.line_of(head);
    let addr = |tok: &str| -> Option<(usize, usize)> {
        match tok.as_bytes().first()? {
            b'%' => Some((0, 1)),
            b'.' => Some((cur, 1)),
            b'$' => Some((last, 1)),
            b if b.is_ascii_digit() => {
                let digits = tok.len() - tok.trim_start_matches(|c: char| c.is_ascii_digit()).len();
                let n: usize = tok[..digits].parse().ok()?;
                Some((n.saturating_sub(1).min(last), digits))
            }
            _ => None,
        }
    };
    let (mut first, mut used) = match addr(cmdline) {
        Some(value) => value,
        None => return Ok((None, cmdline)),
    };
    let mut second = None;
    if cmdline.as_bytes().get(used) == Some(&b',') {
        match addr(&cmdline[used + 1..]) {
            Some((line, consumed)) => {
                second = Some(line);
                used += 1 + consumed;
            }
            None => return Err("incomplete Ex range".into()),
        }
    }
    let percent = &cmdline[..used] == "%";
    let mut tail = &cmdline[used..];
    let percent_tail = tail;
    let apply_offset = |mut line: usize, tail: &str| -> (usize, usize) {
        let bytes = tail.as_bytes();
        let mut index = 0;
        while index < tail.len() && matches!(bytes[index], b'+' | b'-') {
            let negative = bytes[index] == b'-';
            index += 1;
            let digits = tail[index..].len()
                - tail[index..]
                    .trim_start_matches(|c: char| c.is_ascii_digit())
                    .len();
            let count = if digits == 0 {
                1
            } else {
                tail[index..index + digits].parse().unwrap_or(1)
            };
            index += digits;
            line = if negative {
                line.saturating_sub(count)
            } else {
                line.saturating_add(count).min(last)
            };
        }
        (line, index)
    };
    let (line, consumed) = apply_offset(first, tail);
    first = line;
    tail = &tail[consumed..];
    let mut last_line = second.unwrap_or(first);
    if second.is_some() {
        let (line, consumed) = apply_offset(last_line, tail);
        last_line = line;
        tail = &tail[consumed..];
    }
    if percent {
        if tail != percent_tail {
            return Err("invalid Ex range after %".into());
        }
        return Ok((Some((0, last)), tail));
    }
    if first > last_line {
        return Err("backward Ex range".into());
    }
    Ok((Some((first.min(last), last_line.min(last))), tail))
}

/// Whether the command after an optional range is the substitute command. Other
/// `s...` Ex commands (`set`, splits) remain owned by their existing dispatch.
pub(crate) fn is_substitute(rest: &str) -> bool {
    let Some(after_s) = rest.strip_prefix('s') else {
        return false;
    };
    after_s.is_empty() || after_s.chars().next().is_some_and(valid_delimiter)
}

fn valid_delimiter(c: char) -> bool {
    !c.is_alphanumeric() && !c.is_whitespace() && !matches!(c, '\\' | '"')
}

/// Split at an unescaped delimiter. Backslashes only affect delimiter
/// recognition here; grammar/replacement unescaping is deliberately separate.
fn segment(input: &str, delimiter: char) -> Result<(&str, &str, bool), SubstituteError> {
    let mut escaped = false;
    for (index, c) in input.char_indices() {
        if c == delimiter && !escaped {
            return Ok((&input[..index], &input[index + c.len_utf8()..], true));
        }
        if c == '\\' {
            escaped = !escaped;
        } else {
            escaped = false;
        }
    }
    Ok((input, "", false))
}

fn unescape_pattern(raw: &str, delimiter: char) -> std::borrow::Cow<'_, str> {
    let escaped_delimiter = format!("\\{delimiter}");
    if raw.contains(&escaped_delimiter) {
        std::borrow::Cow::Owned(raw.replace(&escaped_delimiter, &delimiter.to_string()))
    } else {
        std::borrow::Cow::Borrowed(raw)
    }
}

/// `:s` matches case exactly unless the pattern says otherwise (`\c`, a
/// leading `case:` directive): a substitution that also rewrites `Edge` when
/// asked for `edge` changes text nobody named (0091 B2). The directive is
/// the buffer-search grammar's own `\C`, appended.
fn exact_case(pattern: &str) -> std::borrow::Cow<'_, str> {
    let body = pattern
        .strip_prefix("\\v")
        .or_else(|| pattern.strip_prefix("\\m"))
        .unwrap_or(pattern);
    if pattern.contains("\\c") || pattern.contains("\\C") || body.starts_with("case:") {
        std::borrow::Cow::Borrowed(pattern)
    } else {
        std::borrow::Cow::Owned(format!("{pattern}\\C"))
    }
}

/// The replacement contract above is intentionally not Search's `$1` contract.
fn unescape_replacement(raw: &str, delimiter: char) -> Result<String, SubstituteError> {
    let mut output = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            output.push(c);
            continue;
        }
        let Some(&next) = chars.peek() else {
            return Err(SubstituteError::Incomplete(
                "replacement ends with an unfinished escape".into(),
            ));
        };
        chars.next();
        if next == delimiter || next == '\\' {
            output.push(next);
        } else {
            output.push('\\');
            output.push(next);
        }
    }
    Ok(output)
}

fn parse_command(range: (usize, usize), rest: &str) -> Result<SubstituteSpec, SubstituteError> {
    let Some(after_s) = rest.strip_prefix('s') else {
        panic!("substitute dispatch");
    };
    let Some(delimiter) = after_s.chars().next() else {
        return Err(SubstituteError::Incomplete(
            ":s needs a delimiter, pattern and replacement".into(),
        ));
    };
    if !valid_delimiter(delimiter) {
        return Err(SubstituteError::Invalid(
            "invalid substitute delimiter".into(),
        ));
    }
    let body = &after_s[delimiter.len_utf8()..];
    let (raw_pattern, after_pattern, terminated) = segment(body, delimiter)?;
    if !terminated {
        return Err(SubstituteError::Incomplete(
            ":s needs a delimiter after the pattern".into(),
        ));
    }
    if raw_pattern.is_empty() {
        return Err(SubstituteError::Invalid("empty pattern".into()));
    }
    let (raw_replacement, flags, closed) = segment(after_pattern, delimiter)?;
    if closed && !matches!(flags, "" | "g") {
        return Err(SubstituteError::Invalid(format!(
            "unsupported substitute flags: {flags}"
        )));
    }
    let pattern = unescape_pattern(raw_pattern, delimiter);
    let query = CompiledQuery::compile(&exact_case(&pattern), false)
        .map_err(|error| SubstituteError::Invalid(error.to_string()))?;
    let replacement = unescape_replacement(raw_replacement, delimiter)?;
    Ok(SubstituteSpec {
        range,
        query,
        replacement,
        global: flags == "g",
        within: Vec::new(),
    })
}

/// Parse an entire Ex body, or return `Ok(None)` when it is not `:substitute`.
pub(crate) fn parse(
    body: &str,
    buf: &Buffer,
    head: usize,
) -> Result<Option<SubstituteSpec>, SubstituteError> {
    let (range, rest) = parse_ex_range(buf, head, body).map_err(SubstituteError::Invalid)?;
    if !is_substitute(rest) {
        return Ok(None);
    }
    let line = buf.line_of(head);
    let range = range.unwrap_or((line, line));
    parse_command(range, rest).map(Some)
}

/// Compute replacements and their before/after diff against one immutable
/// buffer. No Editor, register, search history, undo, or mutation API is read.
pub(crate) fn plan(buf: &Buffer, spec: &SubstituteSpec) -> Result<SubstitutionPlan, String> {
    let (first, last) = spec.range;
    if first > last || last > buf.last_content_line() {
        return Err("substitution range is outside the source".into());
    }
    let start = buf.line_start(first);
    let end = buf.line_end(last);
    let mut edits = Vec::new();
    let mut changed_lines = Vec::new();
    let mut previous_line = None;
    let query = &spec.query;
    strop_grammar::search_visit(buf, query, |hit: SearchMatch| {
        let hit_start = hit.start.get();
        let hit_end = hit.end.get();
        if hit_start > end {
            return std::ops::ControlFlow::Break(());
        }
        // Matches must be wholly inside the inclusive line scope. A match
        // crossing either Ex boundary is not a partial substitution target.
        if hit_start < start || hit_end > end {
            return std::ops::ControlFlow::Continue(());
        }
        if !spec.within.is_empty() {
            let after = spec.within.partition_point(|span| span.0 <= hit_start);
            // strictly inside: the span's last byte is its line terminator
            if !after
                .checked_sub(1)
                .is_some_and(|index| hit_end < spec.within[index].1)
            {
                return std::ops::ControlFlow::Continue(());
            }
        }
        if !spec.global {
            let line = buf.line_of(hit_start);
            if previous_line == Some(line) {
                return std::ops::ControlFlow::Continue(());
            }
            previous_line = Some(line);
        }
        let hit = Range::charwise(hit_start, hit_end);
        if hit.is_empty() && spec.replacement.is_empty() {
            return std::ops::ControlFlow::Continue(());
        }
        edits.push(Replacement::new(hit, spec.replacement.clone()));
        std::ops::ControlFlow::Continue(())
    })
    .map_err(|error| error.to_string())?;
    for edit in &edits {
        let first = buf.line_of(edit.range.start);
        let end = if edit.range.is_empty() {
            edit.range.end.get()
        } else {
            edit.range.end.get() - 1
        };
        let last = buf.line_of(end);
        for line in first..=last {
            if changed_lines.last() != Some(&line) {
                changed_lines.push(line);
            }
        }
    }
    let diff = render::file_diff("substitute", buf, buf.revision(), &edits)
        .map_err(|error| error.to_string())?;
    Ok(SubstitutionPlan {
        edits,
        diff: diff.text,
        rows: diff.rows,
        changed_lines,
        scope: spec.range,
    })
}

impl Editor {
    pub(crate) fn prepare_ex_substitute(&mut self, spec: SubstituteSpec, cursors: Vec<usize>) {
        self.prepare_substitute_resolution(spec, cursors);
    }

    /// Read only the plan stamped for the exact current Ex body, source,
    /// pane and saved cursors. Parsing happens before consulting that cache.
    pub fn substitute_preview(&self) -> Option<SubstitutePreviewState<'_>> {
        let prompt = self.pending.prompt()?;
        let crate::editor::pending::PromptContext::Ex(origin) = prompt.context() else {
            return None;
        };
        if !self.pending_origin_valid(origin)
            || self.sels().extra_heads() != origin.pane.sels.extra_heads()
        {
            return Some(SubstitutePreviewState::Unavailable(
                "substitution source, cursor or focus changed".into(),
            ));
        }
        let spec = match parse(prompt.body(), self.buf(), origin.pane.sels.primary().head) {
            Ok(Some(mut spec)) => {
                spec.within = self.collection_body_spans(self.current());
                spec
            }
            Ok(None) => return None,
            Err(error) if error.is_incomplete() => {
                return Some(SubstitutePreviewState::Incomplete(error.to_string()));
            }
            Err(error) => return Some(SubstitutePreviewState::Invalid(error.to_string())),
        };
        let cursors = origin.pane.sels.heads();
        if self.substitution_is_large() && !self.substitution_ready(&spec, &cursors) {
            return Some(if self.substitution_resolving(&spec, &cursors) {
                SubstitutePreviewState::Resolving
            } else {
                SubstitutePreviewState::Unavailable("substitution resolver is unavailable".into())
            });
        }
        match self.prepared_substitution(&spec, &cursors) {
            Some(Ok(plan)) if plan.edits.is_empty() => Some(SubstitutePreviewState::NoTarget),
            Some(Ok(plan)) => {
                let top = origin.pane.view_top;
                let bottom = top.saturating_add(self.view_rows());
                let offscreen = plan
                    .changed_lines
                    .iter()
                    .filter(|&&line| line < top || line >= bottom)
                    .count();
                Some(SubstitutePreviewState::Ready(SubstitutionPreview {
                    plan,
                    offscreen,
                }))
            }
            Some(Err(error)) => Some(SubstitutePreviewState::Unavailable(error.to_owned())),
            None => Some(SubstitutePreviewState::Unavailable(
                "substitution preview is not prepared".into(),
            )),
        }
    }

    /// Execute the same parsed spec through the same plan used by the card.
    pub(crate) fn run_substitute(&mut self, range: (usize, usize), rest: &str) {
        let spec = match parse_command(range, rest) {
            Ok(mut spec) => {
                spec.within = self.collection_body_spans(self.current());
                spec
            }
            Err(error) => {
                self.message = error.to_string();
                return;
            }
        };
        let cursors = self.all_cursors();
        // `:cdo` finishes each command before it moves on (0094 §3.1).
        if !self.changes.batch_ex
            && self.substitution_is_large()
            && !self.substitution_ready(&spec, &cursors)
        {
            self.defer_substitute(spec, cursors, ResolutionPurpose::Execute);
            return;
        }
        let plan = match self.take_substitution(&spec, &cursors) {
            Some(Ok(plan)) => plan,
            Some(Err(error)) => {
                self.message = format!("substitution failed: {error}");
                return;
            }
            None => match plan(self.buf(), &spec) {
                Ok(plan) => plan,
                Err(error) => {
                    self.message = format!("substitution failed: {error}");
                    return;
                }
            },
        };
        self.apply_substitution(spec, plan);
    }

    pub(crate) fn execute_prepared_substitution(&mut self, spec: SubstituteSpec) {
        let cursors = self.all_cursors();
        match self.take_substitution(&spec, &cursors) {
            Some(Ok(plan)) => self.apply_substitution(spec, plan),
            Some(Err(error)) => self.message = format!("substitution failed: {error}"),
            None => self.message = "substitution plan invalidated before apply".into(),
        }
    }

    fn apply_substitution(&mut self, spec: SubstituteSpec, plan: SubstitutionPlan) {
        if plan.edits.is_empty() {
            let source = spec.query.source();
            self.message = format!(
                "pattern not found: {}",
                source.strip_suffix("\\C").unwrap_or(source)
            );
            return;
        }
        let hits = plan.edits.len();
        let start = self.buf().line_start(spec.range.0);
        let old_end = self.buf().line_end(spec.range.1);
        let delta = plan.edits.iter().fold(0isize, |delta, edit| {
            delta + edit.text.len() as isize - edit.range.len() as isize
        });
        let previous_jumps = self.view().jumps.clone();
        self.push_jump();
        if let Err(error) = self.apply(
            self.current(),
            self.buf().revision(),
            crate::editor::transact::ChangeSet {
                edits: plan.edits,
                undo_open: false,
            },
        ) {
            self.view_mut().jumps = previous_jumps;
            self.message = format!("substitution failed: {error}");
            return;
        }
        let end = old_end
            .saturating_add_signed(delta)
            .min(self.buf().len_bytes());
        let last_line = self.buf().line_of(end);
        let last_start = self.buf().line_start(last_line);
        self.set_head(self.buf().clamp_boundary(last_start));
        self.clamp_cursor();
        self.flash(Range::charwise(start, end));
        self.message = format!("{hits} substitution{}", if hits == 1 { "" } else { "s" });
    }

    /// Field ownership is independent of the outer walker's operator grammar.
    pub fn pending_field_state(&self) -> Option<PendingFieldState> {
        let prompt = self.pending.prompt()?;
        if !matches!(
            prompt.context(),
            crate::editor::pending::PromptContext::Ex(_)
                | crate::editor::pending::PromptContext::Search { .. }
        ) {
            return None;
        }
        let outer_operator = match prompt.context() {
            crate::editor::pending::PromptContext::Search { state, .. } => state.op,
            _ => self.walker.state.op,
        };
        let normal = prompt.normal();
        Some(PendingFieldState {
            mode: if normal {
                PendingFieldMode::Normal
            } else {
                PendingFieldMode::Insert
            },
            outer_operator,
            next_escape: if normal {
                PendingFieldEscape::CancelPrompt
            } else {
                PendingFieldEscape::EnterNormal
            },
        })
    }
}

#[cfg(test)]
#[path = "substitute/tests.rs"]
mod tests;
