use super::*;
use strop_core::{Buffer, Range};

fn build_index(buffer: &Buffer) -> WordIndex {
    WordIndex::build(buffer.revision(), buffer.snapshot(), || false)
        .unwrap()
        .0
}

fn journal_for(index: &WordIndex) -> WordJournal {
    let Snapshot::Current { revision, rope } = &index.snapshot else {
        panic!("test needs a current index");
    };
    WordJournal::new(*revision, rope.len_bytes())
}

fn advance(index: &mut WordIndex, buffer: &mut Buffer) -> IndexWork {
    let mut journal = journal_for(index);
    journal.record(buffer.changes()).unwrap();
    let work = index
        .advance(buffer.snapshot(), &journal, || false)
        .unwrap();
    buffer.clear_changes();
    work
}

// Independent whole-string tokenization is an oracle, not the production
// scanner or dirty-window mapper. Only the keyword policy is intentionally shared.
fn assert_counts(index: &WordIndex, buffer: &Buffer) {
    let text = buffer.text().to_string();
    let mut expected = BTreeMap::<String, u64>::new();
    for word in text.split(|character| !strop_grammar::is_keyword(character)) {
        if !word.is_empty() && word.len() <= MAX_WORD_BYTES {
            *expected.entry(word.to_string()).or_default() += 1;
        }
    }
    let actual: BTreeMap<_, _> = index
        .words
        .iter()
        .map(|(word, &count)| (word.to_string(), count))
        .collect();
    assert_eq!(actual, expected, "source: {text:?}");
    assert_eq!(index.revision(), Some(buffer.revision()));
}

#[test]
fn unicode_keywords_keep_spelling_and_rank_by_occurrences_then_spelling() {
    let buffer = Buffer::from_text("alpha αλφα\r\nalpha alpine _name _name 🦀 élan\n");
    let index = build_index(&buffer);
    let words = index
        .query(buffer.revision(), "al", false, || false)
        .unwrap();
    assert_eq!(
        words.items,
        vec![
            WordCandidate {
                text: Arc::from("alpha"),
                occurrences: 2
            },
            WordCandidate {
                text: Arc::from("alpine"),
                occurrences: 1
            },
        ]
    );
    assert_eq!(
        index
            .query(buffer.revision(), "α", false, || false)
            .unwrap()
            .items,
        vec![WordCandidate {
            text: Arc::from("αλφα"),
            occurrences: 1
        }]
    );
    assert_eq!(
        index
            .query(buffer.revision(), "élan", false, || false)
            .unwrap()
            .items,
        vec![]
    );
    assert_eq!(
        index
            .query(buffer.revision(), "élan", true, || false)
            .unwrap()
            .items,
        vec![WordCandidate {
            text: Arc::from("élan"),
            occurrences: 1
        }]
    );
    assert_counts(&index, &buffer);
}

#[test]
fn sequential_interacting_journals_match_an_independent_full_index() {
    let mut buffer = Buffer::from_text("alpha beta\r\nβeta élan _name foo_bar 😀 punctuation!\n");
    let mut index = build_index(&buffer);
    let replacements = ["alpha", "_", " ", "\r\n", "βeta", "😀", "", "é"];
    let mut state = 0x179b_341du64;
    for batch in 0..160 {
        for edit in 0..=batch % 7 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let characters = buffer.text().len_chars();
            let start = (state as usize) % (characters + 1);
            let count = (state.rotate_right(13) as usize % 6).min(characters - start);
            let start_byte = buffer.text().char_to_byte(start);
            let end_byte = buffer.text().char_to_byte(start + count);
            buffer
                .edit()
                .replace(
                    Range::charwise(start_byte, end_byte),
                    replacements[(batch + edit) % replacements.len()],
                )
                .unwrap();
        }
        advance(&mut index, &mut buffer);
        assert_counts(&index, &buffer);
    }
}

#[test]
fn distant_cursor_edits_do_not_rescan_the_intervening_megabyte_line() {
    let mut text = String::from("alpha ");
    text.push_str(&"filler ".repeat(160_000));
    text.push_str("omega");
    let mut buffer = Buffer::from_text(&text);
    let mut index = build_index(&buffer);
    buffer.edit().insert(1usize, "l").unwrap();
    let last = buffer.len_bytes() - 1;
    buffer.edit().insert(last, "g").unwrap();
    let work = advance(&mut index, &mut buffer);
    assert!(
        work.scanned_bytes < 4096,
        "ordinary edits rescanned {} bytes",
        work.scanned_bytes
    );
    assert_eq!(
        index
            .query(buffer.revision(), "fi", false, || false)
            .unwrap()
            .items,
        vec![WordCandidate {
            text: Arc::from("filler"),
            occurrences: 160_000
        }]
    );
    assert_counts(&index, &buffer);
}

#[test]
fn interrupted_maintenance_never_publishes_partially_updated_counts() {
    let mut buffer = Buffer::from_text(&"alpha ".repeat(4096));
    let mut index = build_index(&buffer);
    let before = buffer.revision();
    let length = buffer.len_bytes();
    buffer
        .edit()
        .replace(Range::charwise(0, length), "beta")
        .unwrap();
    let mut journal = journal_for(&index);
    journal.record(buffer.changes()).unwrap();
    let mut polls = 0;
    assert_eq!(
        index.advance(buffer.snapshot(), &journal, || {
            polls += 1;
            polls >= 4
        }),
        Err(IndexError::Cancelled)
    );
    assert_eq!(
        index.query(before, "a", false, || false),
        Err(IndexError::StaleRevision)
    );
    assert_eq!(
        index.query(buffer.revision(), "b", false, || false),
        Err(IndexError::StaleRevision)
    );
}

#[test]
fn a_missing_revision_cannot_be_relabelled_as_the_new_snapshot() {
    let mut buffer = Buffer::from_text("alpha");
    let mut index = build_index(&buffer);
    let before = buffer.revision();
    buffer.edit().insert(0usize, "beta ").unwrap();
    buffer.edit().insert(0usize, "gamma ").unwrap();
    let mut missing = journal_for(&index);
    assert_eq!(
        missing.record(&buffer.changes()[1..]),
        Err(IndexError::JournalGap)
    );
    assert_eq!(
        index.advance(buffer.snapshot(), &missing, || false),
        Err(IndexError::JournalGap)
    );
    assert_eq!(
        index.query(buffer.revision(), "g", false, || false),
        Err(IndexError::StaleRevision)
    );
    assert_eq!(
        index.query(before, "a", false, || false).unwrap().items,
        vec![WordCandidate {
            text: Arc::from("alpha"),
            occurrences: 1
        }]
    );
    let mut invalid = buffer.changes().to_vec();
    invalid[0].edit.old_end_byte = usize::MAX;
    let mut invalid_journal = journal_for(&index);
    assert_eq!(
        invalid_journal.record(&invalid),
        Err(IndexError::InvalidGeometry)
    );
    assert_eq!(
        index.advance(buffer.snapshot(), &invalid_journal, || false),
        Err(IndexError::InvalidGeometry)
    );
}

#[test]
fn oversized_unicode_words_can_split_and_merge_without_indexing_cropped_edges() {
    let long = "é".repeat(MAX_WORD_BYTES / 2 + 1);
    let mut buffer = Buffer::from_text(&format!("{long} alpha"));
    let mut index = build_index(&buffer);
    assert_eq!(index.coverage, IndexCoverage::Limited);
    assert_counts(&index, &buffer);
    buffer.edit().insert(MAX_WORD_BYTES / 2, " ").unwrap();
    advance(&mut index, &mut buffer);
    assert_counts(&index, &buffer);
    buffer
        .edit()
        .replace(
            Range::charwise(MAX_WORD_BYTES / 2, MAX_WORD_BYTES / 2 + 1),
            "",
        )
        .unwrap();
    advance(&mut index, &mut buffer);
    assert_counts(&index, &buffer);

    let mut huge = Buffer::from_text(&"x".repeat(1024 * 1024));
    let mut huge_index = build_index(&huge);
    huge.edit().insert(512 * 1024usize, "y").unwrap();
    let work = advance(&mut huge_index, &mut huge);
    assert!(work.scanned_bytes <= 4 * MAX_WORD_BYTES + 1);
    assert_eq!(huge_index.retained_words(), 0);
    assert_eq!(
        huge_index
            .query(huge.revision(), "x", false, || false)
            .unwrap()
            .coverage,
        IndexCoverage::Limited
    );
}

#[test]
fn retained_word_count_and_text_bytes_have_independent_bounds() {
    let text = (0..MAX_UNIQUE_WORDS + 1)
        .map(|n| format!("word{n:06} "))
        .collect::<String>();
    let buffer = Buffer::from_text(&text);
    let index = build_index(&buffer);
    assert_eq!(index.retained_words(), MAX_UNIQUE_WORDS);
    let result = index
        .query(buffer.revision(), "word", false, || false)
        .unwrap();
    assert_eq!(result.items[0].text.as_ref(), "word000000");
    assert_eq!(result.items.len(), MAX_QUERY_WORDS);
    assert!(result.truncated);
    assert_eq!(result.coverage, IndexCoverage::Limited);
    let mut polls = 0;
    assert_eq!(
        index.query(buffer.revision(), "word", false, || {
            polls += 1;
            polls > 1
        }),
        Err(IndexError::Cancelled)
    );
    assert_eq!(
        index
            .query(buffer.revision(), "word", false, || false)
            .unwrap(),
        result
    );

    let suffix = "x".repeat(MAX_WORD_BYTES - 16);
    let text = (0..MAX_INDEX_WORD_BYTES / suffix.len() + 1)
        .map(|n| format!("word{n:06}{suffix} "))
        .collect::<String>();
    let buffer = Buffer::from_text(&text);
    let index = build_index(&buffer);
    assert!(index.retained_words() < MAX_UNIQUE_WORDS);
    assert!(index.retained_word_bytes() <= MAX_INDEX_WORD_BYTES);
    assert_eq!(
        index
            .query(buffer.revision(), "word", false, || false)
            .unwrap()
            .coverage,
        IndexCoverage::Limited
    );
}

#[test]
fn sustained_typing_compacts_geometry_without_restarting_a_busy_index() {
    let mut buffer = Buffer::from_text("alpha ");
    let mut index = build_index(&buffer);
    let mut journal = journal_for(&index);
    for _ in 0..5000 {
        let end = buffer.len_bytes();
        buffer.edit().insert(end, "beta ").unwrap();
        journal.record(buffer.changes()).unwrap();
        buffer.clear_changes();
        assert_eq!(journal.retained_regions(), 1);
    }
    index
        .advance(buffer.snapshot(), &journal, || false)
        .unwrap();
    assert_eq!(
        index
            .query(buffer.revision(), "b", false, || false)
            .unwrap()
            .items,
        vec![WordCandidate {
            text: Arc::from("beta"),
            occurrences: 5000
        }]
    );
    assert_counts(&index, &buffer);
    journal.reset(buffer.revision(), buffer.len_bytes());
    assert_eq!(journal.retained_regions(), 0);
    buffer.edit().insert(0usize, "gamma ").unwrap();
    journal.record(buffer.changes()).unwrap();
    index
        .advance(buffer.snapshot(), &journal, || false)
        .unwrap();
    assert_counts(&index, &buffer);
}

#[test]
fn excessive_disjoint_updates_require_a_new_snapshot_without_retaining_a_backlog() {
    let mut buffer = Buffer::from_text(&"aa ".repeat(MAX_JOURNAL_CHANGES + 2));
    let mut index = build_index(&buffer);
    let mut journal = journal_for(&index);
    for position in 0..=MAX_JOURNAL_CHANGES {
        let at = position * 3;
        buffer
            .edit()
            .replace(Range::charwise(at, at + 1), "b")
            .unwrap();
        let admitted = journal.record(buffer.changes());
        buffer.clear_changes();
        if position == MAX_JOURNAL_CHANGES {
            assert_eq!(admitted, Err(IndexError::JournalLimit));
        } else {
            admitted.unwrap();
        }
    }
    assert!(journal.retained_regions() <= MAX_JOURNAL_CHANGES);
    assert_eq!(
        index.advance(buffer.snapshot(), &journal, || false),
        Err(IndexError::JournalLimit)
    );
    assert_eq!(
        index.query(buffer.revision(), "b", false, || false),
        Err(IndexError::StaleRevision)
    );
    let rebuilt = build_index(&buffer);
    assert_counts(&rebuilt, &buffer);
}
