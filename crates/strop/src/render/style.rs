//! The per-frame style table (0088 §2): every colour a renderer paints
//! comes from one `Tokens` value built once per frame from the editor's
//! palette and passed by reference. Nothing in `render/` names a hex
//! value or reads a global.

use ratatui::style::{Color, Modifier, Style};
use strop_core::theme::{Rgb, Theme, ThemeId};

use crate::editor::Mode;

pub(crate) const fn color(value: Rgb) -> Color {
    Color::Rgb(value.r, value.g, value.b)
}

/// Syntax inks, one per `strop_syntax::Class` family.
#[derive(Clone, Copy)]
pub(crate) struct SyntaxInks {
    pub keyword: Color,
    pub function: Color,
    pub r#type: Color,
    pub string: Color,
    pub number: Color,
    pub constant: Color,
    pub attribute: Color,
    pub operator: Color,
    pub punctuation: Color,
    pub comment: Color,
    pub variable: Color,
    pub namespace: Color,
    pub heading: Color,
    pub link: Color,
    pub code: Color,
}

/// The token set as toolkit colours.
#[derive(Clone, Copy)]
pub(crate) struct Tokens {
    pub id: ThemeId,
    // planes and overlays
    pub base: Color,
    pub surface: Color,
    pub raised: Color,
    pub cursorline: Color,
    pub selection: Color,
    pub preview: Color,
    pub flash: Color,
    pub pair: Color,
    pub diff_add_bg: Color,
    pub diff_del_bg: Color,
    pub diff_add_strong_bg: Color,
    pub diff_del_strong_bg: Color,
    // inks
    pub text: Color,
    pub secondary: Color,
    pub muted: Color,
    pub faint: Color,
    pub guide: Color,
    pub on_accent: Color,
    pub accent: Color,
    pub mode_insert: Color,
    pub mode_visual: Color,
    pub mode_pending: Color,
    pub mode_terminal: Color,
    pub error: Color,
    pub warning: Color,
    pub info: Color,
    pub hint: Color,
    pub diff_add: Color,
    pub diff_del: Color,
    pub diff_change: Color,
    pub syn: SyntaxInks,
    base_rgb: Rgb,
}

impl Tokens {
    pub(crate) fn new(id: ThemeId) -> Self {
        let theme: &Theme = id.palette();
        let (bg, fg, syn) = (theme.bg, theme.fg, theme.syn);
        Self {
            id,
            base: color(bg.base),
            surface: color(bg.surface),
            raised: color(bg.raised),
            cursorline: color(bg.cursorline),
            selection: color(bg.selection),
            preview: color(bg.preview),
            flash: color(bg.flash),
            pair: color(bg.pair),
            diff_add_bg: color(bg.diff_add),
            diff_del_bg: color(bg.diff_del),
            diff_add_strong_bg: color(bg.diff_add_strong),
            diff_del_strong_bg: color(bg.diff_del_strong),
            text: color(fg.text),
            secondary: color(fg.secondary),
            muted: color(fg.muted),
            faint: color(fg.faint),
            guide: color(fg.guide),
            on_accent: color(fg.on_accent),
            accent: color(theme.accent),
            mode_insert: color(theme.mode.insert),
            mode_visual: color(theme.mode.visual),
            mode_pending: color(theme.mode.pending),
            mode_terminal: color(theme.mode.terminal),
            error: color(theme.diag.error),
            warning: color(theme.diag.warning),
            info: color(theme.diag.info),
            hint: color(theme.diag.hint),
            diff_add: color(theme.diff.add),
            diff_del: color(theme.diff.del),
            diff_change: color(theme.diff.change),
            syn: SyntaxInks {
                keyword: color(syn.keyword),
                function: color(syn.function),
                r#type: color(syn.r#type),
                string: color(syn.string),
                number: color(syn.number),
                constant: color(syn.constant),
                attribute: color(syn.attribute),
                operator: color(syn.operator),
                punctuation: color(syn.punctuation),
                comment: color(syn.comment),
                variable: color(syn.variable),
                namespace: color(syn.namespace),
                heading: color(syn.heading),
                link: color(syn.link),
                code: color(syn.code),
            },
            base_rgb: bg.base,
        }
    }

    /// Mode chip colours (0001 §4): the mode is a colour change, not a
    /// bar. Normal is the accent itself.
    pub(crate) fn mode(&self, mode: Mode) -> Color {
        match mode {
            Mode::Normal => self.accent,
            Mode::Insert => self.mode_insert,
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => self.mode_visual,
        }
    }

    /// Diagnostic severity → ink; one source for the gutter sign, the
    /// EOL note, the statusline counts and the diagnostics picker.
    pub(crate) fn severity(&self, severity: strop_lsp::Severity) -> Color {
        use strop_lsp::Severity;
        match severity {
            Severity::Error => self.error,
            Severity::Warning => self.warning,
            Severity::Information => self.info,
            Severity::Hint => self.hint,
        }
    }

    /// Syntax class → ink (one projection; `:theme` swaps the palette).
    pub(crate) fn class(&self, class: strop_syntax::Class) -> Color {
        use strop_syntax::Class as C;
        match class {
            C::Keyword => self.syn.keyword,
            C::Function => self.syn.function,
            C::Type => self.syn.r#type,
            C::String => self.syn.string,
            C::Comment | C::Quote => self.syn.comment,
            C::Number => self.syn.number,
            C::Operator => self.syn.operator,
            C::Punctuation => self.syn.punctuation,
            C::Constant => self.syn.constant,
            C::Attribute => self.syn.attribute,
            C::Variable => self.syn.variable,
            C::Heading | C::List => self.syn.heading,
            C::Link | C::Tag => self.syn.link,
            C::Code => self.syn.code,
        }
    }

    /// One style projection for panes and picker previews; overlays
    /// compose later. Comments are italic by contract (rule 7).
    pub(crate) fn syntax(&self, span: &strop_syntax::Span) -> Style {
        let mut style = Style::default().fg(self.class(span.class));
        if span.emphasis.bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if span.emphasis.italic || matches!(span.class, strop_syntax::Class::Comment) {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if span.emphasis.underline {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        if span.emphasis.strikethrough {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        style
    }

    /// Completion/symbol kind → ink. One vocabulary projection shared by
    /// the picker's chips and the completion menu (0050 §4): four quiet
    /// families — callables, types, data, modules — unknown kinds neutral.
    pub(crate) fn kind(&self, kind: &str) -> Color {
        match kind {
            "fn" | "meth" | "new" | "func" | "ctor" => self.syn.function,
            "struct" | "class" | "iface" | "enum" | "variant" | "T" | "type" | "unit" => {
                self.syn.r#type
            }
            "const" | "var" | "field" | "prop" | "value" => self.syn.string,
            "mod" | "ns" | "pkg" => self.syn.namespace,
            _ => self.secondary,
        }
    }

    /// Completion provenance → quiet ink. The ASCII tag stays the icon
    /// (no Nerd Font glyphs): language service in the type ink, buffer
    /// words in the string ink.
    pub(crate) fn source(&self, source: &str) -> Color {
        match source {
            "lsp" => self.syn.r#type,
            "buf" => self.syn.string,
            _ => self.muted,
        }
    }

    /// Pull a colour toward the base for the picker's dimmed backdrop.
    pub(crate) fn dim(&self, c: Color) -> Color {
        match c {
            Color::Rgb(r, g, b) => {
                let base = self.base_rgb;
                let m = |a: u8, b: u8| (u16::from(a) * 45 / 100 + u16::from(b) * 55 / 100) as u8;
                Color::Rgb(m(r, base.r), m(g, base.g), m(b, base.b))
            }
            other => other,
        }
    }
}

/// Linear RGB ramp for the cursor fade (0064 §2). Every token is RGB, so
/// the interpolation is total.
pub(crate) fn fade_mix(from: Color, to: Color, pct: u8) -> Color {
    let (Color::Rgb(fr, fg, fb), Color::Rgb(tr, tg, tb)) = (from, to) else {
        return to;
    };
    let mix =
        |a: u8, b: u8| (i32::from(a) + (i32::from(b) - i32::from(a)) * i32::from(pct) / 100) as u8;
    Color::Rgb(mix(fr, tr), mix(fg, tg), mix(fb, tb))
}
