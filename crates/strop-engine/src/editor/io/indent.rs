//! Indent resolution for opened documents.

use super::Editor;
use strop_core::id::DocumentId;

impl Editor {
    /// Re-resolve every open document (config lands after the startup
    /// buffer's construction in main).
    pub fn reresolve_indents(&mut self) {
        let ids: Vec<_> = self.docs.iter().map(|(id, _)| id).collect();
        for id in ids {
            self.resolve_indent_for(id);
        }
    }

    /// Indent resolution (0051 R08): manual override → confident
    /// detection → config, decided independently for style and width.
    /// Overrides live on the document, so reloads and config refreshes
    /// preserve them; a detected Tab style never dictates a width —
    /// display width falls through to the configured/manual width.
    pub(crate) fn resolve_indent_for(&mut self, document: DocumentId) {
        use super::super::document::{Detection, IndentSource};
        let detection = if self.config.indent_detect {
            self.docs.get(document).and_then(|doc| doc.detection)
        } else {
            None
        };
        let configured = super::super::document::Indent {
            style: self.config.indent_style,
            width: self.config.tab_size,
            style_source: IndentSource::Configured,
            width_source: IndentSource::Configured,
        };
        let Some(doc) = self.docs.get_mut(document) else {
            return;
        };
        let (style, style_source) = match (doc.indent_override.style, detection) {
            (Some(style), _) => (style, IndentSource::Manual),
            (None, Some(Detection::Tabs { .. })) => {
                (crate::config::IndentStyle::Tabs, IndentSource::Detected)
            }
            (None, Some(Detection::Spaces { .. })) => {
                (crate::config::IndentStyle::Spaces, IndentSource::Detected)
            }
            (None, _) => (configured.style, IndentSource::Configured),
        };
        let (width, width_source) = match (doc.indent_override.width, detection, style_source) {
            (Some(width), _, _) => (width, IndentSource::Manual),
            // A detected width is meaningful only with the detected
            // spaces style it was measured on.
            (None, Some(Detection::Spaces { width, .. }), IndentSource::Detected) => {
                (width, IndentSource::Detected)
            }
            (None, _, _) => (configured.width, IndentSource::Configured),
        };
        doc.indent = super::super::document::Indent {
            style,
            width,
            style_source,
            width_source,
        };
    }
}
