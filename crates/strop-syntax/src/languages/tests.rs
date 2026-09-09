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
