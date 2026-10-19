use super::*;

fn luminance(color: Rgb) -> f64 {
    fn linear(component: u8) -> f64 {
        let value = f64::from(component) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

fn contrast(first: Rgb, second: Rgb) -> f64 {
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

#[test]
fn main_text_remains_readable_on_every_editor_background() {
    for id in ThemeId::ALL {
        let palette = id.palette();
        for (role, background) in [
            ("base", palette.base),
            ("surface", palette.surface),
            ("statusline", palette.statusline_bg),
            ("cursor row", palette.cursor_row_bg),
            ("inactive row", palette.inactive_bg),
            ("selection", palette.select_bg),
            ("matching pair", palette.pair_bg),
            ("preview", palette.preview_bg),
            ("flash", palette.flash_bg),
            ("diff addition", palette.diff_add_bg),
            ("diff deletion", palette.diff_del_bg),
            ("diff strong addition", palette.diff_add_strong_bg),
            ("diff strong deletion", palette.diff_del_strong_bg),
        ] {
            let ratio = contrast(palette.text, background);
            assert!(ratio >= 4.5, "{} text on {role}: {ratio:.3}:1", id.name());
        }
    }
}

#[test]
fn syntax_and_comments_are_legible_on_editor_and_popup_surfaces() {
    // ANSI entries are deliberately excluded: terminal children choose the
    // pairings and may replace the palette or specify arbitrary truecolor.
    for id in ThemeId::ALL {
        let palette = id.palette();
        for (role, foreground) in [
            ("comment", palette.muted),
            ("secondary", palette.secondary),
            ("keyword", palette.class_keyword),
            ("function", palette.class_function),
            ("type", palette.class_type),
            ("string", palette.class_string),
            ("number", palette.class_number),
            ("operator", palette.class_operator),
            ("punctuation", palette.class_punctuation),
            ("attribute", palette.class_attribute),
        ] {
            for (surface, background) in [("base", palette.base), ("surface", palette.surface)] {
                let ratio = contrast(foreground, background);
                assert!(
                    ratio >= 3.0,
                    "{} {role} on {surface}: {ratio:.3}:1",
                    id.name()
                );
            }
        }
    }
}

#[test]
fn selection_and_change_overlays_have_distinct_visual_strength() {
    for id in ThemeId::ALL {
        let palette = id.palette();
        let selection = contrast(palette.select_bg, palette.base);
        assert!(selection >= 1.15, "{} selection disappears", id.name());
        assert!(
            contrast(palette.pair_bg, palette.base) > selection,
            "{} matching pair must stand above selection",
            id.name()
        );
        assert_ne!(palette.diff_add_bg, palette.diff_del_bg, "{}", id.name());
        for (role, weak, strong) in [
            ("addition", palette.diff_add_bg, palette.diff_add_strong_bg),
            ("deletion", palette.diff_del_bg, palette.diff_del_strong_bg),
            ("preview", palette.preview_bg, palette.flash_bg),
        ] {
            assert!(
                contrast(strong, palette.base) > contrast(weak, palette.base),
                "{} {role} loses emphasis",
                id.name()
            );
        }
    }
}

#[test]
fn ansi256_preserves_indexed_terminal_color_contract() {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    for id in ThemeId::ALL {
        let palette = id.palette();
        let colors = palette.ansi256();
        assert_eq!(&colors[..16], &palette.ansi16, "{} ANSI-16", id.name());
        for (red, r) in LEVELS.into_iter().enumerate() {
            for (green, g) in LEVELS.into_iter().enumerate() {
                for (blue, b) in LEVELS.into_iter().enumerate() {
                    let index = 16 + red * 36 + green * 6 + blue;
                    assert_eq!(colors[index], rgb(r, g, b), "{} index {index}", id.name());
                }
            }
        }
        for index in 0..24 {
            let level = 8 + 10 * index as u8;
            assert_eq!(colors[232 + index], rgb(level, level, level));
        }
    }
}

#[test]
fn canonical_names_match_the_serialized_consumer_contract() {
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Selection {
        theme: ThemeId,
    }

    let expected = [
        "dark",
        "light",
        "catppuccin-mocha",
        "catppuccin-latte",
        "nord",
        "strop-ink",
    ];
    assert_eq!(ThemeId::ALL.len(), expected.len());
    for (id, name) in ThemeId::ALL.into_iter().zip(expected) {
        assert_eq!(id.name(), name);
        assert_eq!(ThemeId::from_name(name), Some(id));
        assert_eq!(
            ThemeId::from_name(&format!(" {} ", name.to_uppercase())),
            Some(id)
        );
        let selection = Selection { theme: id };
        let document = format!(r#"{{"theme":"{name}"}}"#);
        assert_eq!(serde_json::to_string(&selection).unwrap(), document);
        assert_eq!(
            serde_json::from_str::<Selection>(&document).unwrap(),
            selection
        );
    }
    for invalid in ["", "solarized", "mocha", "catppuccin_mocha", "stropink"] {
        assert_eq!(ThemeId::from_name(invalid), None);
        let document = format!(r#"{{"theme":"{invalid}"}}"#);
        assert!(serde_json::from_str::<Selection>(&document).is_err());
    }
    // CLI lookup is forgiving; persisted serde names remain canonical.
    assert!(serde_json::from_str::<ThemeId>(r#""Catppuccin-Mocha""#).is_err());
}

#[test]
fn live_selector_switches_all_palettes_in_an_isolated_process() {
    const CHILD: &str = "STROP_THEME_SELECTOR_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        assert_eq!(current_id(), ThemeId::Dark);
        assert_eq!(current(), ThemeId::Dark.palette());
        for id in ThemeId::ALL.into_iter().rev() {
            set_current(id);
            assert_eq!(current_id(), id);
            assert_eq!(current(), id.palette());
            // Reading another palette never changes the live selection.
            for other in ThemeId::ALL {
                let _ = other.palette();
                assert_eq!(current_id(), id);
            }
        }
        return;
    }

    // Never mutate the shared selector in this parallel test process.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "theme::tests::live_selector_switches_all_palettes_in_an_isolated_process",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated selector test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}
