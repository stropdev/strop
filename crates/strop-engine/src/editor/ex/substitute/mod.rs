//! `:substitute`: one parsed specification and one pure plan for preview,
//! apply and confirm.
//!
//! Patterns use the buffer-search Vim-magic grammar and `/`'s case policy
//! (`\c` / `\C` and the `i` / `I` flags override it).
//! The replacement is Vim's (`replace.rs`): `&`, `\1`, `~`, `\r`, `\u`….

mod confirm;
mod parse;
mod replace;
#[cfg(test)]
mod tests;

use strop_core::{Buffer, Range, Replacement};
use strop_grammar::{CompiledQuery, SearchMatch};

use crate::editor::ex::*;
use crate::editor::resolution::ResolutionPurpose;
use crate::editor::Editor;

pub(crate) use confirm::*;
pub(crate) use parse::*;

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

/// The flags after the replacement (`:help :s_flags`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Flags {
    /// `g`: every match on a line, not the first.
    pub global: bool,
    /// `c`: ask at each match.
    pub confirm: bool,
    /// `n`: count the matches, change nothing.
    pub count_only: bool,
    /// `e`: no match is not an error.
    pub quiet: bool,
    /// `i` (`Some(true)`) / `I` (`Some(false)`).
    pub case: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SubstituteSpec {
    /// Inclusive, zero-based lines.
    pub range: (usize, usize),
    pub query: CompiledQuery,
    /// The pattern as typed (messages, `:&`).
    pub pattern: String,
    /// The replacement string, `~` expanded, specials unparsed.
    pub replacement: String,
    pub flags: Flags,
    /// Byte spans a match must lie inside; empty means anywhere. A
    /// collection substitutes inside its excerpts, never in a card border
    /// that happens to spell the pattern (0094 §3.1).
    pub within: Vec<(usize, usize)>,
}

/// What `:&`, `&`, `g&` and `~` repeat.
#[derive(Debug, Clone)]
pub(crate) struct LastSubstitute {
    pub pattern: String,
    pub replacement: String,
    pub flags: Flags,
}

/// Everything apply needs, plus each changed line as it will read (the
/// in-place preview, built once with the plan). The source revision is
/// stamped by the surrounding resolution key, not recomputed.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SubstitutionPlan {
    pub edits: Vec<Replacement>,
    /// Ascending, one per source line a replacement starts on.
    pub lines: Vec<PreviewLine>,
    pub changed_lines: Vec<usize>,
    pub scope: (usize, usize),
}

/// One source line as `Enter` will leave it: its text with the
/// replacements made (a line break drawn `↵`), and where the new text is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PreviewLine {
    /// Zero-based source line.
    pub line: usize,
    pub text: String,
    /// Byte ranges of `text` that are replacement text.
    pub added: Vec<(usize, usize)>,
}

/// A ready, revision-stamped substitution preview. This borrows the prepared
/// plan: rendering never rebuilds a diff or copies all of its changed text.
pub struct SubstitutionPreview<'a> {
    pub(crate) plan: &'a SubstitutionPlan,
    /// Changed lines above and below the view.
    pub(crate) above: usize,
    pub(crate) below: usize,
}
impl SubstitutionPreview<'_> {
    pub fn ranges(&self) -> impl Iterator<Item = Range> + '_ {
        self.plan.edits.iter().map(|edit| edit.range)
    }
    /// `line` as it will read, when a replacement starts on it.
    pub fn line(&self, line: usize) -> Option<&PreviewLine> {
        let at = self
            .plan
            .lines
            .binary_search_by_key(&line, |preview| preview.line)
            .ok()?;
        self.plan.lines.get(at)
    }
    pub fn count(&self) -> usize {
        self.plan.edits.len()
    }
    pub fn changed_line_count(&self) -> usize {
        self.plan.changed_lines.len()
    }
    /// Changed lines out of view: above it, below it.
    pub fn offscreen(&self) -> (usize, usize) {
        (self.above, self.below)
    }
    /// Inclusive, zero-based source line bounds.
    pub fn scope(&self) -> (usize, usize) {
        self.plan.scope
    }
    /// `4 changes · 1 below`: what the command line's right side says.
    pub fn summary(&self) -> String {
        let count = self.count();
        let mut summary = format!("{count} change{}", if count == 1 { "" } else { "s" });
        if self.above > 0 {
            summary.push_str(&format!(" · {} above", self.above));
        }
        if self.below > 0 {
            summary.push_str(&format!(" · {} below", self.below));
        }
        summary
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

/// Compute replacements and their before/after diff against one immutable
/// buffer. No Editor, register, search history, undo, or mutation API is read.
pub(crate) fn plan(buf: &Buffer, spec: &SubstituteSpec) -> Result<SubstitutionPlan, String> {
    let (first, last) = spec.range;
    if first > last || last > buf.last_content_line() {
        return Err("substitution range is outside the source".into());
    }
    let start = buf.line_start(first);
    let end = buf.line_end(last);
    let template = replace::Template::parse(&spec.replacement);
    let mut edits = Vec::new();
    let mut changed_lines = Vec::new();
    let mut previous_line = None;
    // the search starts at the range: `:s` inside `:g` costs its line,
    // not the text above it
    strop_grammar::search_visit_captures(buf, &spec.query, start, |hit: SearchMatch, groups| {
        let hit_start = hit.start.get();
        if hit_start > end {
            return std::ops::ControlFlow::Break(());
        }
        // A match belongs to the line it starts on (Vim): one that starts
        // inside the range may run past it; the search began at the
        // range, so none starts above it.
        debug_assert!(hit_start >= start);
        if !spec.within.is_empty() {
            let after = spec.within.partition_point(|span| span.0 <= hit_start);
            // strictly inside: the span's last byte is its line terminator
            if !after
                .checked_sub(1)
                .is_some_and(|index| hit.end.get() < spec.within[index].1)
            {
                return std::ops::ControlFlow::Continue(());
            }
        }
        if !spec.flags.global {
            let line = buf.line_of(hit_start);
            if previous_line == Some(line) {
                return std::ops::ControlFlow::Continue(());
            }
            previous_line = Some(line);
        }
        let text = template.expand(buf, hit, groups);
        if hit.is_empty() && text.is_empty() {
            return std::ops::ControlFlow::Continue(());
        }
        edits.push(Replacement::new(
            Range::charwise(hit_start, hit.end.get()),
            text,
        ));
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
    Ok(SubstitutionPlan {
        lines: preview_lines(buf, &edits),
        edits,
        changed_lines,
        scope: spec.range,
    })
}

/// Each line a replacement starts on, as it will read. A replacement that
/// runs past the line's end takes the line break with it (`↵` in the
/// new text says where one is, in either direction).
fn preview_lines(buf: &Buffer, edits: &[Replacement]) -> Vec<PreviewLine> {
    let mut lines: Vec<PreviewLine> = Vec::new();
    let mut at = 0;
    while at < edits.len() {
        let line = buf.line_of(edits[at].range.start.get());
        let start = buf.line_start(line);
        let end = buf.line_end(line);
        let mut text = String::new();
        let mut added = Vec::new();
        let mut copied = start;
        while at < edits.len() && buf.line_of(edits[at].range.start.get()) == line {
            let edit = &edits[at];
            let from = edit.range.start.get().max(copied);
            text.push_str(&buf.text().byte_slice(copied..from.min(end)).to_string());
            let first = text.len();
            text.push_str(&edit.text.replace(['\r', '\n'], "↵"));
            if edit.range.end.get() > end {
                text.push('↵');
            }
            added.push((first, text.len()));
            copied = edit.range.end.get().max(copied);
            at += 1;
        }
        if copied < end {
            text.push_str(&buf.text().byte_slice(copied..end).to_string());
        }
        lines.push(PreviewLine { line, text, added });
    }
    lines
}

/// Read only the plan stamped for the exact current Ex body, source,
/// pane and saved cursors. Parsing happens before consulting that cache.
pub(crate) fn substitute_preview(editor: &Editor) -> Option<SubstitutePreviewState<'_>> {
    let prompt = editor.cmdline.line()?;
    let crate::editor::cmdline::Context::Ex(origin) = prompt.context() else {
        return None;
    };
    if !editor.cmdline_origin_valid(origin)
        || editor.sels().extra_heads() != origin.pane.sels.extra_heads()
    {
        return Some(SubstitutePreviewState::Unavailable(
            "substitution source, cursor or focus changed".into(),
        ));
    }
    // parsed once per change of the line (cmdline/parsed.rs)
    let spec = match editor.cmdline.parsed.substitute.as_ref()? {
        Ok(spec) => spec,
        Err(error) if error.is_incomplete() => {
            return Some(SubstitutePreviewState::Incomplete(error.to_string()));
        }
        Err(error) => return Some(SubstitutePreviewState::Invalid(error.to_string())),
    };
    let cursors = origin.pane.sels.heads();
    if editor.substitution_is_large() && !editor.substitution_ready(spec, &cursors) {
        return Some(if editor.substitution_resolving(spec, &cursors) {
            SubstitutePreviewState::Resolving
        } else {
            SubstitutePreviewState::Unavailable("substitution resolver is unavailable".into())
        });
    }
    match editor.prepared_substitution(spec, &cursors) {
        Some(Ok(plan)) if plan.edits.is_empty() => Some(SubstitutePreviewState::NoTarget),
        Some(Ok(plan)) => {
            let top = origin.pane.view_top;
            let bottom = top.saturating_add(editor.view_rows());
            let above = plan.changed_lines.partition_point(|&line| line < top);
            let below = plan.changed_lines.len()
                - plan.changed_lines.partition_point(|&line| line < bottom);
            Some(SubstitutePreviewState::Ready(SubstitutionPreview {
                plan,
                above,
                below,
            }))
        }
        Some(Err(error)) => Some(SubstitutePreviewState::Unavailable(error.to_owned())),
        None => Some(SubstitutePreviewState::Unavailable(
            "substitution preview is not prepared".into(),
        )),
    }
}

/// `:s`, `:&`, `:&&`, `:~`: execute the same parsed spec through the
/// same plan the card showed.
pub(crate) fn ex_substitute(editor: &mut Editor, call: &super::ExCall<'_>) -> super::ExResult {
    let Some(form) = Form::of(call.name) else {
        return Ok(());
    };
    let line = editor.buf().line_of(editor.head());
    let spec = match parse_substitute(editor, form, call.range, line, call.arg) {
        Ok(spec) => spec,
        Err(error) => return Err(error.to_string()),
    };
    if spec.flags.confirm && editor.cmdline.ex.compound > 0 {
        // The question would have to stop a `:g` or `:normal` in the
        // middle of its one undo step.
        return Err("the c flag cannot ask inside :g or :normal".into());
    }
    remember_substitute(editor, &spec);
    // Vim: the substitute pattern is the search pattern from here on.
    if let Ok(query) = CompiledQuery::compile(&spec.pattern, false) {
        editor.last_search = Some(crate::editor::LastSearch {
            query,
            backward: false,
        });
    }
    let cursors = editor.all_cursors();
    // A compound command (`:g`, `:normal`) and a confirmation need the
    // answer now; a lone `:s` on a large buffer resolves off the input
    // path.
    let inline = editor.cmdline.ex.compound > 0 || spec.flags.confirm;
    if !inline && editor.substitution_is_large() && !editor.substitution_ready(&spec, &cursors) {
        editor.defer_substitute(spec, cursors, ResolutionPurpose::Execute);
        return Ok(());
    }
    let plan = match editor.take_substitution(&spec, &cursors) {
        Some(Ok(plan)) => plan,
        Some(Err(error)) => {
            return Err(format!("substitution failed: {error}"));
        }
        None => match plan(editor.buf(), &spec) {
            Ok(plan) => plan,
            Err(error) => return Err(format!("substitution failed: {error}")),
        },
    };
    apply_substitution(editor, spec, plan)
}

pub(crate) fn execute_prepared_substitution(editor: &mut Editor, spec: SubstituteSpec) {
    let cursors = editor.all_cursors();
    match editor.take_substitution(&spec, &cursors) {
        Some(Ok(plan)) => {
            if let Err(error) = apply_substitution(editor, spec, plan) {
                editor.fail(error);
            }
        }
        Some(Err(error)) => editor.fail(format!("substitution failed: {error}")),
        None => editor.fail("substitution plan invalidated before apply".into()),
    }
}

fn apply_substitution(
    editor: &mut Editor,
    spec: SubstituteSpec,
    plan: SubstitutionPlan,
) -> super::ExResult {
    if plan.edits.is_empty() {
        if !spec.flags.quiet {
            return Err(format!("pattern not found: {}", spec.pattern));
        }
        return Ok(());
    }
    let hits = plan.edits.len();
    let lines = plan.changed_lines.len();
    if spec.flags.count_only {
        editor.message = format!(
            "{hits} match{} on {lines} line{}",
            if hits == 1 { "" } else { "es" },
            if lines == 1 { "" } else { "s" }
        );
        return Ok(());
    }
    if spec.flags.confirm {
        begin_confirm(editor, plan);
        return Ok(());
    }
    let start = editor.buf().line_start(spec.range.0);
    let old_end = editor.buf().line_end(spec.range.1);
    // The cursor lands on the last line a substitution touched: the
    // end of the last replacement, in the text as it will be.
    let (mut delta, mut landing) = (0isize, 0usize);
    for edit in &plan.edits {
        delta += edit.text.len() as isize - edit.range.len() as isize;
        landing = edit.range.end.get().saturating_add_signed(delta);
    }
    let previous_jumps = editor.view().jumps.clone();
    editor.push_jump();
    if let Err(error) = editor.apply(
        editor.current(),
        editor.buf().revision(),
        crate::editor::transact::ChangeSet {
            edits: plan.edits,
            undo_open: false,
        },
    ) {
        editor.view_mut().jumps = previous_jumps;
        return Err(format!("substitution failed: {error}"));
    }
    let end = old_end
        .saturating_add_signed(delta)
        .min(editor.buf().len_bytes());
    ex_land(
        editor,
        editor.buf().line_of(landing.min(editor.buf().len_bytes())),
    );
    editor.flash(Range::charwise(start, end));
    editor.message = format!(
        "{hits} substitution{} on {lines} line{}",
        if hits == 1 { "" } else { "s" },
        if lines == 1 { "" } else { "s" }
    );
    Ok(())
}
