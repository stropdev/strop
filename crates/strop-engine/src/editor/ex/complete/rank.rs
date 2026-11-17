//! Ranking (plan 0110 §3.2): the command `Enter` would run for the typed
//! word first, then the other names that start with it in table order,
//! then fuzzy matches by score. The fuzzy scorer is the picker's
//! (nucleo, one thread-local matcher), called on plain text: no filter
//! language, and no allocation per key for ASCII names.

use std::cell::RefCell;

use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::super::table::COMMANDS;
use super::{Row, Text};

thread_local! {
    static MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new(Config::DEFAULT));
}

/// The fuzzy score of `word` in `text` (smart case), and the matched
/// character positions as a bitmask of the first 64 characters.
pub(crate) fn fuzzy(word: &str, text: &str, indices: &mut Vec<u32>) -> Option<(u16, u64)> {
    if !word.is_ascii() || !text.is_ascii() {
        return fuzzy_unicode(word, text);
    }
    MATCHER.with(|matcher| {
        let mut matcher = matcher.borrow_mut();
        matcher.config.ignore_case = !word.bytes().any(|b| b.is_ascii_uppercase());
        indices.clear();
        let score = matcher.fuzzy_indices(
            Utf32Str::Ascii(text.as_bytes()),
            Utf32Str::Ascii(word.as_bytes()),
            indices,
        )?;
        Some((score, mask(indices)))
    })
}

/// Names and paths past ASCII take the allocating path (never a command).
fn fuzzy_unicode(word: &str, text: &str) -> Option<(u16, u64)> {
    let (score, columns) = strop_picker::fuzzy_score(word, text)?;
    Some((
        u16::try_from(score.max(0)).unwrap_or(u16::MAX),
        mask(&columns),
    ))
}

fn mask(indices: &[u32]) -> u64 {
    indices
        .iter()
        .filter(|&&at| at < 64)
        .fold(0, |mask, &at| mask | 1 << at)
}

/// The first `count` characters matched.
pub(crate) fn prefix_mask(count: usize) -> u64 {
    if count >= 64 {
        u64::MAX
    } else {
        (1u64 << count) - 1
    }
}

/// Scratch kept between keys so ranking never allocates once warm.
#[derive(Debug, Default)]
pub(crate) struct Scratch {
    indices: Vec<u32>,
    /// (score, table index, matched) for the fuzzy tier.
    fuzzy: Vec<(u16, usize, u64, &'static str)>,
}

impl Scratch {
    /// The first use takes the room every later key needs: ranking never
    /// allocates again (a test counts).
    fn warm(&mut self, rows: &mut Vec<Row>) {
        if self.fuzzy.capacity() >= COMMANDS.len() {
            return;
        }
        self.fuzzy.reserve(COMMANDS.len());
        self.indices.reserve(64);
        rows.reserve(COMMANDS.len());
        MATCHER.with(|_| ());
    }
}

/// Command rows for the typed `word`, into `rows` (cleared first).
pub(crate) fn commands(word: &str, rows: &mut Vec<Row>, scratch: &mut Scratch) {
    scratch.warm(rows);
    rows.clear();
    // 1. what Enter runs for this word
    let enter = super::super::command_word(word)
        .filter(|(_, _, length)| *length == word.len())
        .map(|(command, name, _)| (command, name));
    let enter_index =
        enter.and_then(|(command, _)| COMMANDS.iter().position(|row| std::ptr::eq(row, command)));
    if let Some((command, name)) = enter {
        rows.push(Row::command(
            command,
            name,
            prefix_mask(word.chars().count()),
        ));
    }
    // 2. other names that start with it, in table order
    for (index, command) in COMMANDS.iter().enumerate() {
        if Some(index) == enter_index {
            continue;
        }
        if let Some(name) = command
            .names
            .iter()
            .find(|name| name.name.starts_with(word))
        {
            rows.push(Row::command(
                command,
                name.name,
                prefix_mask(word.chars().count()),
            ));
        }
    }
    // one letter is a prefix, never a fuzzy query
    if word.chars().count() < 2 {
        return;
    }
    // 3. fuzzy matches by score (ties keep table order)
    scratch.fuzzy.clear();
    for (index, command) in COMMANDS.iter().enumerate() {
        if Some(index) == enter_index
            || command.names.iter().any(|name| name.name.starts_with(word))
        {
            continue;
        }
        let best = command
            .names
            .iter()
            .filter_map(|name| {
                fuzzy(word, name.name, &mut scratch.indices)
                    .map(|(score, matched)| (score, matched, name.name))
            })
            .max_by_key(|(score, _, _)| *score);
        if let Some((score, matched, name)) = best {
            scratch.fuzzy.push((score, index, matched, name));
        }
    }
    scratch
        .fuzzy
        .sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for &(_, index, matched, name) in &scratch.fuzzy {
        rows.push(Row::command(&COMMANDS[index], name, matched));
    }
}

/// Rows from a fixed vocabulary (words, themes, help topics): prefix
/// matches in order, then fuzzy ones by score.
pub(crate) fn fixed(
    word: &str,
    candidates: impl Iterator<Item = (&'static str, &'static str)> + Clone,
    rows: &mut Vec<Row>,
    scratch: &mut Scratch,
) {
    rows.clear();
    let prefix = |text: &str| text.starts_with(word);
    for (text, doc) in candidates.clone().filter(|(text, _)| prefix(text)) {
        rows.push(Row::word(text, doc, prefix_mask(word.chars().count())));
    }
    if word.is_empty() {
        return;
    }
    let mut scored: Vec<_> = candidates
        .filter(|(text, _)| !prefix(text))
        .filter_map(|(text, doc)| {
            fuzzy(word, text, &mut scratch.indices)
                .map(|(score, matched)| (score, text, doc, matched))
        })
        .collect();
    scored.sort_by_key(|row| std::cmp::Reverse(row.0));
    rows.extend(
        scored
            .into_iter()
            .map(|(_, text, doc, matched)| Row::word(text, doc, matched)),
    );
}

impl Row {
    fn command(
        command: &'static super::super::table::ExCommand,
        name: &'static str,
        matched: u64,
    ) -> Self {
        let min = command
            .names
            .iter()
            .find(|candidate| candidate.name == name)
            .map_or(name.len(), |candidate| candidate.min);
        Row {
            text: Text::Static(name),
            label: None,
            abbrev: Text::Static(&name[..min]),
            doc: Text::Static(command.doc),
            key: command.key,
            matched,
            mark: false,
        }
    }

    fn word(text: &'static str, doc: &'static str, matched: u64) -> Self {
        Row {
            text: Text::Static(text),
            label: None,
            abbrev: Text::Static(""),
            doc: Text::Static(doc),
            key: None,
            matched,
            mark: false,
        }
    }
}
