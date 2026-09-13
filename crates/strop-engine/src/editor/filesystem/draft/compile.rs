use super::*;
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Compiled {
    pub intents: Vec<OperationIntent>,
    pub sources: Vec<LocatedObservation>,
    pub targets: Vec<(u64, ResourceLocation)>,
}
impl Draft {
    pub(super) fn compile(
        &self,
        text: &ropey::Rope,
        token: &worker::CancelToken,
    ) -> Result<Compiled, String> {
        if !self.editable() {
            return Err("filename draft is waiting for its admitted operation".into());
        }
        if let Some(error) = &self.geometry.error {
            return Err(error.clone());
        }
        if text.len_bytes() > TEXT_LIMIT || text.len_lines() != self.geometry.rows.len() {
            return Err("filename draft exceeds its projection bounds or lost row identity".into());
        }
        let mut result = Compiled {
            intents: Vec::new(),
            sources: Vec::new(),
            targets: Vec::new(),
        };
        let mut represented = vec![0_u8; self.base.len()];
        let mut destinations = HashMap::new();
        for (line, row) in self.geometry.rows.iter().enumerate() {
            if token.is_cancelled() {
                return Err("filename compilation cancelled".into());
            }
            let slice = text.line(line);
            let text = slice
                .as_str()
                .map(std::borrow::Cow::Borrowed)
                .unwrap_or_else(|| std::borrow::Cow::Owned(slice.to_string()));
            let name = text.strip_suffix('\n').unwrap_or(&text);
            if name.is_empty() {
                if row.origin.is_some() {
                    return Err(format!("draft line {}: an existing entry needs a name; delete the whole line to stage removal", line + 1));
                }
                continue;
            }
            let decoded =
                names::decode(name).map_err(|error| format!("draft line {}: {error}", line + 1))?;
            let directory = decoded.as_os_str().as_encoded_bytes().last() == Some(&b'/');
            let relative: std::path::PathBuf = decoded.components().collect();
            let destination = ResourceLocation {
                filesystem: self.root.filesystem.clone(),
                path: self.root.path.join(relative),
            };
            if let Some(previous) = destinations.insert(destination.clone(), line) {
                return Err(format!(
                    "draft lines {} and {} name one destination",
                    previous + 1,
                    line + 1
                ));
            }
            result.targets.push((row.id, destination.clone()));
            match &row.origin {
                None => result.intents.push(OperationIntent {
                    kind: if directory {
                        OperationKind::CreateDirectory
                    } else {
                        OperationKind::CreateFile
                    },
                    source: None,
                    destination: Some(destination),
                    copy_version: CopyVersion::Stored,
                    expected_content: None,
                }),
                Some(Origin::Original(index)) => {
                    let source = self
                        .base
                        .get(*index)
                        .ok_or("filename source token is invalid")?;
                    represented[*index] = represented[*index]
                        .checked_add(1)
                        .ok_or("filename source use count overflow")?;
                    if represented[*index] != 1 {
                        return Err(format!(
                            "draft line {} duplicates an original entry identity",
                            line + 1
                        ));
                    }
                    if directory && source.observation.kind != strop_workspace::EntryKind::Directory
                    {
                        return Err(format!(
                            "draft line {} cannot change an existing entry's type",
                            line + 1
                        ));
                    }
                    if destination != source.location {
                        supported(source, line)?;
                        result.intents.push(OperationIntent {
                            kind: OperationKind::Rename,
                            source: Some(source.location.clone()),
                            destination: Some(destination),
                            copy_version: CopyVersion::Stored,
                            expected_content: None,
                        });
                        condition(&mut result.sources, source);
                    }
                }
                Some(Origin::Copy(source)) => {
                    if source.observation.kind != strop_workspace::EntryKind::File || directory {
                        return Err(format!("draft line {}: copying directories or changing source type is unsupported", line + 1));
                    }
                    if source.location.filesystem != self.root.filesystem {
                        return Err("cross-namespace draft copies are unsupported".into());
                    }
                    result.intents.push(OperationIntent {
                        kind: OperationKind::Copy,
                        source: Some(source.location.clone()),
                        destination: Some(destination),
                        copy_version: self.copy_version.unwrap_or(CopyVersion::Stored),
                        expected_content: None,
                    });
                    condition(&mut result.sources, source);
                }
            }
        }
        for (index, source) in self.base.iter().enumerate() {
            if represented[index] != 0 {
                continue;
            }
            supported(source, index)?;
            let kind = match self.removal {
                Removal::Trash => OperationKind::Trash,
                Removal::Permanent => OperationKind::Remove,
                Removal::Unchosen => return Err("removed entries need a disposition: remote Trash is unavailable; choose :fs deletes permanent explicitly".into()),
            };
            result.intents.push(OperationIntent {
                kind,
                source: Some(source.location.clone()),
                destination: None,
                copy_version: CopyVersion::Stored,
                expected_content: None,
            });
            condition(&mut result.sources, source);
        }
        if result.intents.len() > ROW_LIMIT {
            return Err("filename proposal exceeds the 512-operation bound".into());
        }
        Ok(result)
    }
}
fn condition(conditions: &mut Vec<LocatedObservation>, source: &FileReference) {
    if !conditions
        .iter()
        .any(|condition| condition.location == source.location)
    {
        conditions.push(LocatedObservation {
            location: source.location.clone(),
            value: Some(source.observation.clone()),
        });
    }
}
fn supported(source: &FileReference, line: usize) -> Result<(), String> {
    if matches!(
        source.observation.kind,
        strop_workspace::EntryKind::File | strop_workspace::EntryKind::Directory
    ) {
        Ok(())
    } else {
        Err(format!(
            "draft line {}: link and special-file mutations are unsupported",
            line + 1
        ))
    }
}

pub(super) fn matches_observed(expected: &Observation, observed: &Observation) -> bool {
    fn known<T: PartialEq>(expected: &Option<T>, observed: &Option<T>) -> bool {
        expected
            .as_ref()
            .is_none_or(|expected| observed.as_ref() == Some(expected))
    }
    expected.kind == observed.kind
        && known(&expected.identity, &observed.identity)
        && known(&expected.size, &observed.size)
        && known(&expected.modified, &observed.modified)
        && known(&expected.changed, &observed.changed)
        && known(&expected.permissions, &observed.permissions)
        && known(&expected.uid, &observed.uid)
        && known(&expected.gid, &observed.gid)
        && known(&expected.links, &observed.links)
        && known(&expected.digest, &observed.digest)
}
