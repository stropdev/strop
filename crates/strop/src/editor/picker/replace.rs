//! Global replace (0007 §4): apply accepted hits bottom-up, one undo
//! revision per touched buffer, drifted lines skipped and counted.

use std::collections::HashMap;
use std::path::PathBuf;

use strop_picker::Payload;

use super::super::Editor;

impl Editor {
    /// Replace mode Enter: apply every accepted hit. One undo revision
    /// per touched buffer (0007 §4); lines that drifted since the search
    /// are skipped and counted, never silently rewritten.
    pub(crate) fn apply_replace(&mut self) {
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
            // 0020 §8: unopened files become real buffers — one
            // transaction model, one persistence path, real undo
            let (f, a, s) = match self.buffer_index_of(&full) {
                Some(bi) => self.replace_in_buffer(bi, &hits, &replacement),
                None => {
                    match self.open_buffer(&full) {
                        Ok(()) => {
                            let bi = self.current();
                            let (f, a, s) = self.replace_in_buffer(bi, &hits, &replacement);
                            if a > 0 {
                                // persist through the buffer's own atomic
                                // writer (mode preserved, baseline set)
                                let r = self.buf_mut().save(true);
                                if let Err(err) = r {
                                    self.message = format!("write {}: {err}", full.display());
                                }
                            }
                            (f, a, s)
                        }
                        Err(_) => (0, 0, hits.len()),
                    }
                }
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
            format!("replaced {applied} in {files} files — u per buffer undoes{stale_msg}");
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
    #[cfg(test)]
    pub(crate) fn replace_in_buffer_pub(
        &mut self,
        bi: strop_core::id::DocumentId,
        hits: &[(usize, usize, usize, String)],
        replacement: &str,
    ) -> (usize, usize, usize) {
        self.replace_in_buffer(bi, hits, replacement)
    }

    fn replace_in_buffer(
        &mut self,
        bi: strop_core::id::DocumentId,
        hits: &[(usize, usize, usize, String)],
        replacement: &str,
    ) -> (usize, usize, usize) {
        let mut applied = 0;
        let mut stale = 0;
        if self.docs.get(bi).is_none() {
            return (0, 0, hits.len());
        }
        if self.doc(bi).buf.readonly {
            return (0, 0, hits.len());
        }
        // verify each hit against the CURRENT text; the accepted edits
        // go through the gateway as one validated changeset (0024)
        let mut edits = Vec::new();
        for (line, col, match_len, expected) in hits {
            let (s, e) = strop_picker::replace_span(expected, *col, *match_len);
            let (ls, len) = (
                self.doc(bi).buf.line_start(line - 1),
                self.doc(bi).buf.len_bytes(),
            );
            let abs_s = ls + s;
            let abs_e = (ls + e).min(len);
            // verify the matched *span*, not the whole line: same-line
            // hits stay verifiable as earlier (rightward) ones apply
            let aligned =
                self.doc(bi).buf.is_boundary(abs_s) && self.doc(bi).buf.is_boundary(abs_e);
            let matches = aligned
                && abs_s <= abs_e
                && self
                    .doc(bi)
                    .buf
                    .rope
                    .byte_slice(abs_s..abs_e)
                    .to_string()
                    .as_bytes()
                    == &expected.as_bytes()[s..e];
            if *line == 0 || *line > self.doc(bi).buf.len_lines() || !matches {
                stale += 1;
                continue;
            }
            // delete+insert as one edit pair (delete first)
            edits.push(strop_core::history::Edit {
                at: abs_s,
                text: self.doc(bi).buf.rope.byte_slice(abs_s..abs_e).to_string(),
                kind: strop_core::history::EditKind::Delete,
            });
            edits.push(strop_core::history::Edit {
                at: abs_s,
                text: replacement.to_string(),
                kind: strop_core::history::EditKind::Insert,
            });
            applied += 1;
        }
        if edits.is_empty() {
            return (0, 0, stale);
        }
        let base = self.doc(bi).buf.epoch;
        match self.apply(
            bi,
            base,
            crate::editor::transact::ChangeSet {
                edits,
                undo_open: false,
            },
        ) {
            Ok(_) => (1, applied, stale),
            Err(_) => (0, 0, applied + stale), // raced — report all stale
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;
    use strop_picker::{Item, Kind, Picker};

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
        // 0020 §8: an unopened file becomes a real buffer — atomic
        // write through the buffer's own path, undo included
        let dir = std::env::temp_dir().join(format!("strop-replace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "alpha foo\nbeta foo\ngamma\n").unwrap();
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.cwd = dir.clone();
        let hits = vec![
            hit(2, 6, 3, "beta foo"),
            hit(1, 7, 3, "alpha foo"),
            hit(3, 1, 5, "drifted"),
        ];
        e.open_buffer(&file).unwrap();
        let bi = e.current();
        let (touched, applied, stale) = e.replace_in_buffer(bi, &hits, "bar");
        assert_eq!((touched, applied, stale), (1, 2, 1));
        e.buf_mut().save(true).unwrap();
        // undo exists for the file-backed buffer too
        e.undo();
        assert_eq!(e.buf().rope.to_string(), "alpha foo\nbeta foo\ngamma\n");
        e.redo();
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
}
