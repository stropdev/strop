//! Incremental immutable row/text publication. Old inspection snapshots remain
//! valid while the worker trims history or replaces the live screen tail.
use crate::model::{ProjectedRow, Row, MAX_HISTORY_LINES, MAX_PROJECTION_BYTES};
use crate::Error;
use imbl::Vector;
use ropey::Rope;
use std::sync::Arc;

pub(crate) const ROW_STORAGE_LIMIT: usize = 32 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Projection {
    pub rows: Vector<ProjectedRow>,
    pub rope: Rope,
    pub origin: u64,
    pub history: usize,
    storage: usize,
}

impl Projection {
    pub fn reset(&mut self) -> Result<(), Error> {
        self.origin = self
            .origin
            .checked_add(self.rope.len_bytes() as u64)
            .and_then(|origin| origin.checked_add(1))
            .ok_or(Error::Capacity("terminal text identity exhausted"))?;
        self.rows.clear();
        self.rope = Rope::new();
        self.history = 0;
        self.storage = 0;
        Ok(())
    }

    pub fn replace(
        &mut self,
        drop_history: usize,
        added_history: impl IntoIterator<Item = Result<Arc<Row>, Error>>,
        screen: impl IntoIterator<Item = Arc<Row>>,
    ) -> Result<(), Error> {
        if drop_history > self.history {
            return Err(Error::Protocol(
                "history retirement exceeds owned projection".into(),
            ));
        }
        let screen_start = self
            .rows
            .get(self.history)
            .map_or(self.rope.len_bytes(), |entry| {
                entry.absolute_start.saturating_sub(self.origin) as usize
            });
        self.rope.remove(self.rope.byte_to_char(screen_start)..);
        while self.rows.len() > self.history {
            if let Some(entry) = self.rows.pop_back() {
                self.storage -= cost(&entry.row);
            }
        }
        self.drop_history(drop_history)?;
        for row in added_history {
            self.push(row?)?;
            self.history += 1;
            self.trim()?;
        }
        for row in screen {
            self.push(row)?;
        }
        self.trim()?;
        if self.storage > ROW_STORAGE_LIMIT || self.rope.len_bytes() > MAX_PROJECTION_BYTES {
            return Err(Error::Capacity(
                "visible terminal projection exceeds its byte budget",
            ));
        }
        Ok(())
    }

    fn push(&mut self, row: Arc<Row>) -> Result<(), Error> {
        let absolute_start = self
            .origin
            .checked_add(self.rope.len_bytes() as u64)
            .ok_or(Error::Capacity("terminal text identity exhausted"))?;
        let new_bytes = row
            .text
            .len()
            .checked_add(usize::from(!row.wrapped))
            .ok_or(Error::Capacity("terminal row size overflow"))?;
        if new_bytes > MAX_PROJECTION_BYTES {
            return Err(Error::Capacity("terminal row exceeds projection budget"));
        }
        self.storage = self
            .storage
            .checked_add(cost(&row))
            .ok_or(Error::Capacity("terminal row storage overflow"))?;
        self.rope.insert(self.rope.len_chars(), &row.text);
        if !row.wrapped {
            self.rope.insert(self.rope.len_chars(), "\n");
        }
        self.rows.push_back(ProjectedRow {
            absolute_start,
            row,
        });
        Ok(())
    }

    fn trim(&mut self) -> Result<(), Error> {
        while self.history > 0
            && (self.history > MAX_HISTORY_LINES
                || self.storage > ROW_STORAGE_LIMIT
                || self.rope.len_bytes() > MAX_PROJECTION_BYTES)
        {
            self.drop_history(1)?;
        }
        Ok(())
    }

    fn drop_history(&mut self, count: usize) -> Result<(), Error> {
        if count == 0 {
            return Ok(());
        }
        let bytes = self.rows.get(count).map_or(self.rope.len_bytes(), |entry| {
            entry.absolute_start.saturating_sub(self.origin) as usize
        });
        self.origin = self
            .origin
            .checked_add(bytes as u64)
            .ok_or(Error::Capacity("terminal text identity exhausted"))?;
        self.rope.remove(..self.rope.byte_to_char(bytes));
        for _ in 0..count {
            if let Some(entry) = self.rows.pop_front() {
                self.storage -= cost(&entry.row);
            }
        }
        self.history -= count;
        Ok(())
    }
}

pub(crate) fn cost(row: &Row) -> usize {
    row.text.capacity()
        + row.cells.capacity() * std::mem::size_of::<crate::model::Cell>()
        + std::mem::size_of::<Row>()
        + 64
}

pub(crate) mod rope_serde {
    use super::MAX_PROJECTION_BYTES;
    use ropey::{Rope, RopeBuilder};
    use serde::de::{Error, SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};
    use std::fmt;

    pub fn serialize<S: Serializer>(rope: &Rope, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(rope.chunks())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Rope, D::Error> {
        struct Chunks;
        impl<'de> Visitor<'de> for Chunks {
            type Value = Rope;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("bounded terminal text chunks")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut chunks: A) -> Result<Rope, A::Error> {
                let mut builder = RopeBuilder::new();
                let mut bytes = 0usize;
                while let Some(chunk) = chunks.next_element::<String>()? {
                    bytes = bytes
                        .checked_add(chunk.len())
                        .ok_or_else(|| A::Error::custom("terminal text size overflow"))?;
                    if bytes > MAX_PROJECTION_BYTES {
                        return Err(A::Error::custom("terminal text exceeds projection budget"));
                    }
                    builder.append(&chunk);
                }
                Ok(builder.finish())
            }
        }
        deserializer.deserialize_seq(Chunks)
    }
}
