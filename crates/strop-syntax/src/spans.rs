//! Resolve overlapping captures into disjoint byte intervals. Injected languages
//! own their foreground while outer Markdown emphasis remains compositional.
use super::{Class, Span};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Emphasis {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}
impl Emphasis {
    pub(crate) fn from_capture(name: &str) -> Self {
        Self {
            bold: name.starts_with("markup.heading") || name == "markup.bold",
            italic: name.starts_with("comment") || name == "markup.italic",
            underline: name.starts_with("markup.link.url"),
            strikethrough: name == "markup.strikethrough",
        }
    }
    fn merge(&mut self, other: Self) {
        self.bold |= other.bold;
        self.italic |= other.italic;
        self.underline |= other.underline;
        self.strikethrough |= other.strikethrough;
    }
}
#[derive(Clone, Copy)]
pub(crate) struct CaptureStyle {
    pub class: Class,
    pub emphasis: Emphasis,
}
pub(crate) struct LayeredSpan {
    pub span: Span,
    pub injected: bool,
}

pub(crate) fn flatten(captures: Vec<LayeredSpan>, first: usize, last: usize) -> Vec<Span> {
    let mut events = Vec::with_capacity(captures.len().saturating_mul(2));
    for (index, capture) in captures.iter().enumerate() {
        let start = capture.span.start.max(first);
        let end = capture.span.end.min(last);
        if start < end {
            events.push((start, true, index));
            events.push((end, false, index));
        }
    }
    events.sort_unstable();
    let mut active = Vec::new();
    let mut result: Vec<Span> = Vec::new();
    let mut at = 0;
    while at < events.len() {
        let start = events[at].0;
        while at < events.len() && events[at].0 == start {
            let (_, entering, index) = events[at];
            if entering {
                active.push(index);
            } else {
                active.retain(|&value| value != index);
            }
            at += 1;
        }
        let Some(&(end, _, _)) = events.get(at) else {
            break;
        };
        let Some(&winner) = active.iter().max_by_key(|&&index| {
            let capture = &captures[index];
            (
                capture.injected,
                usize::MAX - (capture.span.end - capture.span.start),
                index,
            )
        }) else {
            continue;
        };
        let mut emphasis = Emphasis::default();
        for &index in &active {
            emphasis.merge(captures[index].span.emphasis);
        }
        let class = captures[winner].span.class;
        if let Some(previous) = result
            .last_mut()
            .filter(|span| span.end == start && span.class == class && span.emphasis == emphasis)
        {
            previous.end = end;
        } else {
            result.push(Span {
                start,
                end,
                class,
                emphasis,
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_emphasis_survives_an_injected_foreground() {
        let captures = vec![
            LayeredSpan {
                span: Span {
                    start: 0,
                    end: 10,
                    class: Class::Variable,
                    emphasis: Emphasis {
                        bold: true,
                        ..Emphasis::default()
                    },
                },
                injected: false,
            },
            LayeredSpan {
                span: Span {
                    start: 2,
                    end: 8,
                    class: Class::Variable,
                    emphasis: Emphasis {
                        italic: true,
                        ..Emphasis::default()
                    },
                },
                injected: false,
            },
            LayeredSpan {
                span: Span {
                    start: 3,
                    end: 5,
                    class: Class::Keyword,
                    emphasis: Emphasis::default(),
                },
                injected: true,
            },
        ];
        let result = flatten(captures, 0, 10);
        let keyword = result.iter().find(|span| span.start == 3).unwrap();
        assert_eq!(keyword.class, Class::Keyword);
        assert!(keyword.emphasis.bold && keyword.emphasis.italic);
        assert_eq!(result.last().unwrap().end, 10);
        assert!(result
            .windows(2)
            .all(|spans| spans[0].end <= spans[1].start));
    }
}
