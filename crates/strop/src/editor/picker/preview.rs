//! Picker preview: files read once and cached with a highlighter;
//! buffers render from the live rope.

use std::path::PathBuf;

use strop_picker::Payload;

use super::super::Editor;
use super::PreviewSource;

impl Editor {
    pub fn picker_preview(&mut self) -> Option<(String, Option<usize>, PreviewSource<'_>)> {
        let item = self.picker.as_ref()?.picker.current()?.clone();
        match item.payload {
            Payload::Buffer(i) => {
                let name = self
                    .docs
                    .get(i)?
                    .buf
                    .path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "[scratch]".into());
                let _ = i;
                Some((name, None, PreviewSource::Buffer(i)))
            }
            Payload::File(rel) => {
                let full = self.cwd.join(&rel);
                if !self.preview_ready(&full) {
                    return Some((rel.display().to_string(), None, PreviewSource::Loading));
                }
                let entry = self.previews.get_mut(&full)?;
                Some((
                    rel.display().to_string(),
                    None,
                    PreviewSource::Cached(entry),
                ))
            }
            Payload::Grep { path, line, .. } => {
                let full = self.cwd.join(&path);
                if !self.preview_ready(&full) {
                    return Some((path.display().to_string(), None, PreviewSource::Loading));
                }
                let entry = self.previews.get_mut(&full)?;
                Some((
                    path.display().to_string(),
                    Some(line),
                    PreviewSource::Cached(entry),
                ))
            }
        }
    }

    /// True when the preview is cached; otherwise kicks a worker thread
    /// (size-capped read) and reports false — the next tick picks it up.
    fn preview_ready(&mut self, path: &PathBuf) -> bool {
        if self.previews.contains_key(path) {
            return true;
        }
        if self.preview_inflight.insert(path.clone()) {
            let tx = self.preview_tx.clone();
            let p = path.clone();
            strop_trace::record_with(
                strop_trace::EventKind::JobStarted,
                || serde_json::json!({"service":"preview","path":path.to_string_lossy()}),
            );
            std::thread::spawn(move || {
                let text = std::fs::metadata(&p)
                    .ok()
                    .filter(|m| m.len() <= 512 * 1024)
                    .and_then(|_| std::fs::read_to_string(&p).ok());
                let _ = tx.send((p, text));
            });
        }
        false
    }
}
