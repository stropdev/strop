//! Width degradation and measurement for the gathered modeline.

use crate::render::clip::{clip_end, clip_start, width};

use super::{chip, Modeline, GAP, SEP_W};

/// Smallest fragment still worth its cells (a directory hint, a name,
/// a position in the degenerate layout).
const MIN_GROUP: usize = 4;
/// A spec shorter than this is noise, not information.
const MIN_SPEC: usize = 3;

impl Modeline {
    /// Narrow the row until it fits `cells`. Each step frees the least
    /// essential thing still on screen; the first step that makes it
    /// fit wins, so a wide terminal loses nothing.
    pub(super) fn fit(&mut self, cells: usize) {
        if self.used() <= cells {
            return;
        }
        self.staged_mark = false;
        // The directory yields before anything else — shrunk towards
        // its tail (the parent closest to the file is the context
        // worth keeping), but never below a smallest useful fragment.
        // The filename is never the thing that gives way.
        let overflow = self.used().saturating_sub(cells);
        let budget = MIN_GROUP.max(width(&self.dir).saturating_sub(overflow));
        if width(&self.dir) > budget {
            self.dir = clip_start(&self.dir, budget).into_owned();
        }
        if self.used() <= cells {
            return;
        }
        self.dir.clear();
        if self.used() <= cells {
            return;
        }
        self.git_context.clear();
        if self.used() <= cells {
            return;
        }
        self.percent = None;
        if self.used() <= cells {
            return;
        }
        self.indent.clear();
        if self.used() <= cells {
            return;
        }
        self.show_diag = false;
        if self.used() <= cells {
            return;
        }
        // Whole chords or nothing (rule 10): hints leave from the end.
        while self.hints.pop().is_some() {
            if self.used() <= cells {
                return;
            }
        }
        self.attention.clear();
        if self.used() <= cells {
            return;
        }
        self.recording = false;
        if self.used() <= cells {
            return;
        }
        self.elastic(cells);
    }

    /// The incompressible ends are the mode chip and the position;
    /// everything optional is already gone. What is left is shared
    /// between the filename and the spec — the filename first, but
    /// capped at two thirds so a huge name cannot starve the grammar
    /// footer (0032: a long path must not crowd out live state).
    fn elastic(&mut self, cells: usize) {
        let fixed = chip::WIDTH
            + GAP
            + self.position_width()
            + self
                .services
                .as_ref()
                .map_or(0, |badge| SEP_W + badge.width());
        let file_sep = if self.has_file() { SEP_W } else { 0 };
        let spec_sep = if self.spec.is_empty() { 0 } else { SEP_W };
        let Some(room) = cells.checked_sub(fixed + file_sep + spec_sep) else {
            return self.ultra(cells);
        };
        if self.spec.is_empty() {
            self.fit_file(room);
            return;
        }
        let file_budget = self.file_w().min(room).min((room * 2 / 3).max(MIN_GROUP));
        self.fit_file(file_budget);
        let spec_budget = room - file_budget;
        if spec_budget >= MIN_SPEC {
            self.spec = clip_end(&self.spec, spec_budget).into_owned();
        } else {
            self.spec.clear();
            self.elastic(cells);
        }
    }

    /// Spend `budget` cells on the file group: the filename first
    /// (clipped from the end, whole graphemes only), then whichever
    /// signals still fit behind it.
    fn fit_file(&mut self, budget: usize) {
        let reserved = 2 * usize::from(self.dirty)
            + 5 * usize::from(self.readonly)
            + self.multicursor.as_ref().map_or(0, |mark| 1 + width(mark));
        let name_budget = budget.saturating_sub(reserved);
        if !self.namespace.is_empty() {
            let namespace_budget = name_budget.saturating_sub(width(&self.name).min(MIN_GROUP) + 1);
            self.namespace = clip_end(&self.namespace, namespace_budget).into_owned();
        }
        let name_budget = name_budget.saturating_sub(self.namespace_width());
        self.name = clip_end(&self.name, name_budget).into_owned();
    }

    /// Degenerate widths: not even the chip, a separator and the
    /// position fit. Show the chip and the position alone, splitting
    /// the row between them — never an empty row. Safety marks (a
    /// failed service, dirty, read-only) ride inside the chip.
    fn ultra(&mut self, cells: usize) {
        let safety = format!(
            "{}{}{}{}",
            self.services.as_ref().map_or("", |_| "!"),
            if self.dirty { "*" } else { "" },
            if self.readonly { "RO" } else { "" },
            self.multicursor.as_deref().unwrap_or("")
        );
        if !safety.is_empty() {
            self.chip = format!("{safety} {}", self.chip.trim());
        }
        self.git_context.clear();
        self.namespace.clear();
        self.dir.clear();
        self.name.clear();
        self.spec.clear();
        self.hints.clear();
        self.attention.clear();
        self.recording = false;
        self.dirty = false;
        self.readonly = false;
        self.multicursor = None;
        self.percent = None;
        self.show_diag = false;
        self.indent.clear();
        self.bare = true;
        let pos_budget = self.position_width().min((cells / 2).max(MIN_GROUP));
        self.bare_chip = cells.saturating_sub(pos_budget);
    }

    /// `current/total` search position, then `line:col` and, while there
    /// is room, the content percentage (1-based line and byte column).
    pub(super) fn position(&self) -> String {
        let search = self
            .search
            .map(|(current, total)| format!("{current}/{total}  "))
            .unwrap_or_default();
        match self.percent {
            Some(percent) => format!("{search}{}:{}  {}%", self.line, self.col, percent),
            None => format!("{search}{}:{}", self.line, self.col),
        }
    }

    fn position_width(&self) -> usize {
        self.search.map_or(0, |(current, total)| {
            digits(current) + 1 + digits(total) + 2
        }) + digits(self.line)
            + 1
            + digits(self.col)
            + self.percent.map_or(0, |percent| 3 + digits(percent))
    }

    fn hints_width(&self) -> usize {
        if self.hints.is_empty() {
            return 0;
        }
        self.hints
            .iter()
            .enumerate()
            .map(|(index, (key, desc))| {
                (if index == 0 { 1 } else { 2 }) + width(key) + 2 + 1 + width(desc)
            })
            .sum::<usize>()
            + 1
    }

    /// Measure the visible groups without building or allocating spans.
    /// Mirrors `halves` exactly: every group costs its text plus a
    /// separator when another group precedes it.
    fn used(&self) -> usize {
        let mut left = width(&self.chip) + if self.recording { 4 } else { 0 };
        if !self.git_context.is_empty() {
            left += SEP_W
                + width(&self.git_context)
                + usize::from(self.worktree_dirty)
                + usize::from(self.staged_mark);
        }
        if self.has_file() {
            left += SEP_W + self.file_w();
        }
        let mut right = Groups::default();
        if !self.spec.is_empty() {
            right.add(width(&self.spec));
        }
        for badge in &self.attention {
            right.add(width(badge));
        }
        if !self.hints.is_empty() {
            // the hint block carries its own leading gap
            right.cells += self.hints_width();
            right.count += 1;
        }
        if let Some(badge) = &self.services {
            right.add(badge.width());
        }
        if self.show_diag {
            right.add(
                if self.errors > 0 {
                    1 + digits(self.errors)
                } else {
                    0
                } + if self.warnings > 0 {
                    1 + digits(self.warnings)
                } else {
                    0
                } + usize::from(self.errors > 0 && self.warnings > 0),
            );
        }
        if !self.indent.is_empty() {
            right.add(width(&self.indent));
        }
        right.add(self.position_width());
        left + GAP + right.cells
    }

    fn namespace_width(&self) -> usize {
        if self.namespace.is_empty() {
            0
        } else {
            width(&self.namespace) + 1
        }
    }

    fn file_w(&self) -> usize {
        self.namespace_width()
            + width(&self.dir)
            + width(&self.name)
            + 2 * usize::from(self.dirty)
            + 5 * usize::from(self.readonly)
            + self.multicursor.as_ref().map_or(0, |mark| 1 + width(mark))
    }
}

/// Right-half measurement: every group after the first costs a separator.
#[derive(Default)]
struct Groups {
    cells: usize,
    count: usize,
}

impl Groups {
    fn add(&mut self, cells: usize) {
        if self.count > 0 {
            self.cells += SEP_W;
        }
        self.cells += cells;
        self.count += 1;
    }
}

fn digits(value: usize) -> usize {
    value
        .checked_ilog10()
        .map_or(1, |digits| digits as usize + 1)
}
