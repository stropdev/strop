//! Presentation state: what the next frame paints beyond the documents
//! themselves. None of it is part of the recorded observation except the
//! palette, which the replay seed carries explicitly.
use strop_core::id::DocumentId;
use strop_core::Range;
use strop_trace::replay::Tick;

#[derive(Default)]
pub(crate) struct Presentation {
    /// The session palette (0088 §3): config supplies the startup choice,
    /// `--theme` overrides it, `:theme` switches it. Nothing is process-global.
    pub(crate) theme: strop_core::theme::ThemeId,
    /// The yank/put flash range and the tick it started.
    pub(crate) flash: Option<(Range, Tick)>,
    /// Cursor fade-in (0064 §2): the Normal-mode block cursor fades back in
    /// from this tick after a focus return, pane/buffer switch or a jump
    /// beyond half a screen.
    pub(crate) cursor_fade: Option<Tick>,
    /// Last cursor-fade track mark (pane, document, head line) for jump
    /// detection across recorded actions.
    pub(crate) fade_track: Option<(usize, DocumentId, usize)>,
}
