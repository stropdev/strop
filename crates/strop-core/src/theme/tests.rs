//! The 0088 §3 contrast rules, asserted for every palette: these are the
//! design contract's numbers, not a percentage.
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

fn assert_at_least(id: ThemeId, role: &str, ink: Rgb, background: Rgb, floor: f64) {
    let ratio = contrast(ink, background);
    assert!(
        ratio + 1e-9 >= floor,
        "{} {role}: {ratio:.3}:1 < {floor}:1",
        id.name()
    );
}

fn syntax_inks(syn: &Syntax) -> [(&'static str, Rgb); 16] {
    [
        ("keyword", syn.keyword),
        ("function", syn.function),
        ("type", syn.r#type),
        ("string", syn.string),
        ("number", syn.number),
        ("constant", syn.constant),
        ("attribute", syn.attribute),
        ("operator", syn.operator),
        ("punctuation", syn.punctuation),
        ("comment", syn.comment),
        ("variable", syn.variable),
        ("property", syn.property),
        ("namespace", syn.namespace),
        ("heading", syn.heading),
        ("link", syn.link),
        ("code", syn.code),
    ]
}

/// text ≥ 10, secondary ≥ 7, muted/comment/every syn.* and every
/// diagnostic ink ≥ 4.5 — on the base and on the surface.
#[test]
fn inks_clear_their_floors_on_base_and_surface() {
    for id in ThemeId::ALL {
        let palette = id.palette();
        for background in [palette.bg.base, palette.bg.surface] {
            assert_at_least(id, "text", palette.fg.text, background, 10.0);
            assert_at_least(id, "secondary", palette.fg.secondary, background, 7.0);
            assert_at_least(id, "muted", palette.fg.muted, background, 4.5);
            for (role, ink) in syntax_inks(&palette.syn) {
                assert_at_least(id, role, ink, background, 4.5);
            }
            for (role, ink) in [
                ("diag.error", palette.diag.error),
                ("diag.warning", palette.diag.warning),
                ("diag.info", palette.diag.info),
                ("diag.hint", palette.diag.hint),
                ("diff.add", palette.diff.add),
                ("diff.del", palette.diff.del),
                ("mode.insert", palette.mode.insert),
                ("mode.visual", palette.mode.visual),
                ("mode.replace", palette.mode.replace),
                ("mode.terminal", palette.mode.terminal),
                ("accent", palette.accent),
            ] {
                assert_at_least(id, role, ink, background, 4.5);
            }
        }
        assert_at_least(id, "on_accent", palette.fg.on_accent, palette.accent, 4.5);
        // Keycaps are accent bold on the raised plane (0088 §2 rule 6).
        assert_at_least(id, "keycap", palette.accent, palette.bg.raised, 4.5);
    }
}

/// Three planes, one ladder: each step 1.07–1.30:1, the raised plane
/// ≥ 1.18:1 above the base, the cursor line a visible hair above it.
#[test]
fn the_tonal_ladder_has_visible_steps() {
    for id in ThemeId::ALL {
        let bg = id.palette().bg;
        for (step, lower, upper) in [
            ("base→surface", bg.base, bg.surface),
            ("surface→raised", bg.surface, bg.raised),
        ] {
            let ratio = contrast(lower, upper);
            assert!(
                (1.07 - 1e-9..=1.30 + 1e-9).contains(&ratio),
                "{} {step}: {ratio:.3}:1 outside 1.07–1.30",
                id.name()
            );
        }
        assert_at_least(id, "raised over base", bg.raised, bg.base, 1.18);
        assert_at_least(id, "cursorline over base", bg.cursorline, bg.base, 1.04);
    }
}

/// Visual mode must be visible: selection ≥ 1.45:1 on light palettes,
/// ≥ 1.55:1 on dark ones. Slate is held to 1.49 — the documented floor
/// for a low-chroma dark palette.
#[test]
fn selection_is_visible_and_overlays_keep_their_order() {
    for id in ThemeId::ALL {
        let bg = id.palette().bg;
        let floor = match id {
            ThemeId::Slate => 1.49,
            _ if id.is_light() => 1.45,
            _ => 1.55,
        };
        assert_at_least(id, "selection", bg.selection, bg.base, floor);
        assert_ne!(bg.diff_add, bg.diff_del, "{}", id.name());
        for (role, weak, strong) in [
            ("addition", bg.diff_add, bg.diff_add_strong),
            ("deletion", bg.diff_del, bg.diff_del_strong),
            ("preview", bg.preview, bg.flash),
        ] {
            assert!(
                contrast(strong, bg.base) > contrast(weak, bg.base),
                "{} {role} loses emphasis",
                id.name()
            );
        }
    }
}

/// Comments and punctuation are never the same ink unless italic carries
/// the distinction (rule 8): the light palettes rely on the italic, so
/// here the dark palettes are held to the 1.2:1 luminance gap.
#[test]
fn dark_palettes_separate_comments_from_punctuation() {
    for id in ThemeId::ALL.into_iter().filter(|id| !id.is_light()) {
        let syn = id.palette().syn;
        assert_at_least(
            id,
            "punctuation vs comment",
            syn.punctuation,
            syn.comment,
            1.2,
        );
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
fn registry_names_round_trip_and_the_default_is_hone() {
    assert_eq!(ThemeId::default(), ThemeId::Hone);
    for id in ThemeId::ALL {
        assert_eq!(ThemeId::from_name(id.name()), Some(id));
        assert_eq!(ThemeId::from_name(&id.name().to_uppercase()), Some(id));
        assert_eq!(
            serde_json::from_str::<ThemeId>(&format!("{:?}", id.name())).ok(),
            Some(id)
        );
    }
    assert_eq!(ThemeId::from_name("nord"), None);
    assert_eq!(ThemeId::from_name("catppuccin-mocha"), None);
    assert_eq!(ThemeId::from_name("dark"), None);
    assert_eq!(
        ThemeId::choices(),
        "hone, slate, carbide, vellum, paper, paper-dark"
    );
}
