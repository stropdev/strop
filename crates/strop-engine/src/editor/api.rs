//! The admitted frontend boundary (0056 AR02): every cross-crate read
//! of editor state is a readonly borrowed query here, every frontend
//! write an admitted method — no public field reaches documents,
//! service maps, leases, config or recovery state. `fixture_*` hooks
//! exist only under test-support so frontend tests plant state through
//! named seams instead of mutating engine internals.

use super::*;

impl Editor {
    /// Focus identity for frame-preparation stamps (AR01).
    pub fn focus_epoch(&self) -> u64 {
        self.focus_epoch
    }

    /// Readonly user config (0056 AR02): frontends never mutate it
    /// field-by-field; startup/reload replaces it whole.
    pub fn config(&self) -> &crate::config::Config {
        &self.config
    }

    /// Replace the user config (startup load, settings reload).
    pub fn set_config(&mut self, config: crate::config::Config) {
        self.config = config;
    }

    /// The shared state root for trust/session/recovery, when enabled.
    pub fn state_dir(&self) -> Option<&std::path::Path> {
        self.state_dir.as_deref()
    }

    pub fn set_state_dir(&mut self, state_dir: Option<PathBuf>) {
        self.state_dir = state_dir;
    }

    pub fn set_session_policy(&mut self, policy: crate::session::SessionPolicy) {
        self.session_policy = policy;
    }

    /// The interaction mode (0056 AR02 readonly query): paint reads it;
    /// input through [`Editor::feed`]/[`Editor::handle_frontend_input`]
    /// is the only way it changes.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The forensic tape (0056 AR02 readonly borrow): the frontend
    /// samples ticks, seeds and seals recordings through [`Tape`]'s own
    /// API; a recorder error surfaces through
    /// [`Editor::note_tape_divergence`], never a direct message write.
    pub fn tape(&self) -> &strop_trace::replay::Tape {
        &self.tape
    }

    /// The open picker, when one is up (readonly; AR02).
    pub fn picker(&self) -> Option<&PickerGlue> {
        self.picker.as_ref()
    }

    /// The picker preview file cache (readonly; AR02).
    pub fn previews(&self) -> &Previews {
        &self.previews
    }

    /// The working-tree hunk set (readonly; AR02).
    pub fn hunks(&self) -> &git_memory::HunkSet {
        &self.hunks
    }

    /// The staged (HEAD↔index) hunk set (readonly; AR02).
    pub fn staged_hunks(&self) -> &git_memory::HunkSet {
        &self.staged_hunks
    }

    /// Whether untracked files exist beside the hunk sets (readonly; AR02).
    pub fn hunks_untracked(&self) -> bool {
        self.hunks_untracked
    }

    /// The blame card under the cursor, when one is up (readonly; AR02).
    pub fn blame_card(&self) -> Option<&strop_git::memory::BlameCard> {
        self.blame_card.as_ref()
    }

    /// The git working-surface context, when discovered (readonly; AR02).
    pub fn git(&self) -> Option<&strop_git::GitContext> {
        self.git.as_ref()
    }

    /// The LSP hover card text, when one is up (readonly; AR02).
    pub fn hover_card(&self) -> Option<&str> {
        self.hover_card.as_deref()
    }

    /// The statusline message (readonly; AR02).
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The admitted message write (0056 AR02): startup/load failures and
    /// host-effect errors surface here; engine handlers keep setting the
    /// field directly.
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    /// Tape divergence at the draw boundary (0056 AR02): a failed
    /// `Frame` record or observation check is the one message paint may
    /// raise, and only through here.
    pub fn note_tape_divergence(&mut self, error: impl std::fmt::Display) {
        self.message = error.to_string();
    }

    /// Whether the editor is quitting (readonly; AR02): quit itself is
    /// an admitted action (`close_buffer`, `ctrl_c_quit`,
    /// [`Editor::request_quit`]).
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// The frontend's quit request (0056 AR02): the input/event stream
    /// ended underneath the editor (terminal disconnect, scripted EOF).
    pub fn request_quit(&mut self) {
        self.should_quit = true;
    }

    /// Take a pending full-repaint request (ctrl-l desync recovery):
    /// the draw loop consumes it exactly once.
    pub fn take_repaint_request(&mut self) -> bool {
        std::mem::take(&mut self.needs_repaint)
    }

    /// Drain the queued host-effect payloads (OSC52 clipboard writes):
    /// the terminal loop owns writing them to the tty.
    pub fn take_terminal_output(&mut self) -> Vec<String> {
        std::mem::take(&mut self.terminal_output)
    }

    /// The process-wide project directory (readonly; AR02).
    pub fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }

    /// Name-resolution state (readonly; AR02): the driver polls
    /// `pending()` on it while waiting out goto-definition turns.
    pub fn resolution(&self) -> &ResolutionState {
        &self.resolution
    }

    /// The free-text pending line (readonly; AR02): `:`/`/` input in
    /// flight, drawn by the command card and statusline.
    pub fn pending(&self) -> &pending::PendingInput {
        &self.pending
    }

    /// The input walker (readonly; AR02): typed count/register/operator
    /// prefix state for the which-key and statusline hints.
    pub fn walker(&self) -> &input::Walker {
        &self.walker
    }

    /// MRU document order, most recent first (readonly; AR02).
    pub fn mru(&self) -> &[strop_core::id::DocumentId] {
        &self.mru
    }

    /// Install the injected frame renderer (0046): the binary's
    /// TestBackend draw used by headless mode and trace replay.
    pub fn set_frame_draw(&mut self, frame_draw: Option<FrameDraw>) {
        self.frame_draw = frame_draw;
    }

    /// Fixture hook (tests only): plant a git context without discovery.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture_set_git(&mut self, git: Option<strop_git::GitContext>) {
        self.git = git;
    }

    /// Fixture hook (tests only): plant the untracked-files flag.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture_set_hunks_untracked(&mut self, untracked: bool) {
        self.hunks_untracked = untracked;
    }

    /// Fixture hook (tests only): plant an LSP hover card.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture_set_hover_card(&mut self, card: Option<String>) {
        self.hover_card = card;
    }

    /// The armed search, when `n`/`N` have something to replay
    /// (readonly; AR02).
    pub fn last_search(&self) -> Option<&LastSearch> {
        self.last_search.as_ref()
    }

    /// The cursor fade-in's start tick, while fading (readonly; AR02) —
    /// progress itself is [`Editor::cursor_fade_progress`].
    pub fn cursor_fade(&self) -> Option<strop_trace::replay::Tick> {
        self.cursor_fade
    }

    /// Fixture hook (tests only): plant diagnostics without an LSP
    /// server round-trip.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture_insert_diagnostics(
        &mut self,
        document: strop_core::id::DocumentId,
        diagnostics: DocumentDiagnostics,
    ) {
        self.diags.insert(document, diagnostics);
    }

    /// Fixture hook (tests only): plant the working-tree hunk set
    /// without a git worker round-trip.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture_set_hunks(&mut self, hunks: git_memory::HunkSet) {
        self.hunks = hunks;
    }

    /// Fixture hook (tests only): plant the cursor fade-in clock.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture_set_cursor_fade(&mut self, fade: Option<strop_trace::replay::Tick>) {
        self.cursor_fade = fade;
    }
}
