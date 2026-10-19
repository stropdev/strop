use super::*;
use std::time::{Duration, Instant};
use strop_core::{frontend_input::Input, theme::ThemeId, Buffer};
use strop_terminal::model::{Color, Palette, Rgb};

fn wait_for(editor: &mut Editor, condition: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition(editor) {
        let session = editor
            .terminals
            .rx
            .as_ref()
            .unwrap()
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| panic!("terminal update missing: {error}: {}", editor.message));
        editor.handle_terminal_update(session);
    }
}

#[test]
fn idle_sessions_retheme_during_startup_and_frozen_inspection() {
    // Both capture policies use the same service/publication path. No global
    // theme selection is changed by this test or its parallel neighbors.
    for capture in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
        editor.set_terminal_capture(capture);
        let command = r"stty -echo; printf '\033[31mI\033[38;2;1;2;3mT\033[mD\033]4;1;rgb:12/34/56\007\033]10;rgb:23/45/67\007\033]11;rgb:34/56/78\007'; read line; printf '\033]104\007\033]110\007\033]111\007'; read line";
        editor.launch_terminal(command, false);
        let first = editor.current();
        editor.launch_terminal(command, false);
        let second = editor.current();
        // Queue while the services can still be starting; both must receive it.
        editor
            .set_terminal_themes(ThemeId::CatppuccinMocha)
            .unwrap();
        let mocha = Palette::for_theme(ThemeId::CatppuccinMocha);
        wait_for(&mut editor, |editor| {
            [first, second].iter().all(|document| {
                editor.terminal_frame(*document, true).is_some_and(|frame| {
                    frame.projection.to_string().starts_with("ITD")
                        && frame.palette.colors[3] == mocha.colors[3]
                        && frame.palette.background
                            == Rgb {
                                red: 0x34,
                                green: 0x56,
                                blue: 0x78,
                            }
                })
            })
        });

        editor.switch_to(first);
        editor.view_mut().terminal_input = false;
        editor.set_head(1);
        let shown = editor.terminal_frame(first, false).unwrap().clone();
        let before: Vec<_> = [first, second]
            .iter()
            .map(|document| editor.terminal_frame(*document, true).unwrap().clone())
            .collect();
        editor.set_terminal_themes(ThemeId::Nord).unwrap();
        let nord = Palette::for_theme(ThemeId::Nord);
        wait_for(&mut editor, |editor| {
            [first, second].iter().all(|document| {
                editor
                    .terminal_frame(*document, true)
                    .is_some_and(|frame| frame.palette.colors[3] == nord.colors[3])
            })
        });
        for (document, old) in [first, second].iter().zip(before) {
            let frame = editor.terminal_frame(*document, true).unwrap();
            assert!(frame.revision > old.revision);
            assert_eq!(frame.projection, old.projection);
            assert_eq!(frame.cursor, old.cursor);
            assert_eq!(frame.palette.foreground, old.palette.foreground);
            assert_eq!(frame.palette.background, old.palette.background);
            assert_eq!(frame.palette.colors[1], old.palette.colors[1]);
        }
        assert!(Arc::ptr_eq(
            &shown,
            editor.terminal_frame(first, false).unwrap()
        ));
        assert_eq!(editor.head(), 1);
        let (style, palette) = editor.terminal_style_at(first, 1).unwrap();
        assert_eq!(
            style.foreground,
            Color::Rgb(Rgb {
                red: 1,
                green: 2,
                blue: 3
            })
        );
        assert_eq!(palette.colors[3], nord.colors[3]);
        assert_eq!(
            palette.colors[1],
            Rgb {
                red: 0x12,
                green: 0x34,
                blue: 0x56
            }
        );
        assert_eq!(
            palette.foreground,
            Rgb {
                red: 0x23,
                green: 0x45,
                blue: 0x67
            }
        );
        assert_eq!(
            palette.background,
            Rgb {
                red: 0x34,
                green: 0x56,
                blue: 0x78
            }
        );
        assert_eq!(editor.terminal_capture_enabled(), capture);
        assert_eq!(editor.private_terminal_view(), !capture);

        // Only the explicit child command produces subsequent output. Resets
        // resolve to Nord in both live and pinned views, with text still pinned.
        editor.feed_terminal(Input::Text("reset\n".into()));
        wait_for(&mut editor, |editor| {
            editor
                .terminal_frame(first, true)
                .is_some_and(|frame| *frame.palette == nord)
        });
        assert_eq!(editor.terminal_style_at(first, 1).unwrap().0, style);
        assert_eq!(*editor.terminal_style_at(first, 1).unwrap().1, nord);
        assert!(Arc::ptr_eq(
            &shown,
            editor.terminal_frame(first, false).unwrap()
        ));
    }
}

#[test]
fn appearance_publication_does_not_consume_the_child_input_sequence() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    editor.launch_terminal(
        "stty -echo; printf READY; read line; printf RECEIVED; read line; printf SECOND; read line",
        false,
    );
    let session = editor.terminal_document(editor.current()).unwrap().session;
    let service = editor
        .terminals
        .entries
        .get_mut(&session)
        .unwrap()
        .service
        .take()
        .unwrap();
    let notifications = editor.terminals.rx.take().unwrap();
    let next = |phase: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            notifications
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| panic!("{phase}: {error}"));
            if let Some(update) = service.take_update() {
                return update;
            }
        }
    };
    loop {
        let update = next("waiting for initial child output");
        if update
            .frame
            .as_ref()
            .is_some_and(|frame| frame.projection.to_string().starts_with("READY"))
        {
            assert_eq!(update.acknowledged_input, 0);
            break;
        }
    }
    for theme in [ThemeId::Light, ThemeId::CatppuccinMocha, ThemeId::Nord] {
        service.set_theme(theme).unwrap();
        let selected = Palette::for_theme(theme);
        loop {
            let update = next(theme.name());
            assert_eq!(update.acknowledged_input, 0);
            if update
                .frame
                .as_ref()
                .is_some_and(|frame| *frame.palette == selected)
            {
                break;
            }
        }
    }
    service.input(Input::Text("one\n".into()), false).unwrap();
    let mut received = false;
    let mut acknowledged = false;
    while !received || !acknowledged {
        let update = next("waiting for input acknowledgement and child reply");
        received |= update
            .frame
            .as_ref()
            .is_some_and(|frame| frame.projection.to_string().contains("RECEIVED"));
        acknowledged |= update.acknowledged_input == 1;
        assert!(update.acknowledged_input <= 1);
    }
    // No-op focus produces no wire input; it must release its permit without
    // waiting for a child that is idle. An ordered resize does use a local
    // sequence, but must not shift subsequent worker-input acknowledgments.
    service.focus(false).unwrap();
    assert_eq!(next("no-op focus").acknowledged_input, 1);
    assert!(!service.pending());
    service
        .resize(strop_terminal::model::Geometry {
            columns: 81,
            rows: 24,
            revision: 2,
        })
        .unwrap();
    while next("ordered resize").acknowledged_input < 2 {}
    service.set_theme(ThemeId::StropInk).unwrap();
    let ink = Palette::for_theme(ThemeId::StropInk);
    loop {
        let update = next("appearance after resize");
        assert_eq!(update.acknowledged_input, 2);
        if update
            .frame
            .as_ref()
            .is_some_and(|frame| *frame.palette == ink)
        {
            break;
        }
    }
    service.input(Input::Text("two\n".into()), false).unwrap();
    let mut received = false;
    let mut acknowledged = false;
    while !received || !acknowledged {
        let update = next("second input after resize");
        received |= update
            .frame
            .as_ref()
            .is_some_and(|frame| frame.projection.to_string().contains("SECOND"));
        acknowledged |= update.acknowledged_input == 3;
        assert!(update.acknowledged_input <= 3);
    }
}
