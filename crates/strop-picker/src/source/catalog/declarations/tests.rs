use super::*;
use crate::{ProjectCatalog, WarmHint};
use std::fs;

#[test]
fn markerless_members_hints_unlisted_and_ignored_paths_share_one_catalog() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for path in [
        ".strop",
        "team/tool",
        "two/tool",
        "ignored/member",
        "loose",
        "cold",
        ".git/member",
    ] {
        fs::create_dir_all(root.join(path)).unwrap();
    }
    fs::write(root.join("cold/go.mod"), "").unwrap();
    fs::write(root.join("two/tool/go.mod"), "").unwrap();
    fs::write(root.join(".ignore"), "ignored\n").unwrap();
    fs::write(root.join(".strop/workspace.toml"), "version=1\n[[project]]\npath='team/tool'\nlanguages=['rust','unknown','go','rust']\n[[project]]\npath='two/tool'\nlanguages=[]\n[[project]]\npath='cold'\n[[project]]\npath='ignored/member'\nlanguages=['go']\n[[project]]\npath='.git/member'\nlanguages=['rust']").unwrap();
    let records = read_workspace_declarations(root).unwrap();
    let member = &records[0];
    assert_eq!(member.path(), Path::new("team/tool"));
    assert_eq!(member.languages().unwrap(), &["go", "rust", "unknown"]);
    assert!(member.open_directory(root).is_ok());
    assert_eq!(records[1].languages(), Some(&[][..]));
    assert_eq!(records[2].languages(), None);
    assert!(records[3].open_directory(root).is_err());
    assert!(records[4].open_directory(root).is_err());

    let (catalog, _) = ProjectCatalog::scan(root, &|| false, &records);
    assert!(!catalog.truncated);
    assert!(catalog.project_for(Path::new("loose/note")).is_some());
    assert_eq!(
        catalog
            .project_for(Path::new("team/tool/note"))
            .unwrap()
            .root,
        Path::new("team/tool")
    );
    let (warm, _) = catalog.warm_projects();
    assert!(warm
        .iter()
        .any(|w| w.root == root.join("team/tool")
            && w.hint == WarmHint::Language("unknown".to_owned())));
    assert!(warm
        .iter()
        .any(|w| w.root == root.join("cold") && w.hint == WarmHint::Language("go".to_owned())));
    assert!(!warm
        .iter()
        .any(|w| ["two/tool", "ignored/member", ".git/member"]
            .iter()
            .any(|p| w.root == root.join(p))));
    fs::rename(root.join("team/tool"), root.join("prior")).unwrap();
    fs::create_dir(root.join("team/tool")).unwrap();
    let after = ProjectCatalog::scan(root, &|| false, &records).0;
    assert!(!after.projects().any(|p| p.root == Path::new("team/tool")));
    assert!(member.open_directory(root).is_err());
}

#[test]
fn replay_preserves_native_metadata_not_a_directory_capability() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".strop")).unwrap();
    fs::create_dir(root.join("café")).unwrap();
    fs::write(
        root.join(".strop/workspace.toml"),
        "version=1\n[[project]]\npath='café'\nlanguages=[]",
    )
    .unwrap();
    let entries = read_workspace_declarations(root).unwrap();
    let value = serde_json::to_value(&entries[0]).unwrap();
    assert_eq!(value["root"]["encoding"], "unix-bytes");
    assert!(value.get("witness").is_none());
    let replay: DeclaredBoundary = serde_json::from_value(value).unwrap();
    assert_eq!(replay.path(), entries[0].path());
    assert_eq!(replay.languages(), entries[0].languages());
    assert!(entries[0].open_directory(root).is_ok());
    assert!(replay.open_directory(root).is_err());
}

#[test]
fn decoded_budget_failure_is_an_actionable_reader_error() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir(root.join(".strop")).unwrap();
    fs::create_dir(root.join("member")).unwrap();
    let ids: Vec<_> = (0..3000).map(|n| format!("'lang{n}'")).collect();
    let manifest = format!(
        "version=1\n[[project]]\npath='member'\nlanguages=[{}]",
        ids.join(",")
    );
    assert!(manifest.len() < 64 * 1024);
    fs::write(root.join(".strop/workspace.toml"), manifest).unwrap();
    assert!(read_workspace_declarations(root)
        .unwrap_err()
        .to_string()
        .contains("decoded declarations"));
}
