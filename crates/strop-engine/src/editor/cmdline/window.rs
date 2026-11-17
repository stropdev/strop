//! The command-line window (plan 0110 §3.2, Vim's `q:`): the history of
//! `:` (or `/ ?`) lines as a real buffer in a split below, the line being
//! typed last. Every Normal key, operator and search works in it; `Enter`
//! runs the cursor's line, `Ctrl-C` or `:q` closes it.

use strop_core::frontend_input::Key;
use strop_core::id::DocumentId;

use super::keys::Action;
use crate::editor::{Document, Editor, Mode};

/// The open window: its buffer and which history it shows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Window {
    pub doc: DocumentId,
    pub sigil: char,
}

/// `q:`, `q/`, `q?`: open the window on `sigil`'s history with
/// `typed` as its last line (`Ctrl-F` carries the open line in).
pub(crate) fn open(editor: &mut Editor, sigil: char, typed: &str) {
    if editor.cmdline.window.is_some() {
        editor.message = "the command-line window is already open".into();
        return;
    }
    let mut text = String::new();
    if let Some(history) = editor.cmdline.history.of(sigil) {
        for line in history.iter() {
            text.push_str(line);
            text.push('\n');
        }
    }
    text.push_str(typed);
    text.push('\n');
    let mut buffer = strop_core::Buffer::from_text(&text);
    buffer.name = Some("command line".into());
    let Ok(doc) = editor.docs.try_insert(Document::scratch(buffer)) else {
        editor.message = "document identity space exhausted".into();
        return;
    };
    editor.mode = Mode::Normal;
    editor.split_document(false, doc);
    let last = editor.buf().last_content_line();
    let end = editor.buf().line_end(last);
    editor.set_head(end.saturating_sub(usize::from(!typed.is_empty())));
    editor.clamp_cursor();
    editor.cmdline.window = Some(Window { doc, sigil });
}

/// `Ctrl-F` on the open line: edit it in the window.
pub(crate) fn from_line(editor: &mut Editor) {
    let Some(line) = editor.cmdline.line() else {
        return;
    };
    let (sigil, body) = (line.sigil(), line.body().to_owned());
    if sigil == '|' {
        return; // a pipe line has no history to edit beside
    }
    editor.cancel_cmdline();
    open(editor, sigil, &body);
}

/// The window's own keys: `Enter` runs the cursor's line, `Ctrl-C`
/// closes. True when the key was the window's.
pub(crate) fn key(editor: &mut Editor, key: Key) -> bool {
    let Some(window) = editor.cmdline.window else {
        return false;
    };
    if editor.current() != window.doc || !matches!(editor.mode, Mode::Normal | Mode::Insert) {
        return false;
    }
    if matches!(key, Key::Enter | Key::CtrlC) && editor.mode == Mode::Insert {
        // the insert ends as Esc would end it (one undo step)
        editor.feed_document(Key::Esc);
    }
    match key {
        Key::Enter => {
            let line = editor.buf().line_text(editor.buf().line_of(editor.head()));
            close(editor);
            // through the line itself, so history and every rule of
            // running a line apply
            editor.begin_text_line(window.sigil, Default::default());
            editor.cmdline_paste(&line);
            editor.cmdline_action(Action::Run);
            true
        }
        Key::CtrlC => {
            close(editor);
            true
        }
        _ => false,
    }
}

/// Close the window: its pane and its buffer go, the focus returns.
pub(crate) fn close(editor: &mut Editor) {
    let Some(window) = editor.cmdline.window.take() else {
        return;
    };
    editor.mode = Mode::Normal;
    let Some(pane) = editor.panes.iter().position(|pane| pane.doc == window.doc) else {
        return;
    };
    editor.focus_pane(pane);
    editor.close_buffer(true);
    if editor.panes.len() > 1 {
        editor.close_pane_or_buffer(false);
    }
}
