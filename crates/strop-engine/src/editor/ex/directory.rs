//! `:cd [dir]`: the one way the project root changes (plan 0092 §2.3).
//! Everything keyed by `Editor::cwd` is re-anchored here: relative buffer
//! names, the selected workspace root, the session and recovery files,
//! the file watcher and the picker's watch coverage, and git discovery.
//! The LSP "outside the project" test reads `cwd` at each use.

use std::path::PathBuf;

use strop_workspace::ResourceLocation;

use super::ExCall;
use crate::editor::{DocumentSource, Editor};

/// `:cd` (home), `:cd -` (back), `:cd dir` (relative to the root, `~`
/// is home). The directory is resolved on a worker; the root changes
/// when the answer lands.
pub(crate) fn ex_cd(editor: &mut Editor, call: &ExCall<'_>) {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let target = match call.arg {
        "" | "~" => home(),
        "-" => editor.pending.ex.previous_directory.clone(),
        arg => match arg.strip_prefix("~/") {
            Some(rest) => home().map(|home| home.join(rest)),
            None => Some(editor.cwd.join(arg)),
        },
    };
    let Some(target) = target else {
        editor.message = if call.arg == "-" {
            "no previous directory".into()
        } else {
            "no home directory".into()
        };
        return;
    };
    editor.request_directory(target);
}

/// Why the root cannot move right now, if anything is mid-flight
/// that was admitted against the present one.
fn directory_busy(editor: &Editor) -> Option<&'static str> {
    if editor.local_write_pending() || editor.remote.writes.pending() {
        Some("a file is still being written")
    } else if !editor.recovery_can_move() {
        Some("the draft checkpoint is still being written")
    } else if editor.notify.subscribing() {
        Some("the file watcher is still starting")
    } else {
        None
    }
}

/// Make `directory` (absolute, existing) the project root.
pub(crate) fn change_directory(editor: &mut Editor, directory: PathBuf) {
    if directory == editor.cwd {
        editor.message = editor.cwd.display().to_string();
        return;
    }
    if let Some(busy) = directory_busy(editor) {
        editor.message = format!("{busy}; :cd again in a moment");
        return;
    }
    // The only step that can refuse goes first: after it, everything
    // moves.
    if let Err(error) = editor
        .workspaces
        .select_root(ResourceLocation::local(directory.clone()))
    {
        editor.message = error.to_string();
        return;
    }
    let old = editor.cwd.clone();
    // The old project keeps a current session and loses this
    // session's drafts; both paths are fixed while the root is old.
    editor.request_session_save();
    editor.recovery_leave_project();
    // A buffer named relative to the old root keeps naming that file.
    for id in editor.docs.iter().map(|(id, _)| id).collect::<Vec<_>>() {
        let Some(document) = editor.docs.get_mut(id) else {
            continue;
        };
        if !matches!(document.source, DocumentSource::File) {
            continue;
        }
        if let Some(path) = document.buf.path.as_mut().filter(|path| path.is_relative()) {
            *path = old.join(&*path);
        }
    }
    // The watcher covered the old root; hints from it would be read
    // against the new one.
    if let Some(source) = &editor.picker_source {
        source.set_watching(&old, false);
    }
    let watching = editor.notify.push_coverage();
    editor.notify.stop();
    editor.close_picker();
    editor
        .workspaces
        .retire_root(ResourceLocation::local(old.clone()));
    editor.cwd = directory;
    editor.pending.ex.previous_directory = Some(old);
    editor.request_session_save();
    editor.recovery_enter_project();
    if watching {
        editor.start_notifications();
    }
    editor.discover_git();
    editor.needs_repaint = true;
    editor.message = editor.cwd.display().to_string();
}
