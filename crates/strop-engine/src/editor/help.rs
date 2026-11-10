//! `:help` (also `Space ?`): the keybinding table as a real readonly
//! buffer (0001 §4) — `/` searches it, motions walk it, `q` closes it.
//! Laid out in the keymap's curated section order (0088 U10): a header,
//! then per section a `─ Title ─` rule and one row per binding with the
//! keys in a padded column the renderer paints as a keycap. Replaces
//! the floating keybinds popup: a buffer you can search beats a card you
//! can only scroll.

use strop_core::Buffer;

use super::Editor;

/// The key column is padded one cell each side of the widest key; two
/// spaces separate it from the description (`render/help.rs` splits
/// rows on exactly this shape).
fn push_row(text: &mut String, keys: &str, width: usize, desc: &str, planned: bool) {
    let soon = if planned { "  (soon)" } else { "" };
    text.push_str(&format!("   {keys:<width$}   {desc}{soon}\n"));
}

fn push_rule(text: &mut String, title: &str) {
    text.push_str(&format!("\n─ {title} ─\n"));
}

/// Hand-written rows for surfaces the binding table does not model
/// (remote browsing, the query language, …): `(title, rows)`.
const EXTRA: &[(&str, &[(&str, &str)])] = &[
    (
        "Remote",
        &[
            ("strop ssh://host/path", "How to open: a remote file or directory from the shell"),
            (":browse ssh://host/path", "How to open: a remote directory from inside the editor"),
            ("space o / :remote", "Choose SSH or a running container; Add a host… enters an address"),
            ("- / backspace", "Visit parent directory and retain the selected child"),
            (":remote root / home", "Browse remote root or negotiated home directory"),
            (":remote worker [URI]", "Explicit worker consent for Git/LSP/search; no edit permit"),
            (":remote edit", "Admit worker, verify full snapshot and enable editing"),
            (":w / :wq", "Save an authorized remote file; no conflict bypass via !"),
            (":remote verify", "Reconcile an unconfirmed save without blind overwrite"),
            (":tail [BYTES] [URI]", "Read a bounded tail (URI defaults to current file)"),
            (":range START BYTES URI", "Read a byte window; line numbers are window-relative"),
            (":follow [URI]", "Follow a bounded tail; Escape stops following"),
            (":unfollow", "Keep the snapshot and stop polling"),
            (":browse [URI]", "Open a directory; Enter selects an entry or ../"),
            (":filter [TEXT]", "Filter a directory; no text restores all entries"),
            (":remote connect URI", "Hold a shared SSH connection explicitly"),
            (":remote disconnect URI", "Close the endpoint's owned connection"),
            (":remote clear", "Close all owned connections"),
            (":remote list", "List authenticated connections in a buffer"),
            (":trust", "Authorize the pending endpoint/project command"),
            ("tab", "Complete an SSH URI without starting authentication"),
            (":e!", "Refresh the remote snapshot and revoke write authority"),
            ("directory columns", "kind, POSIX permissions, server bytes, name; ? = unknown"),
            (":explain", "Why: workspaces, LSP readiness/failures, effective config"),
        ],
    ),
    (
        "Query language",
        &[
            ("surfaces", "space f, space /, space R, symbols (space S), the space j jumplist"),
            ("one language", "Identical local-workspace qualifiers on every surface above"),
            ("language:rust", "Include a language family (alias: language:rs)"),
            ("scope", "Regular local files; directory links and special files are not searched"),
            ("SSH/container listings", "Hidden entries included; no ignore-file filtering (local search defaults do not apply)"),
            ("path:src/", "Literal substring in the workspace-relative path"),
            ("glob:\"**/*.rs\"", "Explicit path glob; quote values with spaces"),
            ("fpath:modules/jweb/", "Fuzzy scope over full workspace-relative path"),
            ("-language: -glob: -path:", "Exclude; exclusions win"),
            ("hidden:include|exclude", "Show/hide dotfiles (default: shown)"),
            ("ignored:include|exclude", "Include/respect ignored files (default: respected)"),
            ("case:sensitive", "Exact case; default is insensitive on every query surface"),
            ("case:ignore|insensitive", "Explicit insensitive matching; case:smart opts into smart case"),
            ("text:\"...\"", "Explicit literal content expression (default for bare words)"),
            ("regex:\"...\"", "Explicit Rust-regex expression"),
            ("ctrl-space", "Manual qualifier/language/value suggestions"),
            ("filter-only Search", "Needs a search expression; file find lists matches"),
            ("quoted text", "Stays literal — never a filter (\"language:rust\" searches that text)"),
            ("old -t / -g flags", "Now ordinary literal text; use language: and glob:"),
            ("file expressions", "Bare words are fuzzy; quoted/text: are literal; regex: is explicit"),
            ("quotes", "Single/double quotes; escape the active quote or backslash"),
            ("With", "Always literal replacement text; $1 is not a capture expansion"),
            (":[range]s/a/b/[g]", "Buffer-search pattern; live before/after preview before Enter"),
            (":s replacement", "Literal text; escaped delimiter and \\\\ are unescaped"),
            ("buffer / ?", "Case-insensitive by default; case:sensitive forces exact case"),
            ("buffer case:ignore", "Explicit insensitive search (alias: case:insensitive)"),
            ("input ctrl-r {reg}", "Insert a register; field Normal p/P pastes; no Ctrl-C quit"),
            ("visual ctrl-c", "Toggle line comments on the selected lines"),
        ],
    ),
    (
        "Search workspace",
        &[
            ("space /", "Open/resume the retained local Search investigation"),
            ("space R", "Resume the same Search with replacement visible"),
            ("ctrl-r", "Toggle With without restarting Search or losing its draft"),
            ("Find enter", "Open selected source, even when With is visible"),
            ("With enter", "Prepare Review; never apply or save directly"),
            ("tab / s-tab", "Switch visible fields; otherwise move results"),
            ("ctrl-x / ctrl-d", "Exclude/include match or file in the shared workset"),
            ("esc", "Dismiss suggestions, enter field Normal, then hide Search"),
            ("re-entry", "Refresh exact witnesses in the captured scope; report lost decisions"),
            ("scope", "Local project backend; unsupported namespaces refuse, never fall back"),
        ],
    ),
    (
        "Collections",
        &[
            ("ctrl-o", "Collect complete included results; all-excluded is empty"),
            ("+ / -", "Expand/contract the current excerpt's context"),
            ("]e / [e", "Next/previous excerpt; ]f / [f move file cards"),
            ("g<space>", "Open the real source; Ctrl-O restores the collection view"),
            ("read-only", "Search/yank/navigate freely; edits happen in the real source"),
        ],
    ),
    (
        "Indentation",
        &[
            (":tab-size [N|auto]", "Source width, 1-16; no argument opens the selector"),
            (":indent-style [spaces|tabs|auto]", "Source style or configured/detected policy"),
            ("detection", "Bounded open/reload sample; ambiguity keeps the configured fallback"),
        ],
    ),
    (
        "Review",
        &[
            ("enter in With", "Prepare the exact review, without changing or saving sources"),
            ("space a", "Apply/Save/Cancel/Inspect actions valid for the focused review"),
            (":apply-change", "Apply to source buffers; results remain dirty until saved"),
            (":cancel-change", "Cancel and restore the query/draft/exclusions"),
            (":save-change", "Save files from the most recent applied operation"),
            (":undo-change", "Undo verified targets; conflicted receipt members remain recoverable"),
        ],
    ),
    (
        "Documentation",
        &[
            ("enter on hover", "Open full searchable documentation; Ctrl-O returns"),
            (":containers", "Attach to a running container (read-only browse)"),
            (":container-worker", "Enable verified services for the attached container"),
            (":container-edit", "Verify the snapshot and enable conditional file saves"),
        ],
    ),
];

impl Editor {
    /// Open the generated help buffer (`:help` / `Space ?`).
    pub(crate) fn open_help(&mut self) {
        // already open → switch, don't stack copies
        let existing = self
            .docs
            .iter()
            .find(|(_, d)| d.buf.name.as_deref() == Some("help"))
            .map(|(id, _)| id);
        if let Some(i) = existing {
            self.push_jump();
            self.switch_to(i);
            self.set_head(0);
            self.view_mut().view_top = 0;
            return;
        }
        let mut text = format!(
            "strop {} · help · / search · q close\n",
            env!("CARGO_PKG_VERSION")
        );
        for section in crate::keymap::SECTIONS {
            push_rule(&mut text, section.title);
            let rows: Vec<_> = crate::keymap::BINDINGS
                .iter()
                .filter(|b| b.sections.contains(&section.id))
                .collect();
            let width = rows
                .iter()
                .map(|b| b.keys.chars().count())
                .max()
                .unwrap_or(0);
            for b in rows {
                push_row(&mut text, b.keys, width, b.desc, !b.live);
            }
        }
        for (title, rows) in EXTRA {
            push_rule(&mut text, title);
            let width = rows
                .iter()
                .map(|(keys, _)| keys.chars().count())
                .max()
                .unwrap_or(0);
            for (keys, desc) in *rows {
                push_row(&mut text, keys, width, desc, false);
            }
        }
        // Which server each language family (the `language:` values)
        // starts, and how to get it; families without one say so.
        push_rule(&mut text, "LSP servers");
        let languages: Vec<_> = strop_core::languages::language_names().collect();
        let width = languages.iter().map(|name| name.len()).max().unwrap_or(0);
        let mut serverless = Vec::new();
        for language in languages {
            match strop_lsp::registry::embedded_server(language) {
                Some((server, hint)) => push_row(
                    &mut text,
                    language,
                    width,
                    &format!("{server} · {hint}"),
                    false,
                ),
                None => serverless.push(language),
            }
        }
        if !serverless.is_empty() {
            push_row(
                &mut text,
                "",
                width,
                &format!(
                    "no built-in server: {} (config: [language.NAME])",
                    serverless.join(", ")
                ),
                false,
            );
        }
        let mut buf = Buffer::from_text(&text);
        buf.name = Some("help".into());
        // a temporary surface (0051 §7 R07): ctrl-o AND `:q` restore
        // the exact view the user came from
        let _ = self.open_temporary_output(buf);
    }

    pub(crate) fn open_help_topic(&mut self, topic: &str) {
        self.open_help();
        // Identity exhaustion leaves us on the origin buffer: never
        // search/scroll the wrong document (0056 AR13).
        if self.cur().buf.name.as_deref() != Some("help") {
            return;
        }
        let topic = topic.trim();
        if topic.is_empty() {
            return;
        }
        // `:help git` reaches a section by id or title, case-insensitively
        let title = crate::keymap::Section::find(topic)
            .map(|section| section.title.to_owned())
            .unwrap_or_else(|| topic.to_owned());
        let heading = format!("─ {title} ─");
        let line = self.buf().text().lines().position(|line| {
            line.chars()
                .take_while(|&c| c != '\n' && c != '\r')
                .collect::<String>()
                .eq_ignore_ascii_case(&heading)
        });
        if let Some(line) = line {
            self.set_head(self.buf().line_start(line));
            self.view_mut().view_top = line;
        } else {
            self.message = format!("no help topic {topic:?}; :help shows the index");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_buffer_lists_every_section_and_searches() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(":help\r");
        assert_eq!(e.buf().name.as_deref(), Some("help"));
        assert!(e.buf().readonly);
        let text = e.buf().text().to_string();
        let mut last = 0;
        for section in crate::keymap::SECTIONS {
            let heading = format!("─ {} ─", section.title);
            let at = text
                .find(&heading)
                .unwrap_or_else(|| panic!("missing {}", section.title));
            assert!(at > last, "{} out of curated order", section.title);
            last = at;
        }
        let missing: Vec<_> = [
            ":theme",
            ":jumps",
            ":noh",
            ":format",
            ":rename",
            ":recover",
            ":qa",
            ":qall",
            ":collection",
            ":symbols",
        ]
        .into_iter()
        .filter(|command| !text.contains(*command))
        .collect();
        assert!(
            missing.is_empty(),
            "dispatchable Ex commands missing from generated help: {missing:?}"
        );
        // 0065 S2: the terminal section names every dispatchable escape —
        // the exit from terminal input is discoverable from `?` alone.
        for escape in ["ctrl-\\ ctrl-n", "ctrl-4 ctrl-n", ":terminal-refresh"] {
            assert!(text.contains(escape), "help lost the {escape} escape");
        }
        assert!(
            text.contains(":explain") && text.contains("LSP readiness/failures"),
            "help makes retained LSP failures discoverable"
        );
        // the language list says what it is: the server each family starts
        let servers = text.find("─ LSP servers ─").expect("LSP servers section");
        assert!(text[servers..].contains("rust          rust-analyzer · rustup"));
        assert!(!text.contains("─ Languages ─"));
        // Remote opens with how to get there
        let remote = text.find("─ Remote ─").expect("Remote section");
        let first = text[remote..].lines().nth(1).unwrap();
        assert!(first.contains("How to open"), "{first:?}");
        // it's a real buffer: / searches it
        e.feed_text("/undo-tree\r");
        assert!(e.head() > 0, "search moved into the help text");
        // q closes back to the file
        e.feed_text("q");
        assert_eq!(e.buf().name.as_deref(), None);
    }

    #[test]
    fn space_question_opens_help_and_topics_reach_sections() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" ?");
        assert_eq!(e.buf().name.as_deref(), Some("help"));
        e.feed_text("q");
        e.feed_text(":help git\r");
        let line = e.buf().line_of(e.head());
        let text = e.buf().text().line(line).to_string();
        assert!(text.starts_with("─ Git ─"), "{text:?}");
    }

    #[test]
    fn closing_help_on_a_fresh_launch_returns_to_the_scratch() {
        let mut e = Editor::new(Buffer::from_text(""));
        e.feed_text(" ?");
        assert_eq!(e.buf().name.as_deref(), Some("help"));
        e.feed_text("q");
        assert!(!e.should_quit, "q on help must not quit the editor");
        assert_eq!(e.buf().name, None);
        assert_eq!(e.buf().path, None);
    }
}
