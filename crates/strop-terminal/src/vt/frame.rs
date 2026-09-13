use super::{check, ffi, Emission, Vt};
use crate::model::*;
use crate::Error;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;

impl Vt {
    pub fn snapshot(&mut self) -> Result<(Arc<Frame>, Emission), Error> {
        let mut state = self.state()?;
        let mut denied_geometry = false;
        if state.cols != u64::from(self.geometry.columns)
            || state.rows != u64::from(self.geometry.rows)
        {
            // A child cannot resize the host pane. Restore the admitted virtual
            // geometry on this same owner, and report the rejected host effect.
            // SAFETY: live unique terminal and previously validated geometry.
            check("constrain geometry", unsafe {
                ffi::strop_vt_resize(
                    self.handle.as_ptr(),
                    self.geometry.columns,
                    self.geometry.rows,
                )
            })?;
            self.rebuild = true;
            denied_geometry = true;
            state = self.state()?;
        }
        if state.cols != u64::from(self.geometry.columns)
            || state.rows != u64::from(self.geometry.rows)
            || state.cursor_x >= state.cols
            || state.cursor_y >= state.rows
        {
            return Err(Error::Protocol(
                "native terminal geometry is inconsistent".into(),
            ));
        }
        let history =
            usize::try_from(state.history).map_err(|_| Error::Capacity("native history count"))?;
        let alternate = state.alternate != 0;
        let mut rebuild = self.rebuild || alternate != self.alternate;
        let mut native_drop = 0usize;
        let mut added = history;
        if !rebuild && self.native_history > 0 {
            if let Ok(anchor) = usize::try_from(state.previous_history_anchor) {
                match (
                    self.native_history.checked_sub(anchor + 1),
                    history.checked_sub(anchor + 1),
                ) {
                    (Some(retired), Some(new)) => {
                        native_drop = retired;
                        added = new;
                    }
                    _ => rebuild = true,
                }
            } else {
                rebuild = true;
            }
        }
        if added >= MAX_HISTORY_LINES {
            rebuild = true;
        }
        let drop_history;
        if rebuild {
            self.projection.reset()?;
            added = history.min(MAX_HISTORY_LINES);
            drop_history = 0;
        } else {
            let omitted = self.native_history.saturating_sub(self.projection.history);
            drop_history = native_drop
                .saturating_sub(omitted)
                .min(self.projection.history);
        }
        let mut dirty = vec![0; usize::from(self.geometry.rows)];
        // SAFETY: initialized output storage is exactly the admitted row count.
        check("dirty rows", unsafe {
            ffi::strop_vt_dirty_rows(self.handle.as_ptr(), dirty.as_mut_ptr(), dirty.len())
        })?;
        let mut screen = Vec::with_capacity(usize::from(self.geometry.rows));
        for row in 0..self.geometry.rows {
            let cached = self
                .screen
                .get(usize::from(row))
                .filter(|_| !rebuild && dirty[usize::from(row)] == 0);
            screen.push(match cached {
                Some(row) => Arc::clone(row),
                None => read_row(self.handle, i32::from(row), self.geometry.columns)?,
            });
        }
        let handle = self.handle;
        let columns = self.geometry.columns;
        let additions = (-(added as i32)..0).map(|row| read_row(handle, row, columns));
        self.projection
            .replace(drop_history, additions, screen.iter().cloned())?;
        self.history_limited |= native_drop > 0 || self.projection.history < history;
        let palette = self.read_palette()?;
        let shape = match state.cursor_style {
            0 => CursorShape::Bar,
            1 => CursorShape::Block,
            2 => CursorShape::Underline,
            3 => CursorShape::HollowBlock,
            _ => return Err(Error::Protocol("unknown native cursor style".into())),
        };
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(Error::Capacity("terminal frame identity exhausted"))?;
        let frame = Arc::new(Frame {
            session: self.session,
            revision: self.revision,
            geometry: self.geometry,
            alternate,
            cursor: Cursor {
                column: state.cursor_x as u16,
                row: state.cursor_y as u16,
                visible: state.visible != 0,
                blinking: state.blinking != 0,
                shape,
            },
            palette,
            history_rows: self.projection.history,
            available_history_rows: history,
            history_limited: self.history_limited,
            origin: self.projection.origin,
            rows: self.projection.rows.clone(),
            projection: self.projection.rope.clone(),
        });
        // SAFETY: all native borrows have been copied; mark the new history
        // frontier only after a complete owned frame has been assembled.
        check("history frontier", unsafe {
            ffi::strop_vt_mark_history(self.handle.as_ptr())
        })?;
        self.native_history = history;
        self.alternate = alternate;
        self.rebuild = false;
        self.screen = screen;
        let mut emission = self.take_events()?;
        if denied_geometry {
            emission.effects.push(Effect::HostControlDenied);
        }
        Ok((frame, emission))
    }

    fn state(&self) -> Result<ffi::State, Error> {
        let mut state = ffi::State::default();
        // SAFETY: live native handle and initialized repr(C) output storage.
        check("snapshot state", unsafe {
            ffi::strop_vt_state(self.handle.as_ptr(), &mut state)
        })?;
        Ok(state)
    }

    fn read_palette(&mut self) -> Result<Arc<Palette>, Error> {
        let mut native = ffi::Palette::default();
        // SAFETY: bridge initializes the entire fixed-size palette value.
        check("palette", unsafe {
            ffi::strop_vt_palette(self.handle.as_ptr(), &mut native)
        })?;
        let rgb = |value: ffi::Rgb| Rgb {
            red: value.r,
            green: value.g,
            blue: value.b,
        };
        if let Some(previous) = self.palette.as_ref().filter(|previous| {
            previous.foreground == rgb(native.foreground)
                && previous.background == rgb(native.background)
                && previous
                    .colors
                    .iter()
                    .copied()
                    .eq(native.colors.iter().copied().map(rgb))
        }) {
            return Ok(Arc::clone(previous));
        }
        let palette = Arc::new(Palette {
            foreground: rgb(native.foreground),
            background: rgb(native.background),
            colors: native.colors.into_iter().map(rgb).collect(),
        });
        self.palette = Some(Arc::clone(&palette));
        Ok(palette)
    }
}

fn read_row(handle: NonNull<c_void>, row: i32, columns: u16) -> Result<Arc<Row>, Error> {
    let mut text = String::with_capacity(usize::from(columns));
    let mut cells = Vec::with_capacity(usize::from(columns));
    let mut wrapped = false;
    for column in 0..columns {
        let mut cell = ffi::Cell::default();
        // SAFETY: single-worker read from an admitted screen/history coordinate;
        // the bridge retains no Rust pointer or native borrow after returning.
        check("cell", unsafe {
            ffi::strop_vt_cell(handle.as_ptr(), row, column, &mut cell)
        })?;
        if cell.width > 2 || cell.graphemes as usize > MAX_GLYPH_CODEPOINTS {
            return Err(Error::Capacity("terminal cell representation"));
        }
        if cell.width != 0 {
            match cell.graphemes {
                0 => text.push(' '),
                1 => push_scalar(&mut text, cell.codepoint)?,
                count => {
                    let mut glyphs = Vec::<u32>::with_capacity(count as usize);
                    let mut written = 0;
                    // SAFETY: writable spare capacity is passed with its exact
                    // bound; the pinned API initializes written u32 elements.
                    check("graphemes", unsafe {
                        ffi::strop_vt_graphemes(
                            handle.as_ptr(),
                            row,
                            column,
                            glyphs.as_mut_ptr(),
                            glyphs.capacity(),
                            &mut written,
                        )
                    })?;
                    if written > glyphs.capacity() {
                        return Err(Error::Protocol(
                            "native grapheme length exceeded capacity".into(),
                        ));
                    }
                    // SAFETY: the successful bounded native call initialized exactly written elements.
                    unsafe { glyphs.set_len(written) };
                    for codepoint in glyphs {
                        push_scalar(&mut text, codepoint)?;
                    }
                }
            }
        }
        if text.len() > MAX_ROW_BYTES {
            return Err(Error::Capacity("terminal row text"));
        }
        cells.push(Cell {
            end: text.len() as u32,
            width: cell.width as u8,
            style: Style {
                foreground: color(cell.fg_kind, cell.fg)?,
                background: color(cell.bg_kind, cell.bg)?,
                bold: cell.flags & 1 != 0,
                italic: cell.flags & 2 != 0,
                underline: cell.flags & 4 != 0,
                inverse: cell.flags & 8 != 0,
                faint: cell.flags & 16 != 0,
                invisible: cell.flags & 32 != 0,
                strikethrough: cell.flags & 64 != 0,
            },
        });
        wrapped = cell.wrapped != 0;
    }
    Ok(Arc::new(Row {
        text,
        cells,
        wrapped,
    }))
}
fn push_scalar(text: &mut String, codepoint: u32) -> Result<(), Error> {
    let ch = char::from_u32(codepoint)
        .filter(|ch| !ch.is_control())
        .ok_or_else(|| {
            Error::Protocol("native terminal cell contains a non-printable scalar".into())
        })?;
    text.push(ch);
    Ok(())
}
fn color(kind: u32, value: u32) -> Result<Color, Error> {
    match kind {
        0 => Ok(Color::Default),
        1 if value <= 255 => Ok(Color::Indexed(value as u8)),
        2 if value <= 0xffffff => Ok(Color::Rgb(Rgb {
            red: (value >> 16) as u8,
            green: (value >> 8) as u8,
            blue: value as u8,
        })),
        _ => Err(Error::Protocol("native cell has an invalid color".into())),
    }
}
