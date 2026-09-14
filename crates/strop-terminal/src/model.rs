//! Frontend-neutral, owned terminal data. No native handles or borrowed VT cells.
use serde::{Deserialize, Serialize};
use std::sync::Arc;
mod validation;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(Rgb),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    /// End of this physical cell's UTF-8 symbol in Row::text. Continuations
    /// retain the preceding end and therefore expose an empty symbol.
    pub end: u32,
    pub width: u8,
    #[serde(skip_serializing_if = "Style::is_default", default)]
    pub style: Style,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub cells: Vec<Cell>,
    pub wrapped: bool,
}

/// Wire form of `Row`. A terminal row stores one cell per column, and at the
/// capture bound a full-history frame costs hundreds of megabytes when each
/// cell and each padded text byte travels separately. The wire therefore
/// carries runs of adjacent cells sharing width, style and symbol; the row
/// text and every cell's byte end are reconstructed, exactly as frame
/// validation re-derives them. Captures written before runs carry the text
/// plus one object per column with explicit ends; that shape still decodes.
mod row_serde {
    use super::{Cell, Row, Style, MAX_COLUMNS, MAX_ROW_BYTES};
    use serde::de::{Error as _, MapAccess, Visitor};
    use serde::ser::SerializeStruct;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::fmt;

    #[derive(Serialize)]
    struct CellRun<'a> {
        repeat: u32,
        width: u8,
        #[serde(skip_serializing_if = "Style::is_default")]
        style: &'a Style,
        text: &'a str,
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum CellEntry {
        Run {
            repeat: u32,
            width: u8,
            #[serde(default)]
            style: Style,
            text: String,
        },
        Single(Cell),
    }

    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct RowWire {
        text: Option<String>,
        cells: Vec<CellEntry>,
        wrapped: bool,
    }

    fn assemble(
        text: Option<String>,
        entries: Vec<CellEntry>,
        wrapped: bool,
    ) -> Result<Row, String> {
        let mut cells = Vec::with_capacity(entries.len());
        if entries
            .iter()
            .all(|entry| matches!(entry, CellEntry::Single(_)))
        {
            // Legacy shape: explicit ends against the recorded text.
            let text = text.ok_or("legacy terminal row is missing its text")?;
            if text.len() > MAX_ROW_BYTES {
                return Err("terminal row text exceeds its budget".into());
            }
            for entry in entries {
                let CellEntry::Single(cell) = entry else {
                    return Err("terminal row mixes run and per-column cells".into());
                };
                if cells.len() >= usize::from(MAX_COLUMNS)
                    || cell.end as usize > text.len()
                    || !text.is_char_boundary(cell.end as usize)
                {
                    return Err("terminal cell exceeds its row".into());
                }
                cells.push(cell);
            }
            return Ok(Row {
                text,
                cells,
                wrapped,
            });
        }
        if entries
            .iter()
            .any(|entry| matches!(entry, CellEntry::Single(_)))
        {
            return Err("terminal row mixes run and per-column cells".into());
        }
        let mut text = String::new();
        for entry in entries {
            let CellEntry::Run {
                repeat,
                width,
                style,
                text: symbol,
            } = entry
            else {
                return Err("terminal row mixes run and per-column cells".into());
            };
            let malformed = repeat == 0
                || repeat as usize > usize::from(MAX_COLUMNS) - cells.len()
                || (width == 0) != symbol.is_empty()
                || width > 2
                || text.len() + symbol.len() * repeat as usize > MAX_ROW_BYTES;
            if malformed {
                return Err("terminal cell run exceeds its bounds".into());
            }
            for _ in 0..repeat {
                text.push_str(&symbol);
                cells.push(Cell {
                    end: text.len() as u32,
                    width,
                    style,
                });
            }
        }
        Ok(Row {
            text,
            cells,
            wrapped,
        })
    }

    impl Serialize for Row {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut runs: Vec<CellRun> = Vec::new();
            let mut start = 0usize;
            for cell in &self.cells {
                let end = cell.end as usize;
                let symbol = self.text.get(start..end).unwrap_or_default();
                let run = runs.last_mut();
                if run.is_some_and(|run| {
                    run.repeat < u32::MAX
                        && run.width == cell.width
                        && run.style == &cell.style
                        && run.text == symbol
                }) {
                    if let Some(run) = runs.last_mut() {
                        run.repeat += 1;
                    }
                } else {
                    runs.push(CellRun {
                        repeat: 1,
                        width: cell.width,
                        style: &cell.style,
                        text: symbol,
                    });
                }
                start = end;
            }
            let mut object = serializer.serialize_struct("Row", 2)?;
            object.serialize_field("cells", &runs)?;
            object.serialize_field("wrapped", &self.wrapped)?;
            object.end()
        }
    }

    impl<'de> Deserialize<'de> for Row {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct RowVisitor;
            impl<'de> Visitor<'de> for RowVisitor {
                type Value = Row;
                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("bounded terminal row")
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Row, A::Error> {
                    let mut wire = RowWire::default();
                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "text" => wire.text = Some(map.next_value()?),
                            "cells" => wire.cells = map.next_value()?,
                            "wrapped" => wire.wrapped = map.next_value()?,
                            _ => return Err(A::Error::custom("unknown terminal row field")),
                        }
                    }
                    assemble(wire.text, wire.cells, wire.wrapped).map_err(A::Error::custom)
                }
            }
            deserializer.deserialize_map(RowVisitor)
        }
    }
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

mod frame_serde {
    use super::{
        Frame, Palette, ProjectedRow, Row, MAX_HISTORY_LINES, MAX_PROJECTION_BYTES, MAX_ROWS,
    };
    use serde::de::{Deserializer, Error as _, MapAccess, Visitor};
    use serde::ser::SerializeStruct;
    use serde::{Deserialize, Serialize, Serializer};
    use std::fmt;
    use std::sync::Arc;

    const FIELDS: &[&str] = &[
        "session",
        "revision",
        "geometry",
        "alternate",
        "cursor",
        "palette",
        "history_rows",
        "available_history_rows",
        "history_limited",
        "origin",
        "rows",
        "projection",
    ];

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RowEntry {
        Projected { absolute_start: u64, row: Row },
        Plain(Row),
    }

    fn rebuild(
        origin: u64,
        entries: Vec<RowEntry>,
        legacy_projection: Option<ropey::Rope>,
    ) -> Result<(imbl::Vector<ProjectedRow>, ropey::Rope), String> {
        let legacy = entries
            .iter()
            .any(|entry| matches!(entry, RowEntry::Projected { .. }));
        if legacy
            && entries
                .iter()
                .any(|entry| matches!(entry, RowEntry::Plain(_)))
        {
            return Err("terminal frame mixes projected and plain rows".into());
        }
        if entries.len() > MAX_HISTORY_LINES + usize::from(MAX_ROWS) {
            return Err("terminal frame row count exceeds its bound".into());
        }
        let mut rows = imbl::Vector::new();
        let mut builder = ropey::RopeBuilder::new();
        let mut expected = origin;
        let mut projected = Vec::with_capacity(entries.len());
        for entry in entries {
            let (offset, row) = match entry {
                RowEntry::Projected {
                    absolute_start,
                    row,
                } => (Some(absolute_start), row),
                RowEntry::Plain(row) => (None, row),
            };
            let row = Arc::new(row);
            let advance = row.text.len() as u64 + u64::from(!row.wrapped);
            builder.append(&row.text);
            if !row.wrapped {
                builder.append("\n");
            }
            projected.push(ProjectedRow {
                absolute_start: offset.unwrap_or(expected),
                row,
            });
            expected = expected
                .checked_add(advance)
                .ok_or("terminal text identity exhausted")?;
        }
        let projection = legacy_projection.unwrap_or_else(|| builder.finish());
        if projection.len_bytes() > MAX_PROJECTION_BYTES {
            return Err("terminal projection exceeds its budget".into());
        }
        for entry in projected {
            rows.push_back(entry);
        }
        Ok((rows, projection))
    }

    /// Legacy captures serialized the projection rope in place.
    #[derive(Deserialize)]
    #[serde(transparent)]
    struct ProjectionWire(#[serde(with = "crate::projection::rope_serde")] ropey::Rope);

    impl Serialize for Frame {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let rows: Vec<&Row> = self.rows.iter().map(|projected| &*projected.row).collect();
            let mut object = serializer.serialize_struct("Frame", FIELDS.len() - 1)?;
            object.serialize_field("session", &self.session)?;
            object.serialize_field("revision", &self.revision)?;
            object.serialize_field("geometry", &self.geometry)?;
            object.serialize_field("alternate", &self.alternate)?;
            object.serialize_field("cursor", &self.cursor)?;
            object.serialize_field("palette", &self.palette)?;
            object.serialize_field("history_rows", &self.history_rows)?;
            object.serialize_field("available_history_rows", &self.available_history_rows)?;
            object.serialize_field("history_limited", &self.history_limited)?;
            object.serialize_field("origin", &self.origin)?;
            object.serialize_field("rows", &rows)?;
            object.end()
        }
    }

    impl<'de> Deserialize<'de> for Frame {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct FrameVisitor;
            impl<'de> Visitor<'de> for FrameVisitor {
                type Value = Frame;
                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("bounded terminal frame")
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Frame, A::Error> {
                    let mut session = None;
                    let mut revision = None;
                    let mut geometry = None;
                    let mut alternate = None;
                    let mut cursor = None;
                    let mut palette: Option<Palette> = None;
                    let mut history_rows = None;
                    let mut available_history_rows = None;
                    let mut history_limited = None;
                    let mut origin = None;
                    let mut rows: Option<Vec<RowEntry>> = None;
                    let mut projection = None;
                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "session" => session = Some(map.next_value()?),
                            "revision" => revision = Some(map.next_value()?),
                            "geometry" => geometry = Some(map.next_value()?),
                            "alternate" => alternate = Some(map.next_value()?),
                            "cursor" => cursor = Some(map.next_value()?),
                            "palette" => palette = Some(map.next_value()?),
                            "history_rows" => history_rows = Some(map.next_value()?),
                            "available_history_rows" => {
                                available_history_rows = Some(map.next_value()?)
                            }
                            "history_limited" => history_limited = Some(map.next_value()?),
                            "origin" => origin = Some(map.next_value()?),
                            "rows" => rows = Some(map.next_value()?),
                            "projection" => {
                                projection = Some(
                                    map.next_value::<ProjectionWire>()
                                        .map_err(|_| {
                                            A::Error::custom("terminal projection does not decode")
                                        })?
                                        .0,
                                )
                            }
                            _ => return Err(A::Error::custom("unknown terminal frame field")),
                        }
                    }
                    let rows = rows.ok_or_else(|| A::Error::custom("frame is missing rows"))?;
                    let origin =
                        origin.ok_or_else(|| A::Error::custom("frame is missing origin"))?;
                    let (rows, projection) =
                        rebuild(origin, rows, projection).map_err(A::Error::custom)?;
                    Ok(Frame {
                        session: session
                            .ok_or_else(|| A::Error::custom("frame is missing session"))?,
                        revision: revision
                            .ok_or_else(|| A::Error::custom("frame is missing revision"))?,
                        geometry: geometry
                            .ok_or_else(|| A::Error::custom("frame is missing geometry"))?,
                        alternate: alternate
                            .ok_or_else(|| A::Error::custom("frame is missing alternate"))?,
                        cursor: cursor
                            .ok_or_else(|| A::Error::custom("frame is missing cursor"))?,
                        palette: Arc::new(
                            palette.ok_or_else(|| A::Error::custom("frame is missing palette"))?,
                        ),
                        history_rows: history_rows
                            .ok_or_else(|| A::Error::custom("frame is missing history_rows"))?,
                        available_history_rows: available_history_rows.ok_or_else(|| {
                            A::Error::custom("frame is missing available_history_rows")
                        })?,
                        history_limited: history_limited
                            .ok_or_else(|| A::Error::custom("frame is missing history_limited"))?,
                        origin,
                        rows,
                        projection,
                    })
                }
            }
            deserializer.deserialize_map(FrameVisitor)
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn padded_row() -> Row {
        // What read_row produces for a flood line: one glyph then
        // default-styled padding out to the geometry width.
        let text = "y".to_string() + &" ".repeat(119);
        let mut cells = Vec::with_capacity(120);
        let mut end = 0;
        for symbol in ["y"].into_iter().chain(std::iter::repeat_n(" ", 119)) {
            end += symbol.len();
            cells.push(Cell {
                end: end as u32,
                width: 1,
                style: Style::default(),
            });
        }
        Row {
            text,
            cells,
            wrapped: false,
        }
    }

    #[test]
    fn padded_rows_travel_as_runs_near_their_text_size() {
        let row = padded_row();
        let wire = serde_json::to_string(&row).unwrap();
        // Without run merging this row costs a cell object per column
        // (~20KB); with it, the wire is the text plus two runs.
        assert!(
            wire.contains(r#""repeat":119"#),
            "padding must merge: {wire}"
        );
        assert!(
            wire.len() < 400,
            "padded row must stay near text size: {wire}"
        );
        assert_eq!(serde_json::from_str::<Row>(&wire).unwrap(), row);
    }

    #[test]
    fn styled_and_wide_cells_survive_the_run_wire() {
        let styled = Style {
            foreground: Color::Indexed(3),
            ..Style::default()
        };
        let row = Row {
            text: "éxx  a".into(),
            cells: vec![
                Cell {
                    end: 2,
                    width: 2,
                    style: styled,
                },
                Cell {
                    end: 2,
                    width: 0,
                    style: styled,
                },
                Cell {
                    end: 3,
                    width: 1,
                    style: styled,
                },
                Cell {
                    end: 4,
                    width: 1,
                    style: styled,
                },
                Cell {
                    end: 5,
                    width: 1,
                    style: Style::default(),
                },
                Cell {
                    end: 6,
                    width: 1,
                    style: Style::default(),
                },
                Cell {
                    end: 7,
                    width: 1,
                    style: Style::default(),
                },
            ],
            wrapped: true,
        };
        let wire = serde_json::to_string(&row).unwrap();
        assert!(
            wire.contains(r#""repeat":2,"width":1,"style""#),
            "styled repeats must merge: {wire}"
        );
        assert!(
            wire.contains(r#""repeat":2,"width":1,"text":" ""#),
            "default-styled repeats must merge without a style: {wire}"
        );
        assert_eq!(serde_json::from_str::<Row>(&wire).unwrap(), row);
    }

    #[test]
    fn legacy_per_column_cell_arrays_still_decode() {
        let style = serde_json::to_value(Style::default()).unwrap();
        let cell = |end: u32| serde_json::json!({"end": end, "width": 1, "style": style});
        let wire = serde_json::to_string(&serde_json::json!({
            "text": "ab",
            "cells": [cell(1), cell(2), cell(2)],
            "wrapped": false,
        }))
        .unwrap();
        let row = serde_json::from_str::<Row>(&wire).unwrap();
        assert_eq!(row.cells.len(), 3);
        assert_eq!(row.symbol(1), Some("b"));
    }

    #[test]
    fn run_expansion_is_bounded_and_shape_consistent() {
        for wire in [
            r#"{"cells":[{"repeat":0,"width":1,"text":" "}],"wrapped":false}"#,
            r#"{"cells":[{"repeat":513,"width":1,"text":" "}],"wrapped":false}"#,
            r#"{"cells":[{"repeat":300,"width":1,"text":" "},{"repeat":300,"width":1,"text":" "}],"wrapped":false}"#,
            r#"{"cells":[{"repeat":1,"width":0,"text":"x"}],"wrapped":false}"#,
            r#"{"cells":[{"repeat":2,"width":1,"text":""}],"wrapped":false}"#,
            r#"{"cells":[{"repeat":1,"width":3,"text":"x"}],"wrapped":false}"#,
            r#"{"text":"ab","cells":[{"repeat":1,"width":1,"text":"a"},{"end":2,"width":1}],"wrapped":false}"#,
        ] {
            assert!(
                serde_json::from_str::<Row>(wire).is_err(),
                "run must be refused: {wire}"
            );
        }
    }

    #[test]
    fn frame_wire_derives_projection_and_offsets() {
        let rows: imbl::Vector<ProjectedRow> = ["ab", "cd"]
            .into_iter()
            .enumerate()
            .map(|(index, text)| ProjectedRow {
                absolute_start: (index * 3) as u64,
                row: std::sync::Arc::new(Row {
                    text: text.into(),
                    cells: text
                        .chars()
                        .scan(0, |end, symbol| {
                            *end += symbol.len_utf8();
                            Some(Cell {
                                end: *end as u32,
                                width: 1,
                                style: Style::default(),
                            })
                        })
                        .collect(),
                    wrapped: false,
                }),
            })
            .collect();
        let frame = Frame {
            session: SessionId::from_request(strop_core::worker::WorkerId::new(7)),
            revision: 3,
            geometry: Geometry {
                columns: 2,
                rows: 2,
                revision: 0,
            },
            alternate: false,
            cursor: Cursor {
                column: 1,
                row: 0,
                visible: true,
                blinking: false,
                shape: CursorShape::Block,
            },
            palette: std::sync::Arc::new(Palette {
                foreground: Rgb {
                    red: 1,
                    green: 2,
                    blue: 3,
                },
                background: Rgb {
                    red: 4,
                    green: 5,
                    blue: 6,
                },
                colors: vec![
                    Rgb {
                        red: 0,
                        green: 0,
                        blue: 0
                    };
                    256
                ],
            }),
            history_rows: 0,
            available_history_rows: 0,
            history_limited: false,
            origin: 0,
            projection: ropey::Rope::from_str("ab\ncd\n"),
            rows,
        };
        let wire = serde_json::to_string(&frame).unwrap();
        assert!(!wire.contains("projection"), "rope must be derived: {wire}");
        assert!(
            !wire.contains("absolute_start"),
            "offsets must be derived: {wire}"
        );
        let decoded: Frame = serde_json::from_str(&wire).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded.projection.to_string(), "ab\ncd\n");
        assert_eq!(decoded.rows[1].absolute_start, 3);
    }

    #[test]
    fn legacy_frames_with_explicit_projection_still_decode() {
        let style = serde_json::to_value(Style::default()).unwrap();
        let cell = |end: u32| serde_json::json!({"end": end, "width": 1, "style": style});
        let palette = serde_json::to_value(Palette {
            foreground: Rgb {
                red: 0,
                green: 0,
                blue: 0,
            },
            background: Rgb {
                red: 0,
                green: 0,
                blue: 0,
            },
            colors: vec![
                Rgb {
                    red: 0,
                    green: 0,
                    blue: 0
                };
                256
            ],
        })
        .unwrap();
        let wire = serde_json::to_string(&serde_json::json!({
            "session": 1,
            "revision": 1,
            "geometry": {"columns": 2, "rows": 1, "revision": 0},
            "alternate": false,
            "cursor": {"column": 0, "row": 0, "visible": true, "blinking": false, "shape": "Block"},
            "palette": palette,
            "history_rows": 0,
            "available_history_rows": 0,
            "history_limited": false,
            "origin": 0,
            "rows": [
                {"absolute_start": 0,
                 "row": {"text": "ab",
                         "cells": [cell(1), cell(2)],
                         "wrapped": false}}
            ],
            "projection": ["ab\n"],
        }))
        .unwrap();
        let frame: Frame = serde_json::from_str(&wire).unwrap();
        frame.validate().unwrap();
        assert_eq!(frame.projection.to_string(), "ab\n");
    }
}
