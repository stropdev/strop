//! Width degradation and measurement for the gathered modeline.

use ratatui::style::Color;
use strop_engine::editor::feedback::FeedbackSeverity;
use strop_lsp::Severity;

use crate::render::clip::{clip_end, clip_start, width};
use crate::render::severity_color;

use super::{Modeline, GAP, SEP_W};

/// Smallest fragment still worth its cells (a directory hint, a name,
/// a position in the degenerate layout).
const MIN_GROUP: usize = 4;
/// A transient status shorter than this is noise, not information.
const MIN_STATUS: usize = 3;
/// Terminal input/return and attention cues survive optional status prose.
const TERMINAL_STATUS_MIN: usize = 14;
/// Wide rows retain the complete pinned-output Refresh action as well.
const TERMINAL_STATUS_WIDE: usize = 38;

impl Modeline {
    /// Narrow the row until it fits `cells`. Each step frees the least
    /// essential thing still on screen; the first step that makes it
    /// fit wins, so a wide terminal loses nothing.
    pub(super) fn fit(&mut self, cells: usize) {
        if self.used() <= cells {
            return;
        }
        self.staged_mark = false;
        if self.terminal && self.used() > cells {
            let overflow = self.used() - cells;
            let essential = self.terminal_status_reserve(cells);
            let budget = width(&self.status).saturating_sub(overflow).max(essential);
            self.status = clip_end(&self.status, budget).into_owned();
            if self.used() <= cells {
                return;
            }
        }
        if self.used() <= cells {
            return;
        }
        // The directory yields before anything else — shrunk towards
        // its tail (the parent closest to the file is the context
        // worth keeping), but never below a smallest useful fragment.
        // The filename is never the thing that gives way.
        let overflow = self.used() - cells;
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
        self.services.drop_ready();
        if self.used() <= cells {
            return;
        }
        self.percent = None;
        if self.used() <= cells {
            return;
        }
        self.services.drop_starting();
        if self.used() <= cells {
            return;
        }
        self.show_diag = false;
        if self.used() <= cells {
            return;
        }
        // R06's essential tail is mode · indentation · position: the
        // indent segment yields only to the incompressible ends.
        self.indent.clear();
        if self.used() <= cells {
            return;
        }
        self.elastic(cells);
    }

    fn has_status(&self) -> bool {
        self.compact_feedback || !self.status.is_empty()
    }

    fn status_width(&self) -> usize {
        if self.compact_feedback {
            1
        } else {
            width(&self.status)
        }
    }

    fn compact_status(&mut self) {
        self.status.clear();
        self.compact_feedback = matches!(
            self.feedback,
            Some(FeedbackSeverity::Warning | FeedbackSeverity::Failure)
        );
    }

    pub(super) fn feedback_color(&self) -> Color {
        match self.feedback {
            Some(FeedbackSeverity::Warning) => severity_color(Severity::Warning),
            Some(FeedbackSeverity::Failure | FeedbackSeverity::Info) | None => {
                severity_color(Severity::Error)
            }
        }
    }

    fn terminal_status_reserve(&self, cells: usize) -> usize {
        if cells >= 80 && self.terminal_prefix {
            return width(&self.status);
        }
        let reserve = if cells >= 80 && self.terminal_attention {
            TERMINAL_STATUS_WIDE
        } else {
            TERMINAL_STATUS_MIN
        };
        reserve.min(width(&self.status))
    }

    /// The incompressible ends are the mode accent and the position;
    /// everything optional is already gone. What is left is shared
    /// between the filename and the transient status — the filename
    /// first, but capped at two thirds so a huge name cannot starve
    /// live activity (0032: a long path must not crowd out the
    /// status).
    fn elastic(&mut self, cells: usize) {
        let chip_w = 1 + width(&self.chip);
        let pos_w = self.position_width();
        let services_w = if self.services.is_empty() {
            0
        } else {
            SEP_W + self.services.width()
        };
        if self.has_status() {
            let Some(room) = cells.checked_sub(chip_w + SEP_W + GAP + SEP_W + pos_w + services_w)
            else {
                if self.services.compact() {
                    return self.elastic(cells);
                }
                if self.compact_feedback {
                    return self.ultra(cells);
                }
                self.compact_status();
                return self.elastic(cells);
            };
            let file_budget = self
                .file_w()
                .min(room)
                .min((room * 2 / 3).max(MIN_GROUP))
                .min(if self.terminal && !self.compact_feedback {
                    room.saturating_sub(self.terminal_status_reserve(cells))
                        .max(MIN_GROUP.min(room))
                } else {
                    room
                })
                .min(room);
            self.fit_file(file_budget);
            let status_budget = room - file_budget;
            if self.compact_feedback {
                if status_budget == 0 {
                    return self.ultra(cells);
                }
                return;
            }
            if status_budget >= MIN_STATUS {
                self.status = clip_end(&self.status, status_budget).into_owned();
            } else {
                self.compact_status();
                return self.elastic(cells);
            }
            return;
        }
        let Some(room) = cells.checked_sub(chip_w + SEP_W + GAP + pos_w + services_w) else {
            if self.services.compact() {
                return self.elastic(cells);
            }
            return self.ultra(cells);
        };
        self.fit_file(room);
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

    /// Degenerate widths: not even the accent, a separator and the
    /// position fit. Show the accent and the position alone, splitting
    /// the row between them — never an empty row.
    fn ultra(&mut self, cells: usize) {
        self.git_context.clear();
        self.namespace.clear();
        // Even at degenerate widths, put safety before decorative mode text.
        let safety = format!(
            "{}{}{}{}",
            self.services.problem_mark(),
            if self.dirty { "*" } else { "" },
            if self.readonly { "RO" } else { "" },
            self.multicursor.as_deref().unwrap_or("")
        );
        if !safety.is_empty() {
            self.chip = format!("{safety} {}", self.chip.trim());
        }
        self.dir.clear();
        self.name.clear();
        self.compact_status();
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
            .map(|(current, total)| format!("{current}/{total} "))
            .unwrap_or_default();
        match self.percent {
            Some(percent) => format!("{search}{}:{} {}%", self.line, self.col, percent),
            None => format!("{search}{}:{}", self.line, self.col),
        }
    }

    fn position_width(&self) -> usize {
        self.search.map_or(0, |(current, total)| {
            digits(current) + 1 + digits(total) + 1
        }) + digits(self.line)
            + 1
            + digits(self.col)
            + self.percent.map_or(0, |percent| 2 + digits(percent))
    }

    /// Measure the visible groups without building or allocating spans.
    fn used(&self) -> usize {
        let mut left = 1 + width(&self.chip);
        if !self.git_context.is_empty() {
            left += SEP_W
                + width(&self.git_context)
                + usize::from(self.worktree_dirty)
                + usize::from(self.staged_mark);
        }
        if !self.dir.is_empty()
            || !self.name.is_empty()
            || self.dirty
            || self.readonly
            || self.multicursor.is_some()
        {
            left += SEP_W + self.file_w();
        }
        let mut right = self.position_width();
        if self.has_status() {
            right += self.status_width() + SEP_W;
        }
        if !self.services.is_empty() {
            right += self.services.width() + SEP_W;
        }
        if self.show_diag {
            right += SEP_W
                + if self.errors > 0 {
                    1 + digits(self.errors)
                } else {
                    0
                }
                + if self.warnings > 0 {
                    1 + digits(self.warnings)
                } else {
                    0
                }
                + usize::from(self.errors > 0 && self.warnings > 0);
        }
        if !self.indent.is_empty() {
            right += SEP_W + width(&self.indent);
        }
        left + GAP + right
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

fn digits(value: usize) -> usize {
    value
        .checked_ilog10()
        .map_or(1, |digits| digits as usize + 1)
}
