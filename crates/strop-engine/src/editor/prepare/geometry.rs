//! Cell geometry and coordinate arithmetic for view preparation (AR01):
//! pane splits, picker card layout, gutter insets and preview windows —
//! pure functions over terminal cells, owned by the engine so preparation
//! and every frontend's paint share one source. No toolkit types cross
//! this boundary; frontends convert [`CellRect`] at their edge.

use strop_core::id::DocumentId;

use super::{Editor, Surface};

/// Terminal-cell geometry: the explicit viewport input a view epoch owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewGeometry {
    pub columns: u16,
    pub rows: u16,
}

/// One rectangle in terminal cells (engine domain; frontends convert to
/// their toolkit's rect type at the boundary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// Width of the standard gutter: sign column + 3-digit number + space.
/// One source for preparation clamps and gutter paint (0064 §1).
pub const STANDARD_GUTTER: usize = 5;

/// The blame column's reserved width (0011 §4): preparation's left inset
/// and the margin paint agree on exactly this.
pub const BLAME_GUTTER_WIDTH: usize = 22;

/// The pane's text budget: the scrollbar's reserved column excluded.
pub fn text_budget(rect: CellRect) -> CellRect {
    CellRect {
        width: rect.width.saturating_sub(1),
        ..rect
    }
}

/// Final pane rectangles (identity title row already subtracted) for the
/// current layout. Pure over pane count, layout direction and geometry;
/// preparation and paint share this one computation so both agree on the
/// exact same cells.
pub fn pane_cells(count: usize, is_row: bool, area: CellRect) -> Vec<CellRect> {
    if count == 0 {
        return Vec::new();
    }
    let total_w = usize::from(area.width);
    let total_h = usize::from(area.height.saturating_sub(1)); // statusline
    let axis = if is_row { total_w } else { total_h };
    let usable = axis.saturating_sub(count - 1);
    let base = usable / count;
    let mut offset = 0usize;
    let mut rects = Vec::with_capacity(count);
    for i in 0..count {
        let size = if i + 1 == count {
            usable - base * (count - 1)
        } else {
            base
        };
        let size = size.min(axis.saturating_sub(offset));
        let offset_in_area = offset.min(axis);
        let (x, y, w, h) = if is_row {
            (
                usize::from(area.x) + offset_in_area,
                usize::from(area.y),
                size,
                total_h,
            )
        } else {
            (
                usize::from(area.x),
                usize::from(area.y) + offset_in_area,
                total_w,
                size,
            )
        };
        let mut rect = CellRect::new(x as u16, y as u16, w as u16, h as u16);
        if count > 1 && h > 0 && w > 0 {
            rect.y = rect.y.saturating_add(1);
            rect.height = rect.height.saturating_sub(1);
        }
        rects.push(rect);
        offset = offset.saturating_add(size).saturating_add(1);
    }
    rects
}

impl CellRect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

impl From<ViewGeometry> for CellRect {
    fn from(geometry: ViewGeometry) -> Self {
        CellRect::new(0, 0, geometry.columns, geometry.rows)
    }
}

// --- picker card geometry (moved from the renderer; plain cell
// arithmetic pinned against the toolkit solver by the strop-editor
// picker geometry tests) ---

/// The floating/workspace card rect for the open kind (0050 §8).
pub fn picker_card(area: CellRect, kind: strop_picker::Kind) -> CellRect {
    if matches!(
        kind,
        strop_picker::Kind::Search
            | strop_picker::Kind::Files
            | strop_picker::Kind::Symbols
            | strop_picker::Kind::WorkspaceSymbols
    ) {
        CellRect {
            x: area.x + 1,
            y: area.y,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(1),
        }
    } else {
        let width = ((u32::from(area.width) * 84 / 100) as u16)
            .max(50)
            .min(area.width.saturating_sub(2));
        let height = if kind == strop_picker::Kind::RemoteAddress {
            8
        } else {
            ((u32::from(area.height) * 70 / 100) as u16).max(12)
        }
        .min(area.height.saturating_sub(2));
        CellRect {
            x: (area.width - width) / 2,
            y: (area.height - height) / 2,
            width,
            height,
        }
    }
}

/// The card's content area: 1-cell border + 1-cell padding (0001 §4).
pub fn picker_inner(card: CellRect) -> CellRect {
    CellRect {
        x: card.x + 2,
        y: card.y + 1,
        width: card.width.saturating_sub(4),
        height: card.height.saturating_sub(2),
    }
}

/// Input field rows for the open kind: replace mode adds a second field.
pub fn picker_input_height(kind: strop_picker::Kind, replace_visible: bool) -> u16 {
    if kind == strop_picker::Kind::Search && replace_visible {
        3
    } else {
        2
    }
}

/// The input/content vertical split: input fields on top, content below.
/// Exact arithmetic matching the historical `[Length(input), Min(1)]`
/// solver split: the content keeps its one-row minimum, so an inner area
/// shorter than the fields shrinks the fields first.
pub fn picker_field_split(inner: CellRect, input_h: u16) -> (CellRect, CellRect) {
    let field_h = input_h.min(inner.height.saturating_sub(1));
    let fields = CellRect {
        height: field_h,
        ..inner
    };
    let content = CellRect {
        y: inner.y + field_h,
        height: inner.height.saturating_sub(field_h),
        ..inner
    };
    (fields, content)
}

/// Two-column percentage split: the first segment rounds half-up (the
/// toolkit solver's behavior, pinned by test), the second takes the rest.
fn percentage_pair(width: u16, first_pct: u16) -> (u16, u16) {
    let first = ((u32::from(width) * u32::from(first_pct) + 50) / 100) as u16;
    (first, width.saturating_sub(first))
}

/// The results list and preview panes for the open kind: the same
/// input/content split and narrow-terminal stacking the paint performs.
/// `None` when the card is too short to show results at all — the active
/// field owns the card and the results viewport must not move.
pub fn picker_split(
    area: CellRect,
    kind: strop_picker::Kind,
    replace_visible: bool,
) -> Option<(CellRect, Option<CellRect>)> {
    let card = picker_card(area, kind);
    let inner = picker_inner(card);
    let input_h = picker_input_height(kind, replace_visible);
    if inner.height < input_h + 1 {
        return None;
    }
    let (_, content) = picker_field_split(inner, input_h);
    let remote_picker = matches!(
        kind,
        strop_picker::Kind::RemoteHosts | strop_picker::Kind::RemoteAddress
    );
    if remote_picker {
        return Some((content, None));
    }
    if card.width < 64 && content.height >= 12 {
        // Narrow terminals stack a short preview below the list: the
        // historical `[Min(8), Length(8)]` solver split — the list keeps
        // at least 8 rows, the preview takes the bottom 8 when they fit.
        let preview_h = content.height.saturating_sub(8).min(8);
        let list_h = content.height - preview_h;
        let list = CellRect {
            height: list_h,
            ..content
        };
        let preview = CellRect {
            y: content.y + list_h,
            height: content.height - list_h,
            ..content
        };
        return Some((list, Some(preview)));
    }
    if card.width < 64 {
        return Some((content, None));
    }
    // The file preview carries the evidence: it gets the wider share of
    // the full-screen workspace (grep 60/40, files/symbols 55/45).
    let list_pct = if kind == strop_picker::Kind::Search {
        60
    } else {
        55
    };
    let (list_w, preview_w) = percentage_pair(content.width, list_pct);
    let list = CellRect {
        width: list_w,
        ..content
    };
    let preview = CellRect {
        x: content.x + list_w,
        width: preview_w,
        ..content
    };
    Some((list, Some(preview)))
}

/// The results list area for the open kind.
pub fn picker_results(
    area: CellRect,
    kind: strop_picker::Kind,
    replace_visible: bool,
) -> Option<CellRect> {
    picker_split(area, kind, replace_visible).map(|(results, _)| results)
}

/// Rows per logical result row for the open kind.
pub fn picker_rows_per_result(kind: strop_picker::Kind, replace_visible: bool) -> usize {
    if kind == strop_picker::Kind::Search {
        if replace_visible {
            3
        } else {
            2
        }
    } else {
        1
    }
}

// --- gutter geometry (moved from the renderer; one composition for
// preparation clamps and paint, 0011 §3/§4) ---

/// Gutter width for a surface's buffer: the diff gutter widens to fit
/// both sides' numbers (min 3 digits each); everything else is the
/// standard sign+number gutter.
pub fn gutter_width(surface: Option<&Surface>) -> usize {
    let Some(Surface::Diff { hunks, .. }) = surface else {
        return STANDARD_GUTTER;
    };
    hunks.gutter_width()
}

impl Editor {
    /// The number gutter for a pane's buffer: diff surfaces keep their
    /// two-sided gutter; ordinary buffers size to the largest line number
    /// (a fixed 5-cell gutter misaligned the caret past line 999).
    pub fn number_gutter_width(&self, doc: DocumentId) -> usize {
        let buffer = self.doc(doc);
        let surface = buffer.surface_payload();
        if matches!(surface, Some(Surface::Diff { .. })) {
            return gutter_width(surface);
        }
        let number = buffer.buf.last_content_line() + 1;
        let digits = number.ilog10() as usize + 1;
        2 + digits.max(3)
    }

    /// Total left inset before a pane's content: file sidebar + blame
    /// column + the surface's number gutter. Cursor placement and the
    /// inactive-pane caret both derive from here — one composition, no
    /// per-surface drift. The sidebar contributes exactly what its
    /// emission draws (`Sidebar::outer_width`), so the caret and the tree
    /// can never disagree (0032 §3).
    pub fn left_inset(&self, buffer: DocumentId) -> usize {
        let surface = self.document(buffer).and_then(|d| d.surface_payload());
        let mut inset = self.number_gutter_width(buffer);
        if self.blame_gutter_for(buffer).is_some() {
            inset += BLAME_GUTTER_WIDTH;
        }
        if let Some(Surface::Diff {
            commit: Some(cf), ..
        }) = surface
        {
            inset += cf.files.sidebar().outer_width();
        }
        inset
    }
}

/// Like an editor buffer, a trailing newline terminates the last source
/// line; it is not an extra numbered line in a read-only preview.
pub fn preview_source_lines(rope: &ropey::Rope) -> usize {
    rope.len_lines()
        .saturating_sub(usize::from(
            rope.len_bytes() > 0 && rope.byte(rope.len_bytes() - 1) == b'\n',
        ))
        .max(1)
}

/// The visible source window of a picker preview: focus line biased a
/// third down, clamped to the source.
pub fn preview_window(
    rope: &ropey::Rope,
    focus: Option<usize>,
    visible: usize,
) -> std::ops::Range<usize> {
    let first = focus
        .map_or(0, |line| line.saturating_sub(1).saturating_sub(visible / 3))
        .min(preview_source_lines(rope).saturating_sub(1));
    first
        ..first
            .saturating_add(visible)
            .min(preview_source_lines(rope))
}
