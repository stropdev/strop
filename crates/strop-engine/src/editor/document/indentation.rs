//! Bounded indentation evidence and per-property provenance.
/// Where one indent fact (style or width) came from (0051 R08).
/// Provenance is recorded separately for style and width — a manual
/// width over a detected style is a real combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentSource {
    /// `:tab-size N` / `:indent-style …` — wins over everything.
    Manual,
    /// Content detection at open (config `indent_detect`).
    Detected,
    /// config.toml's `tab_size`/`indent_style` (or the embedded default).
    Configured,
}

impl IndentSource {
    pub fn label(self) -> &'static str {
        match self {
            IndentSource::Manual => "manual",
            IndentSource::Detected => "detected",
            IndentSource::Configured => "configured",
        }
    }
}

/// One document's indent: manual override → confident detection →
/// config (0051 R08). Resolved at open/reload and after every
/// `:tab-size`/`:indent-style`; overrides on the document survive the
/// re-resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indent {
    pub style: crate::config::IndentStyle,
    pub width: usize,
    pub style_source: IndentSource,
    pub width_source: IndentSource,
}

impl Default for Indent {
    /// The pre-config fallback (spaces, 4); the Editor re-resolves from
    /// config/content at open and reload.
    fn default() -> Self {
        Self {
            style: crate::config::IndentStyle::Spaces,
            width: 4,
            style_source: IndentSource::Configured,
            width_source: IndentSource::Configured,
        }
    }
}

impl Indent {
    /// The unit auto-indent, `>>` and the Tab key emit.
    pub fn unit(&self) -> String {
        match self.style {
            crate::config::IndentStyle::Spaces => " ".repeat(self.width),
            crate::config::IndentStyle::Tabs => "\t".into(),
        }
    }

    /// The modeline segment: `Spaces:4` / `Tabs:4` (0051 R08).
    pub fn label(&self) -> String {
        let style = match self.style {
            crate::config::IndentStyle::Spaces => "Spaces",
            crate::config::IndentStyle::Tabs => "Tabs",
        };
        format!("{style}:{}", self.width)
    }
}

/// `:tab-size N` / `:indent-style …` (0051 R08): explicit per-buffer
/// choices. Each side wins over detection and config independently and
/// survives reloads and config refreshes; `None` defers to the
/// detection/config policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndentOverride {
    pub width: Option<usize>,
    pub style: Option<crate::config::IndentStyle>,
}

/// How much of the sample supports the conclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// ≥90% of the evidence agrees.
    High,
    /// The majority agrees, with real dissent.
    Low,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Low => "low",
        }
    }
}

/// Why detection stayed Unknown (0051 R08): the reason rides along so
/// `:explain` can say it instead of fabricating a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionFailure {
    /// Fewer than [`MIN_EVIDENCE`] lines carried real indentation.
    InsufficientEvidence,
    /// Tab- and space-indented lines are both common.
    MixedStyles,
    /// Two widths explain the sample equally well (divisor tie).
    Ambiguous,
    /// No candidate width covers enough of the space evidence.
    NoConsensus,
    /// Raw-string delimiters need language context; never infer from their body.
    AmbiguousLiteral,
}

impl DetectionFailure {
    pub fn reason(self) -> &'static str {
        match self {
            DetectionFailure::InsufficientEvidence => "too little indentation evidence",
            DetectionFailure::MixedStyles => "tab- and space-indented lines both common",
            DetectionFailure::Ambiguous => "two widths fit the evidence equally well",
            DetectionFailure::NoConsensus => "no width covers the space indentation",
            DetectionFailure::AmbiguousLiteral => {
                "raw-string whitespace is ambiguous without language context"
            }
        }
    }
}

/// What `detect_indent` concluded (0051 R08). A tab verdict carries no
/// width — a hard tab's display width is the configured/manual width,
/// never an inferred fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    Unknown(DetectionFailure),
    Tabs {
        evidence: usize,
        confidence: Confidence,
    },
    Spaces {
        width: usize,
        evidence: usize,
        confidence: Confidence,
    },
}

/// Sampling bounds (0051 R08): lines AND bytes — a minified file's
/// first thousand lines must not read the whole rope.
const SAMPLE_LINES: usize = 1000;
const SAMPLE_BYTES: usize = 256 * 1024;
/// Fewer evidence lines than this is a guess, not detection.
const MIN_EVIDENCE: usize = 4;
/// Exact space indents attested up to this depth; deeper lines still
/// vote through divisibility.
const MAX_ATTESTED: usize = 32;
/// Candidate space widths.
const UNITS: [usize; 4] = [2, 3, 4, 8];

#[derive(Clone, Copy)]
enum SampleState {
    Code,
    BlockComment,
    Quoted(u8, usize),
}

impl SampleState {
    fn prose_line(&mut self, line: ropey::RopeSlice<'_>) -> Result<bool, DetectionFailure> {
        let began_in_prose = !matches!(self, Self::Code);
        let mut at = 0;
        let same = |at: usize, byte: u8, count: usize| {
            at + count <= line.len_bytes() && (at..at + count).all(|index| line.byte(index) == byte)
        };
        while at < line.len_bytes() {
            let byte = line.byte(at);
            let next = (at + 1 < line.len_bytes()).then(|| line.byte(at + 1));
            match *self {
                Self::BlockComment => {
                    if byte == b'*' && next == Some(b'/') {
                        *self = Self::Code;
                        at += 2;
                    } else {
                        at += 1;
                    }
                }
                Self::Quoted(delimiter, count) => {
                    if byte == b'\\' {
                        at += 2;
                    } else if same(at, delimiter, count) {
                        *self = Self::Code;
                        at += count;
                    } else {
                        at += 1;
                    }
                }
                Self::Code => {
                    if byte == b'/' && next == Some(b'/') || byte == b'#' {
                        break;
                    }
                    if byte == b'/' && next == Some(b'*') {
                        *self = Self::BlockComment;
                        at += 2;
                    } else if matches!(byte, b'r' | b'R') && {
                        let mut end = at + 1;
                        while end < line.len_bytes() && line.byte(end) == b'#' {
                            end += 1;
                        }
                        end < line.len_bytes() && line.byte(end) == b'"'
                    } {
                        return Err(DetectionFailure::AmbiguousLiteral);
                    } else if matches!(byte, b'"' | b'\'') && same(at, byte, 3) {
                        *self = Self::Quoted(byte, 3);
                        at += 3;
                    } else if matches!(byte, b'"' | b'`') {
                        *self = Self::Quoted(byte, 1);
                        at += 1;
                    } else {
                        at += 1;
                    }
                }
            }
        }
        Ok(began_in_prose || !matches!(self, Self::Code))
    }
}

/// Detect a buffer's indent from its leading whitespace (0051 R08).
/// Only real block-indentation evidence votes: continuation alignment
/// (a line sitting deeper than a predecessor that ends open), blank
/// lines, comment lines and mixed-whitespace lines are excluded from
/// evidence AND from the denominator. A candidate width must be
/// attested at its own depth and cover ≥60% of the space evidence;
/// equal coverage between two candidates is ambiguity, not a license
/// to pick the largest. Streams
/// rope lines — no materialization.
pub fn detect_indent(text: &ropey::Rope) -> Detection {
    let mut tabs = 0usize;
    let mut spaced = 0usize;
    let mut attested = [0usize; MAX_ATTESTED + 1];
    let mut covered = [0usize; 9]; // per candidate unit, by index
    let mut continuation = false;
    let mut prev_indent = 0usize;
    let mut sampled = 0usize;
    let mut sample_state = SampleState::Code;
    for line in text.lines().take(SAMPLE_LINES) {
        sampled += line.len_bytes();
        if sampled > SAMPLE_BYTES {
            break;
        }
        match sample_state.prose_line(line) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(reason) => return Detection::Unknown(reason),
        }
        let mut it = line.bytes();
        let (mut spaces, mut tabs_lead) = (0usize, 0usize);
        let first = loop {
            match it.next() {
                Some(b' ') => spaces += 1,
                Some(b'\t') => tabs_lead += 1,
                other => break other,
            }
        };
        let Some(first) = first else {
            continue; // an empty line is not evidence
        };
        if first == b'\n' || first == b'\r' {
            continue; // whitespace-only line: not evidence
        }
        let second = it.next().filter(|b| *b != b'\n' && *b != b'\r');
        if comment_start(first, second) {
            continue; // comments align to prose, not blocks
        }
        // The tail decides whether the NEXT content line is alignment.
        // ropey's Bytes is not double-ended: walk the line's tail by
        // index instead of materializing it.
        let mut tail = None;
        for i in (0..line.len_bytes()).rev() {
            let b = line.byte(i);
            if !matches!(b, b'\n' | b'\r' | b' ' | b'\t') {
                tail = Some(b);
                break;
            }
        }
        // `(`/`[`/`,`/operators at end of line open an alignment
        // region; `{` opens a BLOCK — its body is the evidence we
        // want, so it is deliberately absent.
        let opens = matches!(
            tail,
            Some(
                b'(' | b'[' | b',' | b'=' | b'+' | b'-' | b'*' | b'/' | b'&' | b'|' | b'?' | b'\\'
            )
        );
        // Infix leaders (`.method()`, `,`) align to their leader, not
        // the block. Alignment lines sit DEEPER than the line they
        // continue; a sibling at the same depth after a comma (JSON
        // pairs, collection elements) is ordinary block evidence.
        let leading = spaces + tabs_lead;
        let infix = matches!(first, b'.' | b',');
        let evidence = !(continuation && leading > prev_indent) && !infix;
        continuation = opens;
        prev_indent = leading;
        if !evidence {
            continue;
        }
        if spaces > 0 && tabs_lead > 0 {
            continue; // mixed leading whitespace votes for nothing
        }
        if tabs_lead > 0 {
            tabs += 1;
            continue;
        }
        if spaces == 0 {
            continue; // indent 0 is the absence of indentation
        }
        spaced += 1;
        if spaces <= MAX_ATTESTED {
            attested[spaces] += 1;
        }
        for unit in UNITS {
            if spaces % unit == 0 {
                covered[unit] += 1;
            }
        }
    }
    let evidence = tabs + spaced;
    if evidence < MIN_EVIDENCE {
        return Detection::Unknown(DetectionFailure::InsufficientEvidence);
    }
    let confidence = |share: usize, base: usize| {
        if share * 10 >= base * 9 {
            Confidence::High
        } else {
            Confidence::Low
        }
    };
    // A style needs two thirds of the evidence to win; anything less
    // decisive is a mixed file, not a verdict.
    if tabs * 3 >= evidence * 2 {
        return Detection::Tabs {
            evidence: tabs,
            confidence: confidence(tabs, evidence),
        };
    }
    if spaced * 3 < evidence * 2 {
        return Detection::Unknown(DetectionFailure::MixedStyles);
    }
    let mut best: Option<(usize, usize)> = None; // (unit, covered)
    let mut tied = false;
    for unit in UNITS {
        // A width never attested at its own depth is a divisor of the
        // real width, not a candidate; sub-60% coverage is dissent.
        if attested[unit] == 0 || covered[unit] * 5 < spaced * 3 {
            continue;
        }
        match best {
            None => {
                best = Some((unit, covered[unit]));
                tied = false;
            }
            Some((_, share)) if covered[unit] > share => {
                best = Some((unit, covered[unit]));
                tied = false;
            }
            Some((_, share)) if covered[unit] == share => tied = true,
            _ => {}
        }
    }
    if tied {
        return Detection::Unknown(DetectionFailure::Ambiguous);
    }
    match best {
        Some((width, share)) => Detection::Spaces {
            width,
            evidence: spaced,
            confidence: confidence(share, spaced),
        },
        None => Detection::Unknown(DetectionFailure::NoConsensus),
    }
}

/// Comment-only lines are prose alignment, not block indentation:
/// `//`, `/*`, `*` (block-comment bodies), `#`, `--`, `;`.
fn comment_start(first: u8, second: Option<u8>) -> bool {
    match first {
        b'/' => matches!(second, Some(b'/') | Some(b'*')),
        b'*' | b'#' | b';' => true,
        b'-' => second == Some(b'-'),
        _ => false,
    }
}
#[cfg(test)]
mod detect_tests {
    //! 0051 R08: only real block-indentation evidence decides; ties
    //! and thin samples are Unknown, never confident guesses.
    use super::*;

    fn detect(text: &str) -> Detection {
        detect_indent(&ropey::Rope::from_str(text))
    }

    fn spaces(text: &str) -> (usize, Confidence) {
        match detect(text) {
            Detection::Spaces {
                width, confidence, ..
            } => (width, confidence),
            other => panic!("expected spaces, got {other:?}"),
        }
    }

    #[test]
    fn common_widths() {
        let two = "fn f() {\n  a;\n  b;\n  c;\n  d;\n}\n";
        assert_eq!(spaces(two).0, 2);
        let three = "fn f() {\n   a;\n   b;\n   c;\n   d;\n}\n";
        assert_eq!(spaces(three).0, 3);
        let four = "fn f() {\n    a;\n    b;\n    c;\n    d;\n}\n";
        assert_eq!(spaces(four).0, 4);
        // an eight-space file must not collapse onto its divisors:
        // 2 and 4 are never attested at their own depth
        let eight = "fn f() {\n        a;\n        b;\n                c;\n                d;\n}\n";
        assert_eq!(spaces(eight).0, 8);
    }

    #[test]
    fn tabs_carry_no_width() {
        match detect("fn f() {\n\ta;\n\tb;\n\t\tc;\n\td;\n}\n") {
            Detection::Tabs { evidence, .. } => assert_eq!(evidence, 4),
            other => panic!("expected tabs, got {other:?}"),
        }
    }

    #[test]
    fn deep_nesting_is_evidence() {
        // 12/16/20/24-space lines vote for 4 through divisibility; the
        // old histogram dropped everything past 8 spaces
        let text = "fn f() {\n    a;\n        b;\n            c;\n                d;\n                    e;\n}\n";
        let (width, confidence) = spaces(text);
        assert_eq!(width, 4);
        assert_eq!(confidence, Confidence::High);
    }

    #[test]
    fn comments_are_not_evidence() {
        // comment lines at odd/prose indents cannot drag the verdict
        let text = "fn f() {\n    a;\n     // note\n       # shell-ish\n     * doc body\n    b;\n    c;\n    d;\n}\n";
        let (width, confidence) = spaces(text);
        assert_eq!(width, 4);
        assert_eq!(
            confidence,
            Confidence::High,
            "comments stayed out of the denominator"
        );
        // a file that is all comments plus one code line is thin
        let text = "// a\n  // b\n    // c\n      // d\nx;\n";
        assert_eq!(
            detect(text),
            Detection::Unknown(DetectionFailure::InsufficientEvidence)
        );
    }

    #[test]
    fn comma_siblings_at_the_same_depth_are_evidence() {
        // JSON/YAML-style: lines after a comma at the SAME depth are
        // block siblings, not continuation alignment
        let text = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3,\n  \"d\": 4\n}\n";
        let (width, confidence) = spaces(text);
        assert_eq!(width, 2);
        assert_eq!(confidence, Confidence::High);
    }

    #[test]
    fn continuation_alignment_is_not_evidence() {
        // the 19-space alignment under `foo(a,` fits no unit; counted,
        // it would only dissent — excluded, confidence stays high
        let text =
            "fn f() {\n    let x = foo(a,\n                   b);\n    c;\n    d;\n    e;\n}\n";
        let (width, confidence) = spaces(text);
        assert_eq!(width, 4);
        assert_eq!(confidence, Confidence::High);
        // infix leaders align to their leader, not the block
        let text =
            "fn f() {\n    let x = foo\n        .a()\n        .b();\n    c;\n    d;\n    e;\n}\n";
        let (width, confidence) = spaces(text);
        assert_eq!(width, 4);
        assert_eq!(confidence, Confidence::High);
    }

    #[test]
    fn mixed_styles_are_unknown() {
        let text = "fn f() {\n\ta;\n\tb;\n    c;\n    d;\n\te;\n    f;\n}\n";
        assert_eq!(
            detect(text),
            Detection::Unknown(DetectionFailure::MixedStyles)
        );
    }

    #[test]
    fn thin_samples_are_unknown() {
        assert_eq!(
            detect("\t\tx = 1\n"),
            Detection::Unknown(DetectionFailure::InsufficientEvidence)
        );
        assert_eq!(
            detect("no indent here\nat all\nreally\nnone\n"),
            Detection::Unknown(DetectionFailure::InsufficientEvidence)
        );
    }

    #[test]
    fn divisor_ties_are_ambiguous() {
        // 2 and 3 cover the sample equally well; the answer must not
        // be a confident pick of either
        let text = "fn f() {\n  a;\n   b;\n      c;\n            d;\n}\n";
        assert_eq!(
            detect(text),
            Detection::Unknown(DetectionFailure::Ambiguous)
        );
    }

    #[test]
    fn blank_and_top_level_lines_are_ignored() {
        let text = "top;\n\n    a;\n   \n    b;\nmore;\n    c;\n    d;\n";
        let (width, _) = spaces(text);
        assert_eq!(width, 4);
    }

    #[test]
    fn sampling_is_bounded_by_lines() {
        // 1000 space lines inside the bound; 1500 tab lines after it
        // must not flip the verdict
        let mut text = String::from("fn f() {\n");
        for _ in 0..1000 {
            text.push_str("    a;\n");
        }
        for _ in 0..1500 {
            text.push_str("\tb;\n");
        }
        assert_eq!(spaces(&text).0, 4);
    }

    #[test]
    fn sampling_is_bounded_by_bytes() {
        // the space lines alone exceed the byte budget, so the tab
        // lines after them are never sampled
        let mut text = String::from("fn f() {\n");
        let long = format!("    {}\n", "x".repeat(600));
        for _ in 0..600 {
            text.push_str(&long);
        }
        for _ in 0..500 {
            text.push_str("\tb;\n");
        }
        const { assert!(600 * 605 > SAMPLE_BYTES, "the fixture exceeds the budget") }
        assert_eq!(spaces(&text).0, 4);
    }
    #[test]
    fn multiline_prose_never_votes_for_a_code_indent() {
        let python = "def f():\n  a = 1\n  text = \"\"\"\n     prose\n     prose\n     prose\n     prose\n  \"\"\"\n  b = 2\n  c = 3\n  d = 4\n";
        assert_eq!(spaces(python).0, 2);
        let comment = "fn f() {\n    a;\n/*\n   prose\n   prose\n   prose\n   prose\n*/\n    b;\n    c;\n    d;\n}\n";
        assert_eq!(spaces(comment).0, 4);
    }
}
