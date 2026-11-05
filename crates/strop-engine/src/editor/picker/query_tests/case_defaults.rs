use super::*;

#[test]
fn file_queries_are_insensitive_by_default_with_sensitive_override() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["MixedCase.rs", "mixedcase.rs", "MIXEDCASE.rs"] {
        std::fs::write(dir.path().join(name), "").unwrap();
    }
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::Files);
    for needle in ["mixedcase", "MIXEDCASE", "MixedCase"] {
        editor
            .picker
            .as_mut()
            .unwrap()
            .picker
            .input
            .set_text(needle);
        editor.picker_input_changed();
        editor.wait_picker();
        let picker = &editor.picker.as_ref().unwrap().picker;
        assert_eq!(picker.rows.len(), 3, "{needle}: {:?}", picker.items);
    }
    editor
        .picker
        .as_mut()
        .unwrap()
        .picker
        .input
        .set_text("case:sensitive MIXEDCASE");
    editor.picker_input_changed();
    editor.wait_picker();
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert_eq!(picker.rows.len(), 1);
    assert_eq!(picker.current().unwrap().text, "MIXEDCASE.rs");
}
