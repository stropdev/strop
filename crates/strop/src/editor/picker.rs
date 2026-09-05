//! Picker glue: the editor side of strop-picker. Workers post onto the
//! event loop (0001 §5.6); the editor drains them between keystrokes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use strop_picker::{spawn_files, GrepWorker, Item, Kind, Payload, Picker, PickerMsg};
use strop_syntax::Highlighter;

use super::{Editor, Key};

impl PickerGlue {
    /// A picker over editor-computed items (diagnostics; 0009 §3 Space d).
    pub fn diagnostics(picker: Picker) -> Self {
        Self {
            picker,
            tx: None,
            rx: None,
            grep_worker: None,
        }
    }
}

pub struct PickerGlue {
    pub picker: Picker,
    /// Sender stays alive for grep respawns (kill + respawn per keystroke).
    tx: Option<Sender<PickerMsg>>,
    rx: Option<Receiver<PickerMsg>>,
    grep_worker: Option<GrepWorker>,
}

impl Editor {
    pub fn open_picker(&mut self, kind: Kind) {
        let (tx, rx) = channel();
        let mut tx = Some(tx);
        let (items, streaming, rx) = match kind {
            Kind::Buffers => {
                // MRU-ordered (0003 §2): most-recent *other* buffer first,
                // vim's alternate-file instinct.
                let items = self
                    .mru
                    .iter()
                    .map(|&i| {
                        let name = self
                            .doc(i)
                            .buf
                            .path
                            .clone()
                            .unwrap_or_else(|| "[scratch]".into());
                        Item {
                            text: name,
                            payload: Payload::Buffer(i),
                        }
                    })
                    .collect();
                (items, false, None)
            }
            Kind::Files => {
                spawn_files(self.cwd.clone(), tx.take().expect("fresh channel"));
                (vec![], true, Some(rx))
            }
            Kind::Grep | Kind::Replace => (vec![], false, Some(rx)),
            Kind::Diagnostics | Kind::Locations => {
                unreachable!("location lists use PickerGlue::diagnostics")
            }
        };
        let picker = Picker::new(kind, items, streaming);
        self.picker = Some(PickerGlue {
            picker,
            tx,
            rx,
            grep_worker: None,
        });
    }

    pub fn close_picker(&mut self) {
        self.picker = None;
    }

    pub fn picker_open(&self) -> bool {
        self.picker.is_some()
    }

    /// Keystrokes while a picker is open. The input line is insert-mode
    /// semantics (0003 §1); nav is arrows / ctrl-n,p / tab.
    pub(crate) fn feed_picker(&mut self, key: Key) {
        let Some(glue) = &mut self.picker else { return };
        let replace = glue.picker.kind == Kind::Replace;
        match key {
            Key::Esc => {
                // rootle's input boxes: Esc enters vim normal mode on the
                // field; Esc again closes the picker
                if glue.picker.input_normal() {
                    self.close_picker();
                } else {
                    glue.picker.enter_normal();
                }
            }
            Key::Enter if replace => self.apply_replace(),
            Key::Enter => {
                let payload = glue.picker.current().map(|i| i.payload.clone());
                self.picker = None;
                if let Some(p) = payload {
                    self.accept_picker(p);
                }
            }
            // 0007 §2: Tab cycles the two fields; results nav is
            // arrows / ctrl-n,p while a field has focus
            Key::Tab | Key::Backtab if replace => glue.picker.toggle_field(),
            Key::CtrlX if replace => glue.picker.toggle_excluded(),
            Key::CtrlD if replace => glue.picker.toggle_file_excluded(),
            Key::CtrlD => {}
            Key::CtrlX | Key::CtrlO => {}
            Key::Backspace => {
                if glue.picker.input_normal() {
                    glue.picker.normal_key('h'); // vim: bs in normal = h
                } else if replace && glue.picker.field == strop_picker::Field::Replace {
                    glue.picker.pop_replace_char();
                } else {
                    glue.picker.pop_char();
                    self.picker_input_changed();
                }
            }
            Key::CtrlR | Key::CtrlW => {}
            Key::CtrlU | Key::CtrlF | Key::CtrlB | Key::CtrlCaret => {}
            Key::Up => glue.picker.move_by(-1),
            Key::Down => glue.picker.move_by(1),
            Key::Tab => glue.picker.move_by(1),
            Key::Backtab => glue.picker.move_by(-1),
            // arrows: Up/Down walk results, Left/Right move the caret
            Key::Left => glue.picker.caret_left(),
            Key::Right => glue.picker.caret_right(),
            // picker normal mode (Esc): the field is one line, so h/l
            // own the caret and j/k walk the results — the muscle
            // memory you bring from the buffer
            Key::Char('j') if glue.picker.input_normal() => glue.picker.move_by(1),
            Key::Char('k') if glue.picker.input_normal() => glue.picker.move_by(-1),
            Key::Char(c) => {
                if glue.picker.input_normal() {
                    // modal editing on the field (0003 §1); x/X change
                    // the text → respawn
                    if glue.picker.normal_key(c) {
                        self.picker_input_changed();
                    }
                } else if replace && glue.picker.field == strop_picker::Field::Replace {
                    glue.picker.push_replace_char(c);
                } else {
                    glue.picker.push_char(c);
                    self.picker_input_changed();
                }
            }
        }
    }

    fn picker_input_changed(&mut self) {
        let Some(glue) = &mut self.picker else { return };
        if matches!(glue.picker.kind, Kind::Grep | Kind::Replace) {
            // rg filters; kill + respawn per keystroke (worker is cheap).
            // A fresh channel per respawn: the old worker's messages (incl.
            // its trailing Done) fail to send on the dropped receiver, so
            // stale generations can't race the new one.
            let pattern = glue.picker.input.text.clone();
            let cwd = self.cwd.clone();
            glue.picker.error = None;
            glue.grep_worker = None; // drop kills the old rg
            glue.picker.items.clear();
            glue.picker.rows.clear(); // stale item indices must never render
            glue.picker.excluded.clear(); // item indices die with the respawn
            let (tx, rx) = channel();
            glue.tx = Some(tx.clone());
            glue.rx = Some(rx);
            glue.grep_worker = GrepWorker::spawn(&pattern, &cwd, tx);
            glue.picker.streaming = glue.grep_worker.is_some();
        } else {
            glue.picker.refilter();
        }
    }

    /// Drain worker messages (called from the event loop each tick).
    pub fn drain_picker(&mut self) {
        let mut done = false;
        if let Some(glue) = &mut self.picker {
            if let Some(rx) = &glue.rx {
                let mut items = Vec::new();
                while let Ok(msg) = rx.try_recv() {
                    match msg {
                        PickerMsg::Items(batch) => items.extend(batch),
                        PickerMsg::Error(e) => glue.picker.error = Some(e),
                        PickerMsg::Done => done = true,
                    }
                }
                if !items.is_empty() {
                    glue.picker.append(items);
                }
            }
            if done {
                glue.picker.streaming = false;
            }
        }
        self.drain_previews();
    }

    /// Drain preview worker results (file reads happen off the render
    /// path — 0001 §3).
    fn drain_previews(&mut self) {
        while let Ok((path, text)) = self.preview_rx.try_recv() {
            self.preview_inflight.remove(&path);
            if let Some(text) = text {
                let rope = ropey::Rope::from_str(&text);
                let hl = Highlighter::for_path(&path.display().to_string());
                self.previews.insert(path, PreviewEntry { rope, hl });
            } else {
                // unreadable: cache the miss so we don't respawn per frame
                self.previews.insert(
                    path,
                    PreviewEntry {
                        rope: ropey::Rope::from_str(""),
                        hl: None,
                    },
                );
            }
        }
    }

    fn accept_picker(&mut self, payload: Payload) {
        match payload {
            Payload::File(rel) => {
                let path = self.cwd.join(&rel);
                match self.open_buffer(&path.display().to_string()) {
                    Ok(()) => {}
                    Err(e) => self.message = format!("open {}: {e}", rel.display()),
                }
            }
            Payload::Buffer(i) => {
                if self.docs.get(i).is_some() {
                    self.switch_to(i);
                    self.set_head(0);
                    self.view_mut().view_top = 0;
                }
            }
            Payload::Grep {
                path, line, col, ..
            } => {
                let full = self.cwd.join(&path);
                if let Err(e) = self.open_buffer(&full.display().to_string()) {
                    self.message = format!("open {}: {e}", path.display());
                    return;
                }
                let start = self.buf().line_start(line.saturating_sub(1));
                self.set_head(self.buf().clamp_boundary(start + col.saturating_sub(1)));
                self.clamp_cursor();
            }
        }
    }

    /// Replace mode Enter: apply every accepted hit. One undo revision
    /// per touched buffer (0007 §4); lines that drifted since the search
    /// are skipped and counted, never silently rewritten.
    fn apply_replace(&mut self) {
        let Some(glue) = self.picker.take() else {
            return;
        };
        let replacement = glue.picker.replace_input.text.clone();
        let mut by_path: HashMap<PathBuf, Vec<(usize, usize, usize, String)>> = HashMap::new();
        for it in glue.picker.accepted() {
            if let Payload::Grep {
                path,
                line,
                col,
                match_len,
                line_text,
            } = &it.payload
            {
                by_path.entry(path.clone()).or_default().push((
                    *line,
                    *col,
                    *match_len,
                    line_text.clone(),
                ));
            }
        }
        if by_path.is_empty() {
            self.message = "replace: no matches".into();
            return;
        }
        let mut files = 0usize;
        let mut applied = 0usize;
        let mut stale = 0usize;
        for (rel, mut hits) in by_path {
            // bottom-up: earlier hits' offsets stay valid while applying
            hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
            let full = self.cwd.join(&rel);
            let (f, a, s) = if let Some(bi) = self.buffer_index_of(&full) {
                self.replace_in_buffer(bi, &hits, &replacement)
            } else {
                Self::replace_in_file(&full, &hits, &replacement)
            };
            files += f;
            applied += a;
            stale += s;
        }
        let stale_msg = if stale > 0 {
            format!(" · {stale} stale skipped")
        } else {
            String::new()
        };
        self.message =
            format!("replaced {applied} in {files} files (u per buffer to undo){stale_msg}");
    }

    /// Open-buffer index for an absolute path, if loaded.
    fn buffer_index_of(&self, abs: &std::path::Path) -> Option<strop_core::id::DocumentId> {
        self.docs
            .iter()
            .find(|(_, d)| {
                let b = &d.buf;
                b.path
                    .as_deref()
                    .map(|p| {
                        let p = std::path::Path::new(p);
                        let buf_abs = if p.is_absolute() {
                            p.to_path_buf()
                        } else {
                            self.cwd.join(p)
                        };
                        buf_abs == abs
                            || buf_abs.canonicalize().ok().as_ref() == Some(&abs.to_path_buf())
                    })
                    .unwrap_or(false)
            })
            .map(|(id, _)| id)
    }

    /// Verified, bottom-up replacement in an open buffer: one history
    /// transaction → one `u` reverts this buffer's replacements.
    fn replace_in_buffer(
        &mut self,
        bi: strop_core::id::DocumentId,
        hits: &[(usize, usize, usize, String)],
        replacement: &str,
    ) -> (usize, usize, usize) {
        let mut applied = 0;
        let mut stale = 0;
        let buf = &mut self.doc_mut(bi).buf;
        if buf.readonly {
            return (0, 0, hits.len());
        }
        buf.history.begin();
        for (line, col, match_len, expected) in hits {
            if *line == 0 || *line > buf.len_lines() {
                stale += 1;
                continue;
            }
            // verify the matched *span*, not the whole line: same-line
            // hits stay verifiable as earlier (rightward) ones apply
            let (s, e) = strop_picker::replace_span(expected, *col, *match_len);
            let ls = buf.line_start(line - 1);
            let abs_s = ls + s;
            let abs_e = (ls + e).min(buf.len_bytes());
            if abs_s > abs_e || buf.rope.slice(abs_s..abs_e) != expected[s..e] {
                stale += 1;
                continue;
            }
            buf.delete(strop_core::Range::charwise(abs_s, abs_e));
            buf.insert(abs_s, replacement);
            applied += 1;
        }
        buf.history.commit();
        ((applied > 0) as usize, applied, stale)
    }

    /// Replace hits in a file that isn't open: verified line-by-line,
    /// mtime-guarded, written atomically (temp + rename) — never a silent
    /// partial write (0007 §4). Returns (touched, applied, stale).
    fn replace_in_file(
        path: &std::path::Path,
        hits: &[(usize, usize, usize, String)],
        replacement: &str,
    ) -> (usize, usize, usize) {
        let Ok(meta) = std::fs::metadata(path) else {
            return (0, 0, hits.len());
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return (0, 0, hits.len());
        };
        let line_offsets: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        let mut content = text.clone();
        let mut applied = 0;
        let mut stale = 0;
        for (line, col, match_len, expected) in hits {
            let Some(&ls) = line_offsets.get(line.saturating_sub(1)) else {
                stale += 1;
                continue;
            };
            // verify the matched *span* (same-line hits stay verifiable
            // as rightward ones apply), in content: bottom-up order keeps
            // smaller offsets valid
            let (s, e) = strop_picker::replace_span(expected, *col, *match_len);
            let abs_s = ls + s;
            let abs_e = ls + e;
            if content.get(abs_s..abs_e) != expected.get(s..e) {
                stale += 1;
                continue;
            }
            content.replace_range(abs_s..abs_e, replacement);
            applied += 1;
        }
        if applied == 0 {
            return (0, 0, stale);
        }
        // mtime guard: somebody rewrote the file under the search — skip
        let moved =
            std::fs::metadata(path).ok().and_then(|m| m.modified().ok()) != meta.modified().ok();
        if moved {
            return (0, 0, applied + stale);
        }
        let tmp = path.with_file_name(format!(
            "{}.strop-tmp",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("strop")
        ));
        if std::fs::write(&tmp, &content).is_err() || std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return (0, 0, applied + stale);
        }
        (1, applied, stale)
    }
    /// Preview payload for the render layer: (title, focus line, rope).
    /// Files are read once and cached with a highlighter; buffers render
    /// from the live rope.
    pub fn picker_preview(&mut self) -> Option<(String, Option<usize>, PreviewSource<'_>)> {
        let item = self.picker.as_ref()?.picker.current()?.clone();
        match item.payload {
            Payload::Buffer(i) => {
                let name = self
                    .docs
                    .get(i)?
                    .buf
                    .path
                    .clone()
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

pub struct PreviewEntry {
    pub rope: ropey::Rope,
    pub hl: Option<Highlighter>,
}

pub enum PreviewSource<'a> {
    /// Live document, highlighted with its own highlighter.
    Buffer(strop_core::id::DocumentId),
    Cached(&'a mut PreviewEntry),
    /// Worker read still in flight (or unreadable); render shows a
    /// placeholder, never blocks.
    Loading,
}

pub type Previews = HashMap<PathBuf, PreviewEntry>;

#[cfg(test)]
mod replace_tests {
    use super::*;
    use strop_core::Buffer;

    /// One hit tuple: (line, col, match_len, expected line text).
    fn hit(line: usize, col: usize, len: usize, text: &str) -> (usize, usize, usize, String) {
        (line, col, len, text.to_string())
    }

    #[test]
    fn buffer_replace_applies_bottom_up_and_verifies() {
        let mut e = Editor::new(Buffer::from_text("foo bar foo\n"));
        let hits = vec![
            hit(1, 9, 3, "foo bar foo"),
            hit(1, 1, 3, "foo bar foo"),
            hit(1, 5, 3, "WRONG — stale line"),
        ];
        let (touched, applied, stale) = e.replace_in_buffer(e.first_doc(), &hits, "baz");
        assert_eq!((touched, applied, stale), (1, 2, 1));
        assert_eq!(e.buf().rope.to_string(), "baz bar baz\n");
        // one undo revision for the whole apply (0007 §4)
        e.undo();
        assert_eq!(e.buf().rope.to_string(), "foo bar foo\n");
    }

    #[test]
    fn file_replace_writes_atomically_and_verifies() {
        let dir = std::env::temp_dir().join(format!("strop-replace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "alpha foo\nbeta foo\ngamma\n").unwrap();
        let hits = vec![
            hit(2, 6, 3, "beta foo"),
            hit(1, 7, 3, "alpha foo"),
            hit(3, 1, 5, "drifted"),
        ];
        let (touched, applied, stale) = Editor::replace_in_file(&file, &hits, "bar");
        assert_eq!((touched, applied, stale), (1, 2, 1));
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "alpha bar\nbeta bar\ngamma\n"
        );
        assert!(
            !dir.join("a.txt.strop-tmp").exists(),
            "temp file renamed away"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn excluded_rows_stay_out_of_the_apply_set() {
        let mut p = Picker::new(
            Kind::Replace,
            vec![
                Item {
                    text: "a".into(),
                    payload: Payload::Grep {
                        path: PathBuf::from("a"),
                        line: 1,
                        col: 1,
                        match_len: 1,
                        line_text: "x".into(),
                    },
                },
                Item {
                    text: "b".into(),
                    payload: Payload::Grep {
                        path: PathBuf::from("b"),
                        line: 1,
                        col: 1,
                        match_len: 1,
                        line_text: "y".into(),
                    },
                },
            ],
            false,
        );
        p.toggle_excluded(); // excludes row 0
        assert_eq!(p.accepted().count(), 1);
        assert_eq!(p.accepted().next().unwrap().text, "b");
        p.toggle_excluded(); // toggles back
        assert_eq!(p.accepted().count(), 2);
    }

    #[test]
    fn space_r_replace_field_flow() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" Rfoo");
        assert_eq!(e.picker.as_ref().unwrap().picker.kind, Kind::Replace);

        e.feed(crate::editor::Key::Tab);
        assert_eq!(
            e.picker.as_ref().unwrap().picker.field,
            strop_picker::Field::Replace
        );
        e.feed_text("bar");
        assert_eq!(e.picker.as_ref().unwrap().picker.replace_input.text, "bar");
        assert_eq!(e.picker.as_ref().unwrap().picker.input.text, "foo");
    }

    #[test]
    fn respawn_never_renders_stale_rows() {
        // regression (0.3.3 user crash): Space R, type a query, type
        // more — the respawn cleared items but not rows, and the
        // replace renderer indexed items[stale_row] → panic
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha one\nalpha two\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "alpha three\n").unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.cwd = dir.path().to_path_buf();
        e.open_picker(Kind::Replace);

        e.feed_text("alpha");
        for _ in 0..300 {
            e.drain_picker();
            if !e.picker.as_ref().unwrap().picker.items.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            !e.picker.as_ref().unwrap().picker.items.is_empty(),
            "rg delivered matches"
        );
        e.feed_text("b"); // respawn: items + rows both clear
        let frame = crate::headless::frame_string(&mut e, 80, 20);
        assert!(frame.contains("replace"), "{frame}");
    }

    #[test]
    fn replace_filters_narrow_the_apply_set() {
        // user ask: extension limiting + file exclusion in Space R —
        // -t/--glob ride rg's passthrough and the apply set follows
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "foo one\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "foo two\n").unwrap();
        std::fs::write(dir.path().join("c.py"), "foo three\n").unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.cwd = dir.path().to_path_buf();
        e.open_picker(Kind::Replace);
        e.feed_text("foo --glob !*.py");
        for _ in 0..300 {
            e.drain_picker();
            let p = &e.picker.as_ref().unwrap().picker;
            if !p.streaming && !p.items.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let p = &e.picker.as_ref().unwrap().picker;
        assert_eq!(p.items.len(), 2, "py excluded via --glob");
        assert!(p.items.iter().all(|i| !format!("{i:?}").contains("c.py")));
    }

    #[test]
    fn rg_error_is_sticky_in_the_card() {
        // a bad filter must read as an error in the card, not a silent
        // empty list or a modeline flash (cleared on the next key)
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "foo\n").unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.cwd = dir.path().to_path_buf();
        e.open_picker(Kind::Replace);
        e.feed_text("foo --glob/**/bad[");
        for _ in 0..300 {
            e.drain_picker();
            if e.picker.as_ref().unwrap().picker.error.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let err = e.picker.as_ref().unwrap().picker.error.clone();
        assert!(err.is_some(), "rg error captured");
        // navigation, not a query edit: the error survives (a query
        // edit clears it — the new search might be valid)
        e.feed(crate::editor::Key::Esc); // field normal mode
        e.feed(crate::editor::Key::Char('j'));
        let frame = crate::headless::frame_string(&mut e, 80, 20);
        assert!(
            frame.contains("unclosed character class"),
            "error in the card: {frame}"
        );
    }

    #[test]
    fn picker_field_is_modal() {
        // rootle's input boxes: Esc enters normal mode on the field,
        // keys edit the query, i returns to insert, Esc closes
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.open_picker(Kind::Files);
        e.feed_text("main");
        e.feed(crate::editor::Key::Esc);
        assert!(e.picker_open(), "esc once: picker stays open");
        e.feed_text("0x"); // to 0, delete 'm'
        assert_eq!(e.picker.as_ref().unwrap().picker.input.text, "ain");
        e.feed(crate::editor::Key::Esc);
        assert!(!e.picker_open(), "esc twice closes");
    }

    #[test]
    fn picker_normal_mode_jk_walk_results() {
        // Esc into the field's normal mode; j/k move the selection,
        // not the text caret (user report: only Tab/arrows navigated)
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        for f in ["a.txt", "b.txt", "c.txt", "d.txt"] {
            std::fs::write(dir.path().join(f), "x\n").unwrap();
            e.open_buffer(dir.path().join(f).to_str().unwrap()).unwrap();
        }
        e.open_picker(Kind::Buffers);
        let sel = |e: &Editor| e.picker.as_ref().unwrap().picker.selected;
        assert_eq!(sel(&e), 0);
        e.feed(crate::editor::Key::Esc); // normal mode on the field
        e.feed_text("jj");
        assert_eq!(sel(&e), 2, "j moved the selection down twice");
        e.feed_text("k");
        assert_eq!(sel(&e), 1);
        e.feed_text("i"); // back to insert
        e.feed_text("j"); // types into the query instead
        assert_eq!(
            e.picker.as_ref().unwrap().picker.input.text,
            "j",
            "insert mode: j filters"
        );
    }

    #[test]
    fn picker_arrows_navigate_and_move_caret() {
        // user report: physical arrows did nothing in pickers — the
        // translation layer dropped KeyCode::Up/Down entirely
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        for f in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.path().join(f), "x\n").unwrap();
            e.open_buffer(dir.path().join(f).to_str().unwrap()).unwrap();
        }
        e.open_picker(Kind::Buffers);
        e.feed_text("a");
        e.feed(crate::editor::Key::Down);
        assert_eq!(
            e.picker.as_ref().unwrap().picker.selected,
            1,
            "Down walks results"
        );
        e.feed(crate::editor::Key::Up);
        assert_eq!(e.picker.as_ref().unwrap().picker.selected, 0);
        e.feed(crate::editor::Key::Left);
        assert_eq!(
            e.picker.as_ref().unwrap().picker.input.cursor,
            0,
            "Left moves the caret"
        );
        e.feed(crate::editor::Key::Right);
        assert_eq!(e.picker.as_ref().unwrap().picker.input.cursor, 1);
    }
}
