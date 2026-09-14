//! Dirty-source matching shared by local and SSH providers in the captured scope.
use super::{flow::StreamSender, query::RECORD_LIMIT};
use crate::query::{ContentPlan, Evidence};
use std::{path::PathBuf, sync::Arc};
use strop_core::worker::CancelToken;
use strop_workspace::ResourceLocation;
/// An immutable dirty-source snapshot. Native identity stays separate from display.
pub struct SourceSnapshot {
    pub path: std::path::PathBuf,
    pub text: ropey::Rope,
}
/// The evidence sources a snapshot pass may consult (0063 §4): the
/// project catalog for `repo:` atoms and the mutable symbol index the
/// dirty texts overlay into.
pub(super) struct Sources<'a> {
    pub catalog: Option<&'a crate::source::catalog::ProjectCatalog>,
    pub symbols: Option<&'a mut crate::source::symbols::SymbolIndex>,
}

pub(super) fn emit_snapshots(
    root: &ResourceLocation,
    content: &ContentPlan,
    snapshots: Vec<SourceSnapshot>,
    paths: &mut Vec<PathBuf>,
    tx: &StreamSender,
    token: &CancelToken,
    mut sources: Sources<'_>,
) -> Result<(), String> {
    let Sources {
        catalog,
        ref mut symbols,
    } = sources;
    for snapshot in snapshots {
        let Ok(relative) = snapshot.path.strip_prefix(&root.path) else {
            continue;
        };
        let Some(index) = paths.iter().position(|path| path == relative) else {
            continue;
        };
        paths.swap_remove(index);
        let location = ResourceLocation {
            filesystem: root.filesystem.clone(),
            path: snapshot.path.clone(),
        };
        let label = strop_workspace::directory::display_path(relative);
        let relative = relative.to_string_lossy().into_owned();
        // Dirty text is authoritative for `kind:` evidence too: the
        // overlay replaces whatever the disk pass extracted (0063 §2).
        if let Some(index) = symbols.as_deref_mut() {
            index.overlay(&relative, &snapshot.text.to_string(), &|| {
                token.is_cancelled()
            });
        }
        for (line, text) in snapshot.text.lines().enumerate() {
            if token.is_cancelled() {
                return Err("search cancelled".into());
            }
            if text.len_bytes() > RECORD_LIMIT {
                return Err("open source line exceeds the 1 MiB search bound".into());
            }
            let text = text.to_string();
            let text: Arc<str> = text.strip_suffix('\n').unwrap_or(&text).into();
            let short: String = text.trim().chars().take(80).collect();
            let mut items = Vec::new();
            let mut expanded = text.len();
            let evidence = Evidence {
                catalog,
                symbols: symbols.as_deref(),
                path: Some(&relative),
                line: Some(line + 1),
            };
            if !content.admits(evidence, &text) {
                continue;
            }
            for hit in content.regex.find_iter(&text).take(4097) {
                let item = crate::Item {
                    badge: Some("buffer".into()),
                    text: format!("{label}:{} · {short}", line + 1),
                    payload: crate::Payload::Grep {
                        location: location.clone(),
                        line: line + 1,
                        col: hit.start() + 1,
                        match_len: hit.len(),
                        line_text: text.clone(),
                    },
                };
                expanded = expanded.saturating_add(super::flow::row_bytes(&item));
                if expanded > super::flow::BACKLOG_BYTES || items.len() >= 4096 {
                    return Err("source line exceeds 4096 matches or 4 MiB of result rows".into());
                }
                items.push(item);
            }
            tx.batch(items, token)
                .map_err(|error| error.message().to_string())?;
        }
    }
    Ok(())
}
