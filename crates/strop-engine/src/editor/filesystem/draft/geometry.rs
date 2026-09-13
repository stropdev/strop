use super::*;
impl Draft {
    pub(crate) fn sync(&mut self, buffer: &strop_core::Buffer) {
        if self.revision == buffer.revision() {
            return;
        }
        let hint = self.hint.take();
        let mut history_move = false;
        for (index, change) in buffer.changes().iter().enumerate() {
            if change.revision <= self.revision {
                continue;
            }
            match change.origin {
                strop_core::ChangeOrigin::History => {
                    history_move = true;
                }
                strop_core::ChangeOrigin::System => {
                    self.geometry.error =
                        Some("filename draft was replaced outside its publication owner".into())
                }
                strop_core::ChangeOrigin::User => {
                    if history_move {
                        self.geometry.error = Some(
                            "mixed history/edit publication needs a fresh filename draft".into(),
                        );
                    }
                    if self.geometry.error.is_none() {
                        let result = buffer
                            .change_inserted(index)
                            .ok_or_else(|| "filename edit lost its history payload".to_string())
                            .and_then(|inserted| {
                                self.apply_geometry(change, inserted, hint.as_ref())
                            });
                        if let Err(error) = result {
                            self.geometry.error = Some(format!("draft line {} at revision {}: {error}; undo the ambiguous edit or discard the draft", change.edit.start_point.0 + 1, change.revision));
                        }
                    }
                    if let Some(reference) = change.history {
                        self.snapshots
                            .insert(reference.revision, self.geometry.clone());
                    }
                }
            }
        }
        if history_move {
            if let Some(geometry) = buffer
                .history()
                .committed_position()
                .and_then(|position| self.snapshots.get(&position))
            {
                self.geometry = geometry.clone();
            } else {
                self.geometry.error =
                    Some("filename history provenance is unavailable; discard the draft".into());
            }
        }
        self.revision = buffer.revision();
        if self.geometry.error.is_none() {
            debug_assert_eq!(
                self.geometry.rows.len(),
                buffer.len_lines(),
                "filename metadata follows the real rope rows"
            );
        }
    }

    fn apply_geometry(
        &mut self,
        change: &strop_core::Change,
        inserted: &str,
        hint: Option<&Hint>,
    ) -> Result<(), String> {
        let edit = &change.edit;
        let (start, column) = edit.start_point;
        let (end, end_column) = edit.old_end_point;
        let new_lines = edit
            .new_end_point
            .0
            .checked_sub(start)
            .ok_or("reversed filename edit geometry")?;
        let first = self
            .geometry
            .rows
            .get(start)
            .cloned()
            .ok_or("filename edit starts outside its rows")?;
        let last = self
            .geometry
            .rows
            .get(end)
            .cloned()
            .ok_or("filename edit ends outside its rows")?;
        if column > first.bytes || end_column > last.bytes {
            return Err("filename edit columns exceed their source rows".into());
        }
        let old_lines = end
            .checked_sub(start)
            .ok_or("reversed filename row range")?;
        let length = self
            .geometry
            .rows
            .len()
            .checked_sub(old_lines)
            .and_then(|length| length.checked_add(new_lines))
            .ok_or("filename row count overflow")?;
        if length > ROW_LIMIT + 1 {
            return Err("filename draft exceeds 512 rows".into());
        }
        let hint = hint.filter(|hint| {
            hint.revision.get().checked_add(1) == Some(change.revision.get())
                && hint.at == edit.start_byte
                && hint.removed == edit.old_end_byte - edit.start_byte
                && hint.inserted == edit.new_end_byte - edit.start_byte
        });
        if hint.is_some_and(|hint| hint.ambiguous) {
            return Err("changing several whole source entries has ambiguous provenance".into());
        }
        if start == end && new_lines == 0 {
            let mut row = first;
            row.bytes = row
                .bytes
                .checked_sub(end_column - column)
                .and_then(|length| length.checked_add(inserted.len()))
                .ok_or("filename byte count overflow")?;
            if hint.is_some_and(|hint| hint.delete_last_row) {
                row.origin = None;
            }
            self.geometry.rows.set(start, row);
        } else {
            let (first_origin, last_origin) = if start == end {
                if column == 0 {
                    (None, first.origin.clone())
                } else if end_column == last.bytes {
                    (first.origin.clone(), None)
                } else if first.origin.is_none() {
                    (None, None)
                } else {
                    return Err(
                        "splitting one source name across rows has ambiguous provenance".into(),
                    );
                }
            } else {
                (
                    if column > 0 {
                        first.origin.clone()
                    } else {
                        None
                    },
                    if hint.is_some_and(|hint| hint.delete_last_row) {
                        None
                    } else {
                        last.origin.clone()
                    },
                )
            };
            let pieces: Vec<_> = inserted.split('\n').collect();
            if pieces.len() != new_lines + 1 {
                return Err("filename payload and edit geometry disagree".into());
            }
            let suffix = last.bytes - end_column;
            let mut replacement = Vector::new();
            if new_lines == 0 {
                if first_origin.is_some() && last_origin.is_some() && first_origin != last_origin {
                    return Err(
                        "joining different source entries loses their one-row identity".into(),
                    );
                }
                let origin = first_origin.or(last_origin);
                let mut row = if column > 0 { first } else { last };
                row.origin = origin;
                row.bytes = column + pieces[0].len() + suffix;
                replacement.push_back(row);
            } else {
                for (index, piece) in pieces.iter().enumerate() {
                    let bytes = piece.len()
                        + if index == 0 { column } else { 0 }
                        + if index == new_lines { suffix } else { 0 };
                    let row = if index == 0 && first_origin.is_some() {
                        Row {
                            id: first.id,
                            origin: first_origin.clone(),
                            bytes,
                        }
                    } else if index == new_lines && last_origin.is_some() {
                        Row {
                            id: last.id,
                            origin: last_origin.clone(),
                            bytes,
                        }
                    } else {
                        self.allocate_row(None, bytes)?
                    };
                    replacement.push_back(row);
                }
            }
            let mut removed = self.geometry.rows.split_off(start);
            let tail = removed.split_off(old_lines + 1);
            self.geometry.rows.append(replacement);
            self.geometry.rows.append(tail);
        }
        if let Some((prefix, register, count)) = hint.and_then(|hint| hint.paste.as_ref()) {
            self.apply_paste(start + prefix, register, *count)?;
        }
        Ok(())
    }

    fn apply_paste(
        &mut self,
        start: usize,
        register: &FileRegister,
        count: usize,
    ) -> Result<(), String> {
        let rows = register
            .rows
            .len()
            .checked_mul(count)
            .ok_or("filename paste count overflow")?;
        if rows > ROW_LIMIT
            || start
                .checked_add(rows)
                .is_none_or(|end| end > self.geometry.rows.len())
        {
            return Err("filename paste provenance exceeds its rows".into());
        }
        for offset in 0..rows {
            let entry = &register.rows[offset % register.rows.len()];
            if self.geometry.rows[start + offset].origin.is_some() {
                return Err("linewise paste did not form a separate filename row".into());
            }
            let origin = if let Some(entry) = entry {
                if entry.source.location.filesystem != self.root.filesystem {
                    return Err(
                        "cross-namespace filename transfers require an explicit operation".into(),
                    );
                }
                if entry.source.draft == self.id
                    && self
                        .base
                        .get(entry.source.entry)
                        .is_some_and(|source| source.as_ref() == entry.source.as_ref())
                {
                    let exists = self
                        .geometry
                        .rows
                        .iter()
                        .any(|row| row.origin == Some(Origin::Original(entry.source.entry)));
                    Some(if exists {
                        Origin::Copy(entry.source.clone())
                    } else {
                        Origin::Original(entry.source.entry)
                    })
                } else {
                    if entry.cut {
                        return Err("cross-directory cuts require the explicit Move action".into());
                    }
                    Some(Origin::Copy(entry.source.clone()))
                }
            } else {
                None
            };
            if let Some(row) = self.geometry.rows.get_mut(start + offset) {
                row.origin = origin;
            }
        }
        Ok(())
    }
}
