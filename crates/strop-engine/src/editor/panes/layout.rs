//! The window tree (plans/0104 §4.1): vim's frames. A split divides the
//! current window along one axis; closing a window gives its cells to its
//! neighbours and collapses a split left with one child. Leaf `k` in reading
//! order (top-left first, vim's `winnr`) is `Editor.panes[k]`; the tree holds
//! no ids.
//!
//! How one split's cells are shared among its children is decided by
//! `strop_kernel::layout::tile` (proved in `proofs/Kernel/Layout.lean`); this
//! module walks the tree and asks it once per split.

use strop_kernel::layout::{self as kernel, Span};

use crate::editor::prepare::CellRect;

/// The direction a split lays its children out in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Axis {
    /// Side by side (`:vs`), a divider column between neighbours.
    Row,
    /// Stacked (`:sp`); the lower pane's title row is the boundary.
    Column,
}

/// A node of the window tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Layout {
    Leaf,
    Split { axis: Axis, children: Vec<Layout> },
}

/// Where everything goes for one view area.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    /// Text rects, one per pane, the title row already taken off.
    pub panes: Vec<CellRect>,
    /// Each pane with its title row: what directional focus looks at.
    pub frames: Vec<CellRect>,
    /// One column wide; one per boundary between side-by-side children.
    pub dividers: Vec<CellRect>,
}

/// A stacked window needs its title and one text row.
const STACKED_UNIT: usize = 2;

impl Layout {
    /// The number of windows.
    pub fn leaves(&self) -> usize {
        match self {
            Layout::Leaf => 1,
            Layout::Split { children, .. } => children.iter().map(Layout::leaves).sum(),
        }
    }

    /// Windows across along `axis` (vim's frame width in windows): a split
    /// along it adds its children's, a split across it takes the widest.
    pub fn tracks(&self, axis: Axis) -> usize {
        match self {
            Layout::Leaf => 1,
            Layout::Split {
                axis: own,
                children,
            } => {
                let each = children.iter().map(|child| child.tracks(axis));
                if *own == axis {
                    each.sum()
                } else {
                    each.max().unwrap_or(1)
                }
            }
        }
    }

    /// Every split has two or more children, none with its parent's axis.
    pub fn well_formed(&self) -> bool {
        fn node(layout: &Layout, parent: Option<Axis>) -> bool {
            match layout {
                Layout::Leaf => true,
                Layout::Split { axis, children } => {
                    children.len() >= 2
                        && parent != Some(*axis)
                        && children.iter().all(|child| node(child, Some(*axis)))
                }
            }
        }
        node(self, None)
    }

    /// The child indexes from the root to leaf `leaf`.
    fn path(&self, mut leaf: usize) -> Vec<usize> {
        let mut path = Vec::new();
        let mut node = self;
        while let Layout::Split { children, .. } = node {
            let mut index = 0;
            while index + 1 < children.len() && leaf >= children[index].leaves() {
                leaf -= children[index].leaves();
                index += 1;
            }
            path.push(index);
            node = &children[index];
        }
        path
    }

    fn node_mut(&mut self, path: &[usize]) -> &mut Layout {
        let mut node = self;
        for &index in path {
            let Layout::Split { children, .. } = node else {
                break;
            };
            node = &mut children[index];
        }
        node
    }

    /// Split leaf `leaf` along `axis`; the new window is leaf `leaf + 1`.
    pub fn split(&mut self, leaf: usize, axis: Axis) {
        let path = self.path(leaf);
        let Some((&index, parent_path)) = path.split_last() else {
            *self = Layout::Split {
                axis,
                children: vec![Layout::Leaf, Layout::Leaf],
            };
            return;
        };
        match self.node_mut(parent_path) {
            Layout::Split {
                axis: own,
                children,
            } if *own == axis => children.insert(index + 1, Layout::Leaf),
            Layout::Split { children, .. } => {
                children[index] = Layout::Split {
                    axis,
                    children: vec![Layout::Leaf, Layout::Leaf],
                };
            }
            Layout::Leaf => debug_assert!(false, "a path ends in a split's child"),
        }
    }

    /// Remove leaf `leaf` (two or more windows) and return the window that
    /// takes its cells, numbered after the removal: the first window of the
    /// previous sibling, or with none the first of the next (vim).
    pub fn remove(&mut self, leaf: usize) -> usize {
        let path = self.path(leaf);
        let Some((&index, parent_path)) = path.split_last() else {
            debug_assert!(false, "the last window is never removed");
            return 0;
        };
        let Layout::Split { children, .. } = self.node_mut(parent_path) else {
            return 0;
        };
        let focus = if index > 0 {
            leaf - children[index - 1].leaves()
        } else {
            leaf
        };
        children.remove(index);
        if children.len() == 1 {
            let only = children.remove(0);
            match parent_path.split_last() {
                None => *self = only,
                Some((&at, grand_path)) => {
                    let Layout::Split { children, .. } = self.node_mut(grand_path) else {
                        return focus;
                    };
                    match only {
                        // its axis is the grandparent's: splice, keep normal form
                        Layout::Split {
                            children: inner, ..
                        } => {
                            children.splice(at..=at, inner);
                        }
                        Layout::Leaf => children[at] = Layout::Leaf,
                    }
                }
            }
        }
        focus
    }

    /// The tree as vim's `winlayout()` with window numbers: a leaf is its
    /// 1-based number, a split `["row" | "col", [children…]]`.
    pub fn outline(&self) -> serde_json::Value {
        fn node(layout: &Layout, next: &mut usize) -> serde_json::Value {
            match layout {
                Layout::Leaf => {
                    *next += 1;
                    serde_json::json!(*next)
                }
                Layout::Split { axis, children } => {
                    let name = match axis {
                        Axis::Row => "row",
                        Axis::Column => "col",
                    };
                    let inner: Vec<_> = children.iter().map(|child| node(child, next)).collect();
                    serde_json::json!([name, inner])
                }
            }
        }
        node(self, &mut 0)
    }

    /// Place every window in `area` (its last row is the modeline).
    pub fn place(&self, area: CellRect) -> Placement {
        let canvas = CellRect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
        let mut placement = Placement::default();
        if let Layout::Leaf = self {
            placement.panes.push(canvas);
            placement.frames.push(canvas);
        } else {
            self.place_frame(canvas, &mut placement);
        }
        placement
    }

    fn place_frame(&self, frame: CellRect, placement: &mut Placement) {
        match self {
            Layout::Leaf => {
                let pane = if frame.width > 0 && frame.height > 0 {
                    CellRect::new(frame.x, frame.y + 1, frame.width, frame.height - 1)
                } else {
                    frame
                };
                placement.panes.push(pane);
                placement.frames.push(frame);
            }
            Layout::Split { axis, children } => {
                let tracks: Vec<usize> = children.iter().map(|child| child.tracks(*axis)).collect();
                let mut spans = vec![kernel::empty(); children.len()];
                let (cells, unit, gap) = match axis {
                    Axis::Row => (frame.width, 1, 1),
                    Axis::Column => (frame.height, STACKED_UNIT, 0),
                };
                kernel::tile(usize::from(cells), unit, gap, &tracks, &mut spans);
                let last = children.len() - 1;
                for (k, (child, span)) in children.iter().zip(&spans).enumerate() {
                    child.place_frame(child_frame(frame, *axis, *span), placement);
                    let end = span.at + span.len;
                    if *axis == Axis::Row && k < last && end < usize::from(frame.width) {
                        placement.dividers.push(CellRect::new(
                            frame.x + cell(end),
                            frame.y,
                            1,
                            frame.height,
                        ));
                    }
                }
            }
        }
    }
}

/// Every span is inside its parent (the kernel's `tile_in_bounds`), and the
/// parent's cells came from a `u16`, so the narrowing cannot truncate.
fn cell(value: usize) -> u16 {
    debug_assert!(value <= usize::from(u16::MAX));
    value as u16
}

fn child_frame(frame: CellRect, axis: Axis, span: Span) -> CellRect {
    match axis {
        Axis::Row => CellRect::new(
            frame.x + cell(span.at),
            frame.y,
            cell(span.len),
            frame.height,
        ),
        Axis::Column => CellRect::new(
            frame.x,
            frame.y + cell(span.at),
            frame.width,
            cell(span.len),
        ),
    }
}

/// The fewest cells the tree needs: `(columns, rows above the modeline)`.
pub fn minimum(layout: &Layout) -> (usize, usize) {
    (
        kernel::minimum(layout.tracks(Axis::Row), 1, 1),
        kernel::minimum(layout.tracks(Axis::Column), STACKED_UNIT, 0),
    )
}
