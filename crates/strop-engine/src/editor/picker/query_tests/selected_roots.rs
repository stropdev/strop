use super::*;

#[test]
fn files_walk_and_accept_in_the_focused_documents_selected_root() {
    let fixture = tempfile::tempdir().unwrap();
    let one = fixture.path().join("one");
    let two = fixture.path().join("two");
    std::fs::create_dir_all(one.join("nested")).unwrap();
    std::fs::create_dir_all(two.join("nested")).unwrap();
    std::fs::write(one.join("nested/one.txt"), "one\n").unwrap();
    let two_file = two.join("nested/two.txt");
    std::fs::write(&two_file, "two\n").unwrap();
    let process_cwd = std::env::current_dir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), one.clone());
    editor
        .select_root(strop_workspace::ResourceLocation::local(two.clone()))
        .unwrap();
    editor.open_fixture(&two_file).unwrap();

    editor.open_picker(Kind::Files);
    assert_eq!(
        editor.picker.as_ref().unwrap().file_root,
        Some(strop_workspace::ResourceLocation::local(two.clone()))
    );
    editor.paste_bracketed("two.txt");
    editor.wait_picker();
    assert_eq!(editor.picker.as_ref().unwrap().picker.rows.len(), 1);
    assert_eq!(editor.picker.as_ref().unwrap().active, None);
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    assert!(editor
        .doc(editor.current())
        .matches_target(&crate::files::FileTarget::Local(two_file)));
    assert_eq!(
        editor.cwd, one,
        "the editor's process-root binding is intact"
    );
    assert_eq!(std::env::current_dir().unwrap(), process_cwd);
}

#[test]
fn selecting_another_root_cannot_retarget_an_open_files_surface() {
    let fixture = tempfile::tempdir().unwrap();
    let first = fixture.path().join("first");
    let second = fixture.path().join("second");
    let third = fixture.path().join("third");
    for root in [&first, &second, &third] {
        std::fs::create_dir_all(root).unwrap();
    }
    std::fs::write(second.join("second.txt"), "second\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), first.clone());
    editor
        .select_root(strop_workspace::ResourceLocation::local(second.clone()))
        .unwrap();
    let source = second.join("second.txt");
    editor.open_fixture(&source).unwrap();
    editor.open_picker(Kind::Files);
    let owner = editor.picker.as_ref().unwrap().active.clone().unwrap();
    assert_eq!(owner.key.cwd, second);

    editor
        .select_root(strop_workspace::ResourceLocation::local(third))
        .unwrap();
    assert_eq!(
        editor.picker.as_ref().unwrap().file_root,
        Some(strop_workspace::ResourceLocation::local(second.clone()))
    );
    assert_eq!(
        editor.picker.as_ref().unwrap().active.as_ref(),
        Some(&owner)
    );
    editor.wait_picker();
    assert!(editor.picker.as_ref().unwrap().picker.items.iter().all(
        |item| matches!(&item.payload, Payload::File(path) if path == &PathBuf::from("second.txt"))
    ));
}

#[test]
fn directory_browsing_uses_its_selected_parent_root_without_chdir() {
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    let nested = project.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("file.txt"), "contents\n").unwrap();
    let process_cwd = std::env::current_dir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), project.clone());
    editor.open_fixture(&nested).unwrap();
    assert!(editor.directory().is_some());
    assert_eq!(
        editor.current_workspace_root(),
        strop_workspace::ResourceLocation::local(project.clone())
    );

    editor.open_picker(Kind::Files);
    assert_eq!(
        editor.picker.as_ref().unwrap().file_root,
        Some(strop_workspace::ResourceLocation::local(project.clone()))
    );
    assert_eq!(editor.cwd, project);
    assert_eq!(std::env::current_dir().unwrap(), process_cwd);
}
