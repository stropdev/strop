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
fn padded_rows_drop_their_trailing_padding() {
    let row = padded_row();
    let wire = serde_json::to_string(&row).unwrap();
    // Without runs this row costs a cell object per column (~20KB);
    // with runs plus padding derivation it is one run and no padding.
    assert_eq!(
        wire,
        r#"{"cells":[{"repeat":1,"width":1,"text":"y"}],"wrapped":false}"#
    );
    // The row itself decodes short; only the frame knows the geometry
    // that puts the padding back (see the frame wire tests).
    let decoded = serde_json::from_str::<Row>(&wire).unwrap();
    assert_eq!(decoded.cells.len(), 1);
    assert_eq!(decoded.text, "y");
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

/// A frame of full-width ASCII rows exactly as read_row produces them.
fn full_frame(columns: u16, lines: &[&str]) -> Frame {
    let screen = u16::try_from(lines.len()).unwrap_or(30).min(30);
    let mut projection = String::new();
    let mut rows = imbl::Vector::new();
    let mut offset = 0u64;
    for line in lines {
        let mut text = (*line).to_string();
        while text.len() < usize::from(columns) {
            text.push(' ');
        }
        let cells = (1..=usize::from(columns))
            .map(|column| Cell {
                end: column as u32,
                width: 1,
                style: Style::default(),
            })
            .collect();
        projection.push_str(&text);
        projection.push('\n');
        rows.push_back(ProjectedRow {
            absolute_start: offset,
            row: std::sync::Arc::new(Row {
                text,
                cells,
                wrapped: false,
            }),
        });
        offset += u64::from(columns) + 1;
    }
    Frame {
        session: SessionId::from_request(strop_core::worker::WorkerId::new(7)),
        revision: 3,
        geometry: Geometry {
            columns,
            rows: screen,
            revision: 0,
        },
        alternate: false,
        cursor: Cursor {
            column: 0,
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
        history_rows: rows.len().saturating_sub(usize::from(screen)),
        available_history_rows: rows.len().saturating_sub(usize::from(screen)),
        history_limited: false,
        origin: 0,
        projection: ropey::Rope::from_str(&projection),
        rows,
    }
}

#[test]
fn frame_wire_re_derives_trailing_padding() {
    let frame = full_frame(5, &["ab", "", "     "]);
    let wire = serde_json::to_string(&frame).unwrap();
    assert!(
        !wire.contains(r#""text":" ""#),
        "padding runs must not travel: {wire}"
    );
    let decoded: Frame = serde_json::from_str(&wire).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded.projection.to_string(), "ab   \n     \n     \n");
    for projected in &decoded.rows {
        assert_eq!(projected.row.cells.len(), 5);
        assert_eq!(projected.row.text.len(), 5);
    }
    assert_eq!(decoded.rows[2].row.text, "     ");
    assert_eq!(decoded.rows[1].absolute_start, 6);
}

#[test]
fn frame_wire_interns_repeated_rows() {
    // A flood's shape: thousands of identical rows plus a few unique.
    let mut lines: Vec<&str> = vec!["prompt", "y"];
    lines.extend(std::iter::repeat_n("y", 4000));
    lines.push("done");
    let frame = full_frame(6, &lines);
    let wire = serde_json::to_string(&frame).unwrap();
    // One index run carries the flood; unique rows stay inline.
    assert!(
        wire.contains(r#""repeat":4001,"index":"#),
        "the flood must be one index run: {wire}"
    );
    // The palette alone costs ~7 KB; 4002 inline rows would be ~120 KB.
    assert!(
        wire.len() < 9000,
        "frame must collapse: {} bytes",
        wire.len()
    );
    let decoded: Frame = serde_json::from_str(&wire).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded.rows.len(), lines.len());
    assert_eq!(decoded.rows[0].row.text, "prompt");
    assert_eq!(decoded.rows[1].row.text, "y     ");
    assert_eq!(decoded.rows.last().unwrap().row.text, "done  ");
    // Dictionary rows share identity: repeated positions are one Arc.
    assert!(std::sync::Arc::ptr_eq(
        &decoded.rows[1].row,
        &decoded.rows[2].row
    ));
}

#[test]
fn repeated_row_references_are_bounded_and_checked() {
    let dictionary = serde_json::to_string(&full_frame(2, &["ab"]).rows[0].row).unwrap();
    for wire in [
        format!(
            r#"{{"rows":[{{"repeat":0,"index":0}}],"dictionary":[{dictionary}],"wrapped":false}}"#
        ),
        format!(r#"{{"rows":[{{"repeat":2,"index":1}}],"dictionary":[{dictionary}]}}"#),
        format!(r#"{{"rows":[{{"repeat":20000,"index":0}}],"dictionary":[{dictionary}]}}"#),
    ] {
        assert!(
            serde_json::from_str::<Frame>(&wire).is_err(),
            "frame must be refused: {wire}"
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
