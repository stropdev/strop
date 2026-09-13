use super::*;
use crate::Error;

pub(super) fn deserialize_frame<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Option<Arc<Frame>>, D::Error> {
    use serde::de::Error as _;
    let frame = Option::<Arc<Frame>>::deserialize(decoder)?;
    if let Some(frame) = &frame {
        frame.validate().map_err(D::Error::custom)?;
    }
    Ok(frame)
}

impl Frame {
    pub fn validate(&self) -> Result<(), Error> {
        if !self.geometry.valid()
            || self.palette.colors.len() != 256
            || self.history_rows > MAX_HISTORY_LINES
            || self.rows.len() != self.history_rows + usize::from(self.geometry.rows)
            || self.projection.len_bytes() > MAX_PROJECTION_BYTES
            || self.cursor.column >= self.geometry.columns
            || self.cursor.row >= self.geometry.rows
        {
            return Err(Error::Protocol(
                "invalid terminal frame geometry or bounds".into(),
            ));
        }
        let mut expected = self.origin;
        let mut bytes = self.projection.bytes();
        let mut storage = 0usize;
        for projected in &self.rows {
            let row = &projected.row;
            if projected.absolute_start != expected
                || row.cells.len() != usize::from(self.geometry.columns)
                || row.text.len() > MAX_ROW_BYTES
            {
                return Err(Error::Protocol(
                    "invalid terminal row correspondence".into(),
                ));
            }
            storage = storage
                .checked_add(crate::projection::cost(row))
                .ok_or(Error::Capacity("terminal row storage"))?;
            if storage > crate::projection::ROW_STORAGE_LIMIT {
                return Err(Error::Capacity("terminal row storage"));
            }
            let mut before = 0usize;
            for (column, cell) in row.cells.iter().enumerate() {
                let end = cell.end as usize;
                if end < before
                    || !row.text.is_char_boundary(end)
                    || cell.width > 2
                    || (cell.width == 0
                        && (end != before || column == 0 || row.cells[column - 1].width != 2))
                    || (cell.width != 0 && end == before)
                    || (cell.width == 2
                        && !row
                            .cells
                            .get(column + 1)
                            .is_some_and(|next| next.width == 0))
                {
                    return Err(Error::Protocol("invalid terminal cell mapping".into()));
                }
                before = end;
            }
            if before != row.text.len() {
                return Err(Error::Protocol("terminal cell text is incomplete".into()));
            }
            for byte in row.text.bytes().chain((!row.wrapped).then_some(b'\n')) {
                if bytes.next() != Some(byte) {
                    return Err(Error::Protocol(
                        "terminal text does not match its rows".into(),
                    ));
                }
                expected = expected
                    .checked_add(1)
                    .ok_or(Error::Capacity("terminal text identity"))?;
            }
        }
        if bytes.next().is_some() {
            return Err(Error::Protocol(
                "terminal projection has unowned bytes".into(),
            ));
        }
        Ok(())
    }

    /// Resolve a logical byte without walking or copying retained history.
    pub fn cell_at_byte(&self, byte: usize) -> Option<&Cell> {
        if byte >= self.projection.len_bytes() {
            return None;
        }
        let absolute = self.origin.checked_add(byte as u64)?;
        let (mut first, mut last) = (0, self.rows.len());
        while first < last {
            let middle = first + (last - first) / 2;
            if self.rows.get(middle)?.absolute_start <= absolute {
                first = middle + 1;
            } else {
                last = middle;
            }
        }
        let projected = self.rows.get(first.checked_sub(1)?)?;
        let relative = absolute.checked_sub(projected.absolute_start)?;
        let column = projected
            .row
            .cells
            .partition_point(|cell| u64::from(cell.end) <= relative);
        projected.row.cells.get(column)
    }
}
