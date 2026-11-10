use super::*;

#[test]
fn file_queries_are_smart_case_by_default_with_overrides() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["MixedCase.rs", "mixedcase.rs", "MIXEDCASE.rs"] {
        std::fs::write(dir.path().join(name), "").unwrap();
    }
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::Files);
    // lowercase matches any case; one capital makes case count
    for (needle, rows) in [
        ("mixedcase", 3),
        ("MixedCase", 1),
        ("case:ignore MIXEDCASE", 3),
    ] {
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
        assert_eq!(picker.rows.len(), rows, "{needle}: {:?}", picker.items);
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
