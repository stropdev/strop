use super::*;

#[test]
fn filter_and_rank() {
    let items = vec![
        Item {
            badge: None,
            text: "src/main.rs".into(),
            payload: Payload::File("src/main.rs".into()),
        },
        Item {
            badge: None,
            text: "src/render.rs".into(),
            payload: Payload::File("src/render.rs".into()),
        },
        Item {
            badge: None,
            text: "tests/e2e.py".into(),
            payload: Payload::File("tests/e2e.py".into()),
        },
    ];
    let mut p = Picker::new(Kind::Files, items, false);
    for c in "render".chars() {
        p.push_char(c);
    }
    p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
    assert_eq!(p.rows.len(), 1);
    assert_eq!(p.current().unwrap().text, "src/render.rs");
}

#[test]
fn ranking_defaults_to_smart_case_and_sensitive_restores_exact_case() {
    let item = |text: &str| Item {
        badge: None,
        text: text.into(),
        payload: Payload::File(text.into()),
    };
    for (needle, rows) in [("mixedcase", 2), ("MixedCase", 1), ("MIXEDCASE", 0)] {
        let mut picker = Picker::new(
            Kind::Files,
            vec![item("src/MixedCase.rs"), item("src/mixedcase.rs")],
            false,
        );
        picker.paste(needle);
        picker.install_ranking(
            rank::rank(&picker.filter_request(), || false)
                .unwrap()
                .unwrap(),
        );
        assert_eq!(picker.rows.len(), rows, "{needle}");
    }
    let mut picker = Picker::new(
        Kind::Files,
        vec![item("src/MixedCase.rs"), item("src/mixedcase.rs")],
        false,
    );
    picker.paste("MIXEDCASE");
    picker.rank_mode = rank::MatchMode::Fuzzy(query::CaseMode::Sensitive);
    picker.install_ranking(
        rank::rank(&picker.filter_request(), || false)
            .unwrap()
            .unwrap(),
    );
    assert!(picker.rows.is_empty());
    picker.rank_query = Some("MixedCase".into());
    picker.install_ranking(
        rank::rank(&picker.filter_request(), || false)
            .unwrap()
            .unwrap(),
    );
    assert_eq!(picker.rows.len(), 1);
    assert_eq!(picker.current().unwrap().text, "src/MixedCase.rs");

    let mut picker = Picker::new(
        Kind::Files,
        vec![item("src/MixedCase.rs"), item("src/mixedcase.rs")],
        false,
    );
    picker.rank_query = Some("MIXEDCASE".into());
    picker.rank_mode = rank::MatchMode::Exact(
        std::sync::Arc::new(query::ContentExpr::Literal("MIXEDCASE".into())),
        query::CaseMode::Ignore,
    );
    picker.install_ranking(
        rank::rank(&picker.filter_request(), || false)
            .unwrap()
            .unwrap(),
    );
    assert_eq!(picker.rows.len(), 2);
    picker.rank_mode = rank::MatchMode::Exact(
        std::sync::Arc::new(query::ContentExpr::Literal("MIXEDCASE".into())),
        query::CaseMode::Sensitive,
    );
    picker.install_ranking(
        rank::rank(&picker.filter_request(), || false)
            .unwrap()
            .unwrap(),
    );
    assert!(picker.rows.is_empty());
}

#[test]
fn pinned_tail_survives_filtering_below_real_matches() {
    // The remote destinations picker pins "Add a host…": filtering a
    // query that matches nothing real still offers it, and any real
    // match ranks above it.
    let items = vec![
        Item {
            badge: None,
            text: "prtdv-pw-846".into(),
            payload: Payload::RemoteConnect,
        },
        Item {
            badge: None,
            text: "Add a host\u{2026}".into(),
            payload: Payload::RemoteConnect,
        },
    ];
    let mut p = Picker::new(Kind::RemoteHosts, items, false);
    p.pinned_tail = 1;
    for c in "ewosd".chars() {
        p.push_char(c);
    }
    p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
    assert_eq!(p.rows.len(), 1, "the pinned row survives a dead query");
    assert_eq!(p.current().unwrap().text, "Add a host\u{2026}");
    let mut p = Picker::new(
        Kind::RemoteHosts,
        vec![
            Item {
                badge: None,
                text: "prtdv-pw-846".into(),
                payload: Payload::RemoteConnect,
            },
            Item {
                badge: None,
                text: "Add a host\u{2026}".into(),
                payload: Payload::RemoteConnect,
            },
        ],
        false,
    );
    p.pinned_tail = 1;
    for c in "prtdv".chars() {
        p.push_char(c);
    }
    p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
    assert_eq!(p.rows.len(), 2);
    assert_eq!(
        p.current().unwrap().text,
        "prtdv-pw-846",
        "a real match ranks above the pinned row"
    );
}

#[test]
fn shrinking_pinned_tail_keeps_installed_rows_addressable() {
    let item = |text: &str| Item {
        badge: None,
        text: text.into(),
        payload: Payload::ProjectStatus(PathBuf::from(text)),
    };
    let mut p = Picker::new(
        Kind::WorkspaceSymbols,
        vec![Item {
            badge: None,
            text: "src/main.rs".into(),
            payload: Payload::File("src/main.rs".into()),
        }],
        false,
    );
    p.set_pinned(vec![item("/old/one"), item("/old/two")]);
    p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
    p.select(p.rows.len() - 1);

    p.set_pinned(vec![item("/new")]);

    // The catalog changes synchronously; consumers can read the published
    // rows before the next asynchronous ranking completes.
    assert!(p.rows.iter().all(|row| {
        let item = &p.items[row.item];
        item.text == "src/main.rs" || item.text == "/new"
    }));
    assert_eq!(
        p.current().map(|item| item.text.as_str()),
        p.rows
            .get(p.selected)
            .map(|row| p.items[row.item].text.as_str())
    );
}

#[test]
fn selection_wraps() {
    let mut arena: strop_core::id::Arena<strop_core::id::DocumentKind, ()> =
        strop_core::id::Arena::default();
    let items: Vec<Item> = (0..3)
        .map(|i| Item {
            badge: None,
            text: format!("f{i}"),
            payload: Payload::Buffer(arena.try_insert(()).unwrap()),
        })
        .collect();
    let mut p = Picker::new(Kind::Buffers, items, false);
    p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
    p.move_by(-1);
    assert_eq!(p.selected, 2);
    p.move_by(1);
    assert_eq!(p.selected, 0);
}

#[test]
fn ctrl_d_excludes_a_whole_file() {
    let hit = |path: &str| Item {
        badge: None,
        text: path.into(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(PathBuf::from(path)),
            line: 1,
            col: 1,
            match_len: 1,
            line_text: "x".into(),
        },
    };
    let mut p = Picker::search(vec![hit("a.rs"), hit("a.rs"), hit("b.rs")], false, true);
    p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
    p.toggle_file_excluded(); // row 0 -> a.rs
    assert_eq!(p.accepted().count(), 1);
    assert_eq!(p.accepted().next().unwrap().text, "b.rs");
    p.toggle_file_excluded(); // toggle back
    assert_eq!(p.accepted().count(), 3);
}
#[test]
fn regex_query_never_hides_rg_rows() {
    // 0020 §9: "foo|bar" matched three rows upstream — all of them
    // stay visible even though none contains the literal "|"
    let mut p = Picker::new(
        Kind::Search,
        vec![
            Item {
                badge: None,
                text: "a.rs:1 foo".into(),
                payload: Payload::File("a.rs".into()),
            },
            Item {
                badge: None,
                text: "b.rs:2 bar".into(),
                payload: Payload::File("b.rs".into()),
            },
            Item {
                badge: None,
                text: "c.rs:3 baz".into(),
                payload: Payload::File("c.rs".into()),
            },
        ],
        false,
    );
    for c in "foo|bar".chars() {
        p.push_char(c);
    }
    p.install_ranking(rank::rank(&p.filter_request(), || false).unwrap().unwrap());
    assert_eq!(p.rows.len(), 3);
    // and the apply set is exactly those rows
    assert_eq!(p.accepted().count(), 3);
}

fn file(path: &str) -> Item {
    Item {
        badge: None,
        text: path.into(),
        payload: Payload::File(path.into()),
    }
}

fn rerank(picker: &mut Picker) {
    let ranking = rank::rank(&picker.filter_request(), || false)
        .unwrap()
        .unwrap();
    assert!(picker.install_ranking(ranking));
}

/// 0091 B1: the catalog streams in while the query is already typed.
/// Every re-rank moves rows; the bar stays on the top one until the
/// user moves it, so Enter opens the best match — not whichever file
/// happened to be first when the first batch landed.
#[test]
fn the_bar_rests_on_the_top_row_while_a_typed_query_meets_a_streaming_catalog() {
    let mut picker = Picker::new(Kind::Files, vec![], true);
    picker.paste("Makefile");
    let batches = [
        vec![
            file("Documentation/Makefile"),
            file("Documentation/index.rst"),
        ],
        vec![file("Makefile"), file("arch/Makefile")],
        vec![file("tools/Makefile.include"), file("scripts/Makefile")],
    ];
    for batch in batches {
        picker.append(batch);
        rerank(&mut picker);
        assert_eq!(picker.selected, 0, "the bar is the top row at every drain");
        assert_eq!(picker.scroll_top, 0);
    }
    assert_eq!(picker.current().unwrap().text, "Makefile");
}

#[test]
fn a_moved_bar_follows_its_file_until_the_query_changes() {
    let mut picker = Picker::new(
        Kind::Files,
        vec![file("Documentation/Makefile"), file("arch/Makefile")],
        true,
    );
    picker.paste("Makefile");
    rerank(&mut picker);
    picker.move_by(1);
    let chosen = picker.current().unwrap().text.clone();
    picker.append(vec![file("Makefile"), file("a/Makefile")]);
    rerank(&mut picker);
    assert_eq!(picker.current().unwrap().text, chosen);
    assert_ne!(picker.selected, 0);
    // a new query is a new question: the bar returns to the top
    picker.push_char('x');
    picker.clear_results();
    picker.pop_char();
    rerank(&mut picker);
    assert_eq!(picker.selected, 0);
    assert_eq!(picker.current().unwrap().text, "Makefile");
}

#[test]
fn the_resting_row_applies_to_the_empty_query_only() {
    let mut arena: strop_core::id::Arena<strop_core::id::DocumentKind, ()> =
        strop_core::id::Arena::default();
    let items: Vec<Item> = ["now.rs", "before.rs", "older.rs"]
        .into_iter()
        .map(|name| Item {
            badge: None,
            text: name.into(),
            payload: Payload::Buffer(arena.try_insert(()).unwrap()),
        })
        .collect();
    let mut picker = Picker::new(Kind::Buffers, items, false);
    picker.resting_row = 1;
    rerank(&mut picker);
    assert_eq!(picker.current().unwrap().text, "before.rs");
    picker.paste("older");
    picker.clear_results();
    rerank(&mut picker);
    assert_eq!(picker.current().unwrap().text, "older.rs");
    // one row: the resting row clamps instead of pointing past the list
    let mut single = Picker::new(Kind::Buffers, vec![file("only.rs")], false);
    single.resting_row = 1;
    rerank(&mut single);
    assert_eq!(single.current().unwrap().text, "only.rs");
}
