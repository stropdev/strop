//! Frontend-neutral, owned terminal data. No native handles or borrowed VT cells.
use serde::{Deserialize, Serialize};
use std::sync::Arc;
mod validation;
mod wire;

#[cfg(test)]
mod tests;

pub const MAX_COLUMNS: u16 = 512;
pub const MAX_ROWS: u16 = 256;
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;
pub const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_HISTORY_LINES: usize = 10_000;
pub const MAX_PROJECTION_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_GLYPH_CODEPOINTS: usize = 1024;
pub const MAX_ROW_BYTES: usize = 64 * 1024;
/// Disambiguation, event kinds, all keys and associated text. Alternate physical
/// key codes are not exposed by the current frontend-neutral input contract.
pub const SUPPORTED_KEYBOARD_FLAGS: u8 = 0b1_1011;
pub const MAX_SESSIONS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(u64);
impl SessionId {
    pub fn from_request(request: strop_core::worker::WorkerId) -> Self {
        Self(request.get())
    }
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub columns: u16,
    pub rows: u16,
    pub revision: u64,
}
impl Geometry {
    pub fn valid(self) -> bool {
        self.columns > 0 && self.columns <= MAX_COLUMNS && self.rows > 0 && self.rows <= MAX_ROWS
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq, Default, Serialize, Deserialize)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(Rgb),
}

#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq, Default, Serialize, Deserialize)]
pub struct Style {
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub faint: bool,
    pub invisible: bool,
    pub strikethrough: bool,
}
impl Style {
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq, Serialize, Deserialize)]
pub struct Cell {
    /// End of this physical cell's UTF-8 symbol in Row::text. Continuations
    /// retain the preceding end and therefore expose an empty symbol.
    pub end: u32,
    pub width: u8,
    #[serde(skip_serializing_if = "Style::is_default", default)]
    pub style: Style,
}

#[derive(Debug, Clone, PartialEq, Hash, Eq)]
pub struct Row {
    pub text: String,
    pub cells: Vec<Cell>,
    pub wrapped: bool,
}
impl Row {
    pub fn symbol(&self, column: usize) -> Option<&str> {
        let end = self.cells.get(column)?.end as usize;
        let start = column
            .checked_sub(1)
            .map_or(0, |before| self.cells[before].end as usize);
        self.text.get(start..end)
    }
    pub fn byte_at(&self, column: usize) -> usize {
        let mut column = column.min(self.cells.len());
        while column > 0 && self.cells.get(column).is_some_and(|cell| cell.width == 0) {
            column -= 1;
        }
        column
            .checked_sub(1)
            .map_or(0, |before| self.cells[before].end as usize)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
    HollowBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub column: u16,
    pub row: u16,
    pub visible: bool,
    pub blinking: bool,
    pub shape: CursorShape,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub colors: Vec<Rgb>,
}
impl Palette {
    /// The strop-owned default (0065 D3): theme-seed foreground and
    /// background plus the harmonized ANSI-16 over the standard
    /// 6×6×6/grayscale ramps. One source — `strop_core::theme` — feeds
    /// both this and the frontend's render constants.
    pub fn strop() -> Self {
        let rgb = |value: strop_core::theme::Rgb| Rgb {
            red: value.r,
            green: value.g,
            blue: value.b,
        };
        Self {
            foreground: rgb(strop_core::theme::TEXT),
            background: rgb(strop_core::theme::BASE),
            colors: strop_core::theme::ansi256()
                .iter()
                .map(|value| rgb(*value))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectedRow {
    pub absolute_start: u64,
    pub row: Arc<Row>,
}

/// Wire form of `Frame`. The projection rope is exactly the concatenation
/// of the row texts and every `absolute_start` is the running byte offset
/// from `origin` — frame validation re-derives both — so the wire carries
/// neither: decode rebuilds them. Captures written before this derivation
/// carry explicit offsets and the projection; that shape still decodes.
#[derive(Debug, Clone)]
pub struct Frame {
    pub session: SessionId,
    pub revision: u64,
    pub geometry: Geometry,
    pub alternate: bool,
    pub cursor: Cursor,
    pub palette: Arc<Palette>,
    pub history_rows: usize,
    pub available_history_rows: usize,
    pub history_limited: bool,
    pub origin: u64,
    pub rows: imbl::Vector<ProjectedRow>,
    pub projection: ropey::Rope,
}
impl Frame {
    pub fn cursor_byte(&self) -> usize {
        self.rows
            .get(self.history_rows + self.cursor.row as usize)
            .map_or(0, |entry| {
                entry.absolute_start.saturating_sub(self.origin) as usize
                    + entry.row.byte_at(self.cursor.column as usize)
            })
            .min(self.projection.len_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Starting,
    Running,
    Closing,
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    Failed(String),
}
impl Phase {
    pub fn live(&self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Closing)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    Title(String),
    ReportedDirectory(Vec<u8>),
    Bell,
    ClipboardWriteDenied,
    HostControlDenied,
    PasteConfirmationRequested { ticket: u64 },
    PasteConfirmationCleared { ticket: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Update {
    pub session: SessionId,
    pub phase: Phase,
    #[serde(deserialize_with = "validation::deserialize_frame")]
    pub frame: Option<Arc<Frame>>,
    pub effects: Vec<Effect>,
    pub acknowledged_input: u64,
    pub warning: Option<String>,
}
