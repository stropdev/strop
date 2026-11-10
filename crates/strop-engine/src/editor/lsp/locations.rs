//! Rows for a location list — references, implementations, type
//! definitions (0093 §5): `path:line  source text`, the shape of a grep
//! row, so a list of places reads as places and filters by what is on
//! the line.

use strop_lsp::ServerLocation;
use strop_picker::{Item, Payload};
use strop_workspace::Filesystem;

use crate::editor::Editor;

/// Source text shown per row. The row is one screen line; the preview
/// pane has the rest.
const SOURCE_CHARS: usize = 200;

fn shown(line: &str) -> String {
    line.trim().chars().take(SOURCE_CHARS).collect()
}

/// The line an open document holds at a server position. Unsaved text is
/// what the user is looking at, so it wins over what the disk says.
fn open_line(editor: &Editor, location: &ServerLocation) -> Option<String> {
    if location.doc.filesystem != Filesystem::Local {
        return None;
    }
    let (_, document) = editor.docs.iter().find(|(_, document)| {
        document.remote_metadata().is_none()
            && document.buf.path.as_deref() == Some(location.doc.path.as_path())
    })?;
    let line = location.position.line.get();
    (line < document.buf.len_lines()).then(|| shown(&document.buf.line_text(line)))
}

/// One row per location. `sources` carries the line each location points
/// at as the language client read it from disk (empty when it could not);
/// `sources` shorter than `items` — an older recording — reads as unread.
pub(crate) fn rows(editor: &Editor, items: Vec<ServerLocation>, sources: Vec<String>) -> Vec<Item> {
    let mut sources = sources.into_iter();
    items
        .into_iter()
        .map(|location| {
            let disk = sources.next().unwrap_or_default();
            let source = open_line(editor, &location).unwrap_or_else(|| shown(&disk));
            let line = location.position.line.get() + 1;
            let col = location.position.column.get() + 1;
            let remote_label = location.doc.label();
            match location.doc.filesystem {
                Filesystem::Local => {
                    let path = &location.doc.path;
                    let relative = path.strip_prefix(&editor.cwd).unwrap_or(path);
                    let label = strop_picker::display_path(relative);
                    Item {
                        badge: None,
                        text: format!("{label}:{line}  {source}"),
                        payload: Payload::Grep {
                            location: strop_workspace::ResourceLocation::local(path.clone()),
                            line,
                            col,
                            match_len: 1,
                            line_text: source.into(),
                        },
                    }
                }
                Filesystem::Remote(endpoint) => Item {
                    badge: None,
                    text: format!("{remote_label}:{line}"),
                    payload: Payload::Remote {
                        endpoint,
                        path: location.doc.path,
                        line,
                        col,
                    },
                },
            }
        })
        .collect()
}
