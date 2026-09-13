use super::*;

#[test]
fn diagnostics_resolve_utf8_columns_in_place() {
    let mut e = editor("a😀z\n");
    let owner = arm(&mut e, 0, RequestKind::Hover, PositionEncoding::Utf8);
    diagnostics(
        &mut e,
        owner,
        PositionEncoding::Utf8,
        vec![Diag {
            line: LineIndex::new(0),
            col: ServerColumn::new(5),
            end_line: LineIndex::new(0),
            end_col: ServerColumn::new(6),
            severity: Severity::Error,
            message: "z".into(),
        }],
    );
    let diagnostics = e.diags_for(e.current()).unwrap();
    assert_eq!(
        (diagnostics[0].col.get(), diagnostics[0].end_col.get()),
        (5, 6)
    );
    assert_eq!(diagnostics[0].severity, Severity::Error);
}

#[test]
fn diagnostics_resolve_utf16_columns_against_the_rope() {
    let mut e = editor("a😀z\n");
    let owner = arm(&mut e, 0, RequestKind::Hover, PositionEncoding::Utf8);
    // UTF-16 units: a=1, emoji=2, z=1. Position 3 begins z;
    // position 4 is the boundary after z.
    diagnostics(
        &mut e,
        owner,
        PositionEncoding::Utf16,
        vec![Diag {
            line: LineIndex::new(0),
            col: ServerColumn::new(3),
            end_line: LineIndex::new(0),
            end_col: ServerColumn::new(4),
            severity: Severity::Warning,
            message: "mid-emoji".into(),
        }],
    );
    let diagnostics = e.diags_for(e.current()).unwrap();
    assert_eq!(
        (diagnostics[0].col.get(), diagnostics[0].end_col.get()),
        (5, 6)
    );
    assert_eq!(diagnostics[0].severity_char(), 'W');
}

#[test]
fn diagnostics_beyond_the_document_clamp_instead_of_panicking() {
    let mut e = editor("single line");
    let owner = arm(&mut e, 0, RequestKind::Hover, PositionEncoding::Utf8);
    diagnostics(
        &mut e,
        owner,
        PositionEncoding::Utf16,
        vec![Diag {
            line: LineIndex::new(9),
            col: ServerColumn::new(2),
            end_line: LineIndex::new(12),
            end_col: ServerColumn::new(4),
            severity: Severity::Information,
            message: "stale server range".into(),
        }],
    );
    let diagnostics = e.diags_for(e.current()).unwrap();
    assert_eq!(diagnostics[0].line.get(), 0);
    assert_eq!(diagnostics[0].end_line.get(), 0);
    // "single line": utf16 == bytes, col 2 stays 2.
    assert_eq!(diagnostics[0].col.get(), 2);
}

#[test]
fn attached_server_diagnostics_survive_full_replay() {
    use crate::editor::trace::{
        drive::{self, Action},
        seed::Seed,
    };
    use strop_trace::replay::{Tape, Tick};
    let mut e = editor("a\n");
    e.buf_mut().path = Some(PathBuf::from("/workspace/origin.rs"));
    e.tape = std::rc::Rc::new(Tape::fixture(|operation, _| match operation {
        "lsp.open" => Ok(serde_json::json!(true)),
        _ => Err(std::io::Error::other("unexpected native observation")),
    }));
    e.tape.seed(&Seed::capture(&e).unwrap()).unwrap();
    e.recorded_action(Action::Start { open: None }, Tick::default())
        .unwrap();
    let ticket = *e.lsp_state.attach.pending.values().next().unwrap();
    let attach = AttachRecord {
        ticket,
        server: Some(ServerId::new(3)),
        language: "rust".into(),
        name: "rust-analyzer".into(),
        root: PathBuf::from("/workspace"),
        target: strop_workspace::Filesystem::Local,
        outcome: AttachDecision::Attached,
        layers: Vec::new(),
    };
    e.recorded_action(Action::Event(AppEvent::LspAttach(attach)), Tick::default())
        .unwrap();
    let event = LspEvent::Diagnostics {
        context: strop_lsp::DiagnosticContext {
            server: ServerId::new(3),
            document: e.current(),
            revision: e.buf().revision(),
            encoding: PositionEncoding::Utf8,
            version: Some(WireVersion::new(1)),
        },
        doc: diag_key(Path::new("/workspace/origin.rs")),
        diags: vec![Diag {
            line: LineIndex::new(0),
            col: ServerColumn::new(0),
            end_line: LineIndex::new(0),
            end_col: ServerColumn::new(1),
            severity: Severity::Error,
            message: "source diagnostic".into(),
        }],
    };
    e.recorded_action(Action::Event(AppEvent::Lsp(event)), Tick::default())
        .unwrap();
    assert_eq!(e.diag_counts(e.current()), (1, 0));
    e.tape.finish().unwrap();
    let replayed = drive::replay(e.tape.fixture_nodes(), |_, _, _, _| {
        Err(std::io::Error::other(
            "frame action without an installed renderer",
        ))
    })
    .unwrap();
    assert_eq!(replayed.diag_counts(replayed.current()), (1, 0));
}
