use super::*;

#[test]
fn terminal_context_controls_and_denial_stay_owner_scoped() {
    let mut editor = Editor::new(Buffer::from_text("origin"));
    let document = editor.terminal_fixture(&[("safe", Style::default())], Phase::Running);
    let session = editor.terminal_document(document).unwrap().session;
    editor.feed_text(" a");
    let rows = &editor.picker.as_ref().unwrap().picker.items;
    assert!(rows.iter().any(|item| item.text.contains("Refresh pinned")));
    assert!(rows
        .iter()
        .any(|item| item.text.contains("Select terminal session")));
    assert!(rows
        .iter()
        .any(|item| item.text.contains("Stop terminal session")));
    editor.close_picker();

    editor.apply_terminal_update(Update {
        session,
        phase: Phase::Running,
        frame: None,
        effects: vec![strop_terminal::model::Effect::HostControlDenied],
        acknowledged_input: 0,
        warning: None,
    });
    editor.feed_text("l");
    let record = editor
        .feedback()
        .iter()
        .find(|record| record.summary().contains("host control denied"))
        .unwrap();
    assert!(record.unresolved());
    assert!(matches!(
        record.owner(),
        crate::editor::feedback::FeedbackOwner::Terminal { session: owner, .. } if *owner == session
    ));
    let id = record.id();
    let dismiss = record
        .actions
        .iter()
        .position(|action| matches!(action, crate::editor::feedback::TypedAction::Dismiss))
        .unwrap();
    editor.invoke_feedback_action(id, dismiss);
    assert!(editor.terminal_phase(document).unwrap().live());

    editor.apply_terminal_update(Update {
        session,
        phase: Phase::Exited {
            code: Some(0),
            signal: None,
        },
        frame: None,
        effects: Vec::new(),
        acknowledged_input: 0,
        warning: None,
    });
    editor.context_actions();
    let rows = &editor.picker.as_ref().unwrap().picker.items;
    assert!(rows.iter().any(|item| item.text.contains("New terminal")));
    assert!(!rows
        .iter()
        .any(|item| item.text.contains("Stop terminal session")));
}
