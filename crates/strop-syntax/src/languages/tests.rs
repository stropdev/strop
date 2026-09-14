use super::*;

#[test]
fn exact_filenames_beat_extension_and_shebang() {
    for name in [".bashrc", ".bash_profile", ".profile", "PKGBUILD"] {
        let spec = detect(std::path::Path::new(&format!("/home/tarek/{name}")), None)
            .unwrap_or_else(|| panic!("{name} unresolved"));
        assert_eq!(spec.name, "bash", "{name}");
    }
    // exact basename wins even against a contradictory shebang
    let spec = detect(
        std::path::Path::new("/home/tarek/.bashrc"),
        Some("#!/usr/bin/env fish\n"),
    )
    .unwrap();
    assert_eq!(spec.name, "bash");
    // "PKGBUILD.fish" is not an exact basename — extension rules
    assert_eq!(
        detect(std::path::Path::new("PKGBUILD.fish"), None)
            .unwrap()
            .name,
        "fish"
    );
}

#[test]
fn shebang_resolves_when_extension_unknown_or_absent() {
    for (line, lang) in [
        ("#!/bin/bash\n", "bash"),
        ("#!/bin/bash -euo pipefail\n", "bash"),
        ("#!/usr/bin/env bash\n", "bash"),
        ("#!/usr/bin/env -S bash --norc\n", "bash"),
        ("#!/bin/sh\n", "bash"),
        ("#!/usr/bin/env zsh\n", "bash"),
        ("#!/usr/bin/fish\n", "fish"),
        ("#!/usr/bin/env fish\n", "fish"),
    ] {
        let spec = detect(std::path::Path::new("some-script"), Some(line))
            .unwrap_or_else(|| panic!("unresolved shebang {line:?}"));
        assert_eq!(spec.name, lang, "{line:?}");
    }
    // unknown extension still defers to the shebang
    assert_eq!(
        detect(std::path::Path::new("weird.tool"), Some("#!/bin/bash\n"))
            .unwrap()
            .name,
        "bash"
    );
    // no shebang, no extension, no dice
    assert!(detect(std::path::Path::new("README"), Some("# comment\n")).is_none());
    assert!(detect(std::path::Path::new("run.pl"), Some("#!/usr/bin/perl\n")).is_none());
    assert!(detect(std::path::Path::new("empty"), Some("")).is_none());
}

#[test]
fn known_extension_beats_shebang() {
    let spec = detect(std::path::Path::new("x.fish"), Some("#!/bin/bash\n")).unwrap();
    assert_eq!(spec.name, "fish");
}

#[test]
fn core_catalog_membership_matches_syntax_catalog() {
    // 0063 §3: one language catalog policy. Every syntax entry whose
    // language name the core catalog knows must agree on extension
    // membership, so queries and highlighting never disagree.
    for entry in LANGUAGES {
        let Some(core) = strop_core::languages::extensions_for_language(entry.name) else {
            continue;
        };
        for extension in entry.extensions {
            let resolved = strop_core::languages::language_for_extension_name(extension);
            let expected = resolved.is_some_and(|name| name == entry.name);
            assert!(
                expected || core.contains(extension),
                "{extension} is a {} syntax extension but resolves to {resolved:?} in the core catalog",
                entry.name,
            );
        }
        for extension in core {
            // Core may fold a dialect into its family (tsx into typescript)
            // while syntax gives the dialect its own entry; both are fine —
            // what must hold is that highlighting exists for the extension.
            assert!(
                for_extension(extension).is_some(),
                "{extension} is a core {} extension but syntax highlighting does not claim it",
                entry.name,
            );
        }
    }
}
