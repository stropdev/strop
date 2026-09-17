/// Wire form of `Row`. A terminal row stores one cell per column, and at the
/// capture bound a full-history frame costs hundreds of megabytes when each
/// cell and each padded text byte travels separately. The wire therefore
/// carries runs of adjacent cells sharing width, style and symbol; the row
/// text and every cell's byte end are reconstructed, exactly as frame
/// validation re-derives them, and the row's trailing default-styled space
/// padding is dropped — the frame re-pads to its geometry on decode.
/// Captures written before runs carry the text plus one object per column
/// with explicit ends; that shape still decodes.
mod row_serde {
    use super::super::{Cell, Row, Style, MAX_COLUMNS, MAX_ROW_BYTES};
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
            .any(|entry| matches!(entry, CellEntry::Single(_)))
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
            // Trailing default-styled single-width spaces are the row's
            // padding out to the frame geometry; the frame re-derives them.
            if runs
                .last()
                .is_some_and(|run| run.width == 1 && run.style.is_default() && run.text == " ")
            {
                runs.pop();
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

mod frame_serde {
    use super::super::{
        Cell, Frame, Geometry, Palette, ProjectedRow, Row, Style, MAX_HISTORY_LINES,
        MAX_PROJECTION_BYTES, MAX_ROWS,
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

    #[derive(Serialize)]
    #[serde(untagged)]
    enum WireEntry<'a> {
        Repeated { repeat: u32, index: usize },
        Inline(&'a Row),
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RowEntry {
        /// Consecutive occurrences of one dictionary row.
        Repeated {
            repeat: u32,
            index: usize,
        },
        Projected {
            absolute_start: u64,
            row: Row,
        },
        Plain(Row),
    }
    fn pad_to_geometry(row: &mut Row, columns: u16) {
        // The wire drops each row's trailing default-styled space padding;
        // the frame geometry is the authority that puts it back,
        // byte-for-byte as read_row produced it.
        while row.cells.len() < usize::from(columns) {
            row.text.push(' ');
            let end = row.text.len() as u32;
            row.cells.push(Cell {
                end,
                width: 1,
                style: Style::default(),
            });
        }
    }

    fn rebuild(
        origin: u64,
        columns: u16,
        entries: Vec<RowEntry>,
        dictionary: Vec<Row>,
        legacy_projection: Option<ropey::Rope>,
    ) -> Result<(imbl::Vector<ProjectedRow>, ropey::Rope), String> {
        let legacy = entries
            .iter()
            .any(|entry| matches!(entry, RowEntry::Projected { .. }));
        if legacy
            && entries
                .iter()
                .any(|entry| !matches!(entry, RowEntry::Projected { .. }))
        {
            return Err("terminal frame mixes legacy projected rows".into());
        }
        if dictionary.len() > MAX_HISTORY_LINES + usize::from(MAX_ROWS) {
            return Err("terminal frame dictionary exceeds its bound".into());
        }
        let mut padded: Vec<Arc<Row>> = Vec::with_capacity(dictionary.len());
        for mut row in dictionary {
            pad_to_geometry(&mut row, columns);
            padded.push(Arc::new(row));
        }
        let dictionary = padded;
        let mut total = 0usize;
        for entry in &entries {
            let count = match entry {
                RowEntry::Repeated { repeat, index } => {
                    if *repeat == 0 || *index >= dictionary.len() {
                        return Err("terminal frame repeats an unknown row".into());
                    }
                    *repeat as usize
                }
                _ => 1,
            };
            total = total
                .checked_add(count)
                .ok_or("terminal frame row count exceeds its bound")?;
        }
        if total > MAX_HISTORY_LINES + usize::from(MAX_ROWS) {
            return Err("terminal frame row count exceeds its bound".into());
        }
        fn emit(
            builder: &mut ropey::RopeBuilder,
            expected: &mut u64,
            projected: &mut Vec<ProjectedRow>,
            offset: Option<u64>,
            row: &Arc<Row>,
        ) -> Result<(), String> {
            let advance = row.text.len() as u64 + u64::from(!row.wrapped);
            builder.append(&row.text);
            if !row.wrapped {
                builder.append("\n");
            }
            projected.push(ProjectedRow {
                absolute_start: offset.unwrap_or(*expected),
                row: Arc::clone(row),
            });
            *expected = expected
                .checked_add(advance)
                .ok_or("terminal text identity exhausted")?;
            Ok(())
        }

        let mut rows = imbl::Vector::new();
        let mut builder = ropey::RopeBuilder::new();
        let mut expected = origin;
        let mut projected = Vec::with_capacity(total);
        for entry in entries {
            match entry {
                RowEntry::Repeated { repeat, index } => {
                    let row = Arc::clone(&dictionary[index]);
                    for _ in 0..repeat {
                        emit(&mut builder, &mut expected, &mut projected, None, &row)?;
                    }
                }
                RowEntry::Projected {
                    absolute_start,
                    row,
                } => {
                    let row = Arc::new(row);
                    emit(
                        &mut builder,
                        &mut expected,
                        &mut projected,
                        Some(absolute_start),
                        &row,
                    )?;
                }
                RowEntry::Plain(mut row) => {
                    pad_to_geometry(&mut row, columns);
                    let row = Arc::new(row);
                    emit(&mut builder, &mut expected, &mut projected, None, &row)?;
                }
            }
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
            // Rows repeating anywhere in the frame travel once, in a
            // dictionary in first-appearance order, and are referenced by
            // index runs; unique rows stay inline. Terminal history is
            // repetition-heavy, so a flood's frame collapses to its few
            // distinct rows, and a unique-heavy frame pays nothing.
            let rows: Vec<&Row> = self.rows.iter().map(|projected| &*projected.row).collect();
            let mut counts: std::collections::HashMap<&Row, usize> =
                std::collections::HashMap::with_capacity(rows.len());
            for row in &rows {
                *counts.entry(row).or_insert(0) += 1;
            }
            let mut dictionary: Vec<&Row> = Vec::new();
            let mut index_of: std::collections::HashMap<&Row, usize> =
                std::collections::HashMap::new();
            for row in &rows {
                if counts[row] >= 2 && !index_of.contains_key(row) {
                    index_of.insert(row, dictionary.len());
                    dictionary.push(row);
                }
            }
            let mut entries: Vec<WireEntry> = Vec::with_capacity(rows.len());
            for row in rows {
                match index_of.get(row) {
                    Some(index) => match entries.last_mut() {
                        Some(WireEntry::Repeated {
                            repeat,
                            index: last,
                        }) if last == index && *repeat < u32::MAX => {
                            *repeat += 1;
                        }
                        _ => entries.push(WireEntry::Repeated {
                            repeat: 1,
                            index: *index,
                        }),
                    },
                    None => entries.push(WireEntry::Inline(row)),
                }
            }
            let mut object = serializer.serialize_struct("Frame", FIELDS.len())?;
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
            object.serialize_field("rows", &entries)?;
            if !dictionary.is_empty() {
                object.serialize_field("dictionary", &dictionary)?;
            }
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
                    let mut geometry: Option<Geometry> = None;
                    let mut alternate = None;
                    let mut cursor = None;
                    let mut palette: Option<Palette> = None;
                    let mut history_rows = None;
                    let mut available_history_rows = None;
                    let mut history_limited = None;
                    let mut origin = None;
                    let mut rows: Option<Vec<RowEntry>> = None;
                    let mut dictionary: Option<Vec<Row>> = None;
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
                            "dictionary" => dictionary = Some(map.next_value()?),
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
                    let dictionary = dictionary.unwrap_or_default();
                    let geometry =
                        geometry.ok_or_else(|| A::Error::custom("frame is missing geometry"))?;
                    let (rows, projection) =
                        rebuild(origin, geometry.columns, rows, dictionary, projection)
                            .map_err(A::Error::custom)?;
                    Ok(Frame {
                        session: session
                            .ok_or_else(|| A::Error::custom("frame is missing session"))?,
                        revision: revision
                            .ok_or_else(|| A::Error::custom("frame is missing revision"))?,
                        geometry,
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
