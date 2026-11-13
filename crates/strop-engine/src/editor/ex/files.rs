//! Files and windows: `:w :wq :x :up :wa :wqa :e :view :set :enew :new
//! :close :only`.

use strop_core::id::DocumentId;

use super::ExCall;
use crate::editor::io::OpenIntent;
use crate::editor::{Document, DocumentSource, Editor};

/// `:w[!] [file]`, `:wq[!] [file]`, `:x[!]` / `:exit`, `:up[!]`. The
/// last two write only a modified buffer.
pub(crate) fn ex_write(editor: &mut Editor, call: &ExCall<'_>) {
    let close = matches!(call.name, "wq" | "xit" | "exit");
    let only_modified = matches!(call.name, "xit" | "exit" | "update");
    if editor.collections.contains_key(&editor.current()) {
        // a collection's file is its sources (0094 §3.1)
        editor.collection_save(close);
        return;
    }
    if only_modified && !editor.buf().is_dirty() && call.arg.is_empty() {
        if close {
            editor.close_pane_or_buffer(false);
        }
        return;
    }
    // vim: readonly buffers refuse plain :w (surfaces, :view);
    // :w! forces through the mutation boundary's rule.
    if !close && editor.buf().readonly && !call.bang && editor.remote_file().is_none() {
        let name = editor.buf().name.as_deref().unwrap_or("readonly buffer");
        editor.message = format!("{name}: readonly — :w! to force");
        return;
    }
    editor.request_save(
        (!call.arg.is_empty()).then(|| call.arg.into()),
        call.bang,
        close,
    );
}

/// `:wa[!]`, and with `quit` `:wqa` / `:xa`: write every modified
/// buffer that has a file. Modified buffers without a name are
/// reported, never skipped in silence; `:wqa` then stays open.
pub(crate) fn write_all(editor: &mut Editor, force: bool, quit: bool) {
    let mut named: Vec<DocumentId> = Vec::new();
    let mut unnamed = 0;
    for (id, document) in editor.docs.iter() {
        if !document.buf.is_dirty() || editor.collections.contains_key(&id) {
            continue;
        }
        match document.source {
            DocumentSource::File | DocumentSource::Remote(_) if document.buf.path.is_some() => {
                named.push(id)
            }
            DocumentSource::File | DocumentSource::Scratch => unnamed += 1,
            _ => {}
        }
    }
    let mut written = 0;
    for id in named {
        if editor.request_save_document(id, None, force, false) {
            written += 1;
        } else {
            // The refusal's own message says which write and why.
            return;
        }
    }
    let plural = |count: usize| if count == 1 { "" } else { "s" };
    editor.message = format!("{written} buffer{} written", plural(written));
    if unnamed > 0 {
        editor.message.push_str(&format!(
            "; {unnamed} modified buffer{} without a file name not written (:w NAME)",
            plural(unnamed)
        ));
        return;
    }
    if quit {
        editor.pending.ex.quit_after_writes = true;
        quit_after_writes(editor);
    }
}

/// `:wqa` quits once its writes have landed. A failed write leaves a
/// modified buffer, and `quit_all` then refuses with its own message.
pub(crate) fn quit_after_writes(editor: &mut Editor) {
    if !editor.pending.ex.quit_after_writes
        || editor.local_write_pending()
        || editor.remote.writes.pending()
    {
        return;
    }
    editor.pending.ex.quit_after_writes = false;
    editor.quit_all(false);
}

/// `:e[!] [file]`
pub(crate) fn ex_edit(editor: &mut Editor, call: &ExCall<'_>) {
    if editor.buf().is_dirty() && !call.bang {
        editor.message = "unsaved changes — :e! to force".into();
    } else if call.arg.is_empty() && editor.buf().is_dirty() && editor.directory().is_some() {
        // A folder's pending edits are its unsaved changes: `:e!` drops
        // them and lists the folder again.
        if let Err(error) = editor.discard_filename_draft() {
            editor.message = error;
        }
    } else if call.arg.is_empty() {
        if !editor.refresh_current_resource() {
            editor.message = "no file or directory to reload".into();
        }
    } else {
        editor.request_user_open(call.arg, OpenIntent::Switch { readonly: false });
    }
}

/// `:view [file]`: edit readonly — no argument marks this buffer.
pub(crate) fn ex_view(editor: &mut Editor, call: &ExCall<'_>) {
    if call.arg.is_empty() {
        editor
            .buf_mut()
            .set_readonly(strop_core::ReadonlyReason::Command);
        editor.message = "readonly".into();
    } else {
        editor.request_user_open(call.arg, OpenIntent::Switch { readonly: true });
    }
}

/// `:set`: vim's option surface, narrowly — `ro` / `noro`.
pub(crate) fn ex_set(editor: &mut Editor, call: &ExCall<'_>) {
    match call.arg {
        "ro" | "readonly" => {
            editor
                .buf_mut()
                .set_readonly(strop_core::ReadonlyReason::Command);
            editor.message = "readonly".into();
        }
        "noro" | "noreadonly" => {
            if editor.terminal_document(editor.current()).is_some() {
                editor.message = "terminal buffer is read-only — i enters TERMINAL mode".into();
                return;
            }
            if editor
                .directory()
                .is_some_and(|source| source.draft.as_ref().is_none_or(|draft| !draft.editable()))
            {
                editor.message = crate::editor::filesystem::draft::OVER_LIMIT.into();
                return;
            }
            editor.buf_mut().clear_readonly();
            editor.message = "writable".into();
        }
        arg => editor.message = format!("unknown option: {arg}"),
    }
}

/// `:enew` (`split` None), `:new` (horizontal), `:vnew`: a new unnamed
/// buffer, here or in a split. As with Neovim's 'hidden', a modified
/// buffer stays open behind it.
pub(crate) fn ex_new(editor: &mut Editor, split: Option<bool>) {
    let document = Document::scratch(strop_core::Buffer::from_text(""));
    let Ok(id) = editor.docs.try_insert(document) else {
        editor.message = "document identity space exhausted".into();
        return;
    };
    match split {
        Some(vertical) => editor.split_document(vertical, id),
        None => {
            editor.push_jump();
            editor.switch_to(id);
        }
    }
    editor.touch_mru(id);
}

/// `:clo[se]`: close this window; the last one stays.
pub(crate) fn ex_close(editor: &mut Editor) {
    if editor.panes.len() < 2 {
        editor.message = "cannot close the last window".into();
        return;
    }
    editor.close_pane_or_buffer(false);
}

/// `:on[ly]`: this window becomes the only one.
pub(crate) fn ex_only(editor: &mut Editor) {
    if editor.panes.len() < 2 {
        return;
    }
    editor.cancel_pending();
    let pane = editor.panes.swap_remove(editor.active_pane);
    editor.panes = vec![pane];
    editor.active_pane = 0;
    editor.focus_epoch += 1;
    editor.cancel_open(strop_core::worker::CancelReason::OwnerClosed);
    editor.discover_git();
}
