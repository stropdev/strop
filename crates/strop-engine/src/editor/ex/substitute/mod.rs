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

use crate::editor::changes::review::render;
use crate::editor::ex::*;
use crate::editor::resolution::ResolutionPurpose;
use crate::editor::{Editor, ReviewRow};

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
}

/// What `:&`, `&`, `g&` and `~` repeat.
#[derive(Debug, Clone)]
pub(crate) struct LastSubstitute {
    pub pattern: String,
    pub replacement: String,
    pub flags: Flags,
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
    strop_grammar::search_visit_captures(buf, &spec.query, |hit: SearchMatch, groups| {
        let hit_start = hit.start.get();
        if hit_start > end {
            return std::ops::ControlFlow::Break(());
        }
        // A match belongs to the line it starts on (Vim): one that starts
        // inside the range may run past it, one that starts above may not
        // reach in.
        if hit_start < start {
            return std::ops::ControlFlow::Continue(());
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

/// Read only the plan stamped for the exact current Ex body, source,
/// pane and saved cursors. Parsing happens before consulting that cache.
pub(crate) fn substitute_preview(editor: &Editor) -> Option<SubstitutePreviewState<'_>> {
    let prompt = editor.pending.prompt()?;
    let crate::editor::pending::PromptContext::Ex(origin) = prompt.context() else {
        return None;
    };
    if !editor.pending_origin_valid(origin)
        || editor.sels().extra_heads() != origin.pane.sels.extra_heads()
    {
        return Some(SubstitutePreviewState::Unavailable(
            "substitution source, cursor or focus changed".into(),
        ));
    }
    let spec = match parse_substitute_line(editor, prompt.body(), origin.pane.sels.primary().head) {
        Ok(Some(spec)) => spec,
        Ok(None) => return None,
        Err(error) if error.is_incomplete() => {
            return Some(SubstitutePreviewState::Incomplete(error.to_string()));
        }
        Err(error) => return Some(SubstitutePreviewState::Invalid(error.to_string())),
    };
    let cursors = origin.pane.sels.heads();
    if editor.substitution_is_large() && !editor.substitution_ready(&spec, &cursors) {
        return Some(if editor.substitution_resolving(&spec, &cursors) {
            SubstitutePreviewState::Resolving
        } else {
            SubstitutePreviewState::Unavailable("substitution resolver is unavailable".into())
        });
    }
    match editor.prepared_substitution(&spec, &cursors) {
        Some(Ok(plan)) if plan.edits.is_empty() => Some(SubstitutePreviewState::NoTarget),
        Some(Ok(plan)) => {
            let top = origin.pane.view_top;
            let bottom = top.saturating_add(editor.view_rows());
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

/// `:s`, `:&`, `:&&`, `:~`: execute the same parsed spec through the
/// same plan the card showed.
pub(crate) fn ex_substitute(editor: &mut Editor, call: &super::ExCall<'_>) {
    let Some(form) = Form::of(call.name) else {
        return;
    };
    let line = editor.buf().line_of(editor.head());
    let spec = match parse_substitute(editor, form, call.range, line, call.arg) {
        Ok(spec) => spec,
        Err(error) => {
            editor.message = error.to_string();
            return;
        }
    };
    if spec.flags.confirm && editor.pending.ex.compound > 0 {
        // The question would have to stop a `:g` or `:normal` in the
        // middle of its one undo step.
        editor.message = "the c flag cannot ask inside :g or :normal".into();
        return;
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
    let inline = editor.pending.ex.compound > 0 || spec.flags.confirm;
    if !inline && editor.substitution_is_large() && !editor.substitution_ready(&spec, &cursors) {
        editor.defer_substitute(spec, cursors, ResolutionPurpose::Execute);
        return;
    }
    let plan = match editor.take_substitution(&spec, &cursors) {
        Some(Ok(plan)) => plan,
        Some(Err(error)) => {
            editor.message = format!("substitution failed: {error}");
            return;
        }
        None => match plan(editor.buf(), &spec) {
            Ok(plan) => plan,
            Err(error) => {
                editor.message = format!("substitution failed: {error}");
                return;
            }
        },
    };
    apply_substitution(editor, spec, plan);
}

pub(crate) fn execute_prepared_substitution(editor: &mut Editor, spec: SubstituteSpec) {
    let cursors = editor.all_cursors();
    match editor.take_substitution(&spec, &cursors) {
        Some(Ok(plan)) => apply_substitution(editor, spec, plan),
        Some(Err(error)) => editor.message = format!("substitution failed: {error}"),
        None => editor.message = "substitution plan invalidated before apply".into(),
    }
}

fn apply_substitution(editor: &mut Editor, spec: SubstituteSpec, plan: SubstitutionPlan) {
    if plan.edits.is_empty() {
        if !spec.flags.quiet {
            editor.message = format!("pattern not found: {}", spec.pattern);
        }
        return;
    }
    let hits = plan.edits.len();
    let lines = plan.changed_lines.len();
    if spec.flags.count_only {
        editor.message = format!(
            "{hits} match{} on {lines} line{}",
            if hits == 1 { "" } else { "es" },
            if lines == 1 { "" } else { "s" }
        );
        return;
    }
    if spec.flags.confirm {
        begin_confirm(editor, plan);
        return;
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
        editor.message = format!("substitution failed: {error}");
        return;
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
}
