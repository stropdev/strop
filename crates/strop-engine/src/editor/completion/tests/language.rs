use super::*;
use crate::editor::completion::{acceptance, context, model, session};
use std::sync::Arc;
use strop_core::id::DocumentId;
use strop_core::worker::Outcome;
use strop_lsp::completion::{CompletionEntry, CompletionItem, CompletionList};
use strop_lsp::{PositionEncoding, ReplyContext, RequestId, RequestKind, RequestStamp, ServerId};

pub(super) fn prepare(editor: &mut Editor, value: serde_json::Value) -> model::CompletionDelivery {
    let context = Arc::new(
        context::capture(
            editor,
            context::Invocation::ManualLanguage,
            editor.completion.settings_generation,
            None,
        )
        .unwrap(),
    );
    let primary = context.primary().unwrap();
    let source = primary.source;
    let stamp = RequestStamp {
        request: RequestId::new(1),
        server: ServerId::new(7),
        document: source,
        revision: primary.revision,
    };
    editor.lsp_state.bindings.insert(
        source,
        crate::editor::lsp::state::Binding {
            server: stamp.server,
            path: editor.doc(source).buf.path.clone().unwrap(),
            root: editor.cwd.clone(),
            language: "c".into(),
            target: strop_workspace::Filesystem::Local,
            revision: primary.revision,
        },
    );
    let owner =
        context::LanguageContext::capture(&editor.lsp_state.bindings[&source], None).unwrap();
    let item: Arc<CompletionItem> = Arc::new(serde_json::from_value(value).unwrap());
    let plan = acceptance::language(
        &context,
        editor.doc(source).buf.text(),
        PositionEncoding::Utf8,
        &item,
        None,
        false,
        0,
    )
    .unwrap();
    let query = Arc::new(model::CompletionQuery {
        request: editor.worker_ids.allocate().unwrap(),
        context,
    });
    let selection = editor.worker_ids.allocate().unwrap();
    let work = editor.worker_ids.allocate().unwrap();
    let candidate = model::CandidateKey::Language {
        request: stamp,
        ordinal: 0,
    };
    let mut session = session::Session::new(query.clone(), None);
    session.language = session::ProviderState::Settled(Arc::new(model::CompletionDelivery {
        query: query.clone(),
        scope: model::DeliveryScope::Query,
        payload: model::CompletionPayload::Language(Outcome::Success(model::LanguageSuggestions {
            origin: ReplyContext {
                stamp,
                encoding: PositionEncoding::Utf8,
                kind: RequestKind::Completion,
            },
            owner,
            items: CompletionList {
                items: vec![CompletionEntry { ordinal: 0, item }],
                server_incomplete: false,
                client_truncated: false,
                omitted_oversized: 0,
            },
            resolve_required: false,
            prefix: Arc::from("res"),
            suffix: Arc::from(""),
        })),
    }));
    session.selection = session::Selection::Chosen {
        key: candidate.clone(),
        generation: selection,
    };
    session.preparation = Some(work);
    session.acceptance = session::Acceptance::Preparing {
        candidate: candidate.clone(),
        work,
        preview: false,
    };
    editor.completion.session = Some(session);
    editor.tape =
        std::rc::Rc::new(strop_trace::replay::Tape::fixture(
            move |operation, _| match operation {
                "completion.accept.take" => Ok(serde_json::to_value(Some(&plan)).unwrap()),
                "analysis.start" => Ok(serde_json::json!({"Ok": null})),
                _ => Err(std::io::Error::other(format!(
                    "unexpected native observation: {operation}"
                ))),
            },
        ));
    model::CompletionDelivery {
        query,
        scope: model::DeliveryScope::Preparation { selection, work },
        payload: model::CompletionPayload::Acceptance {
            candidate,
            prepared: Ok(()),
        },
    }
}

fn editing() -> (tempfile::TempDir, Editor, DocumentId) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.txt");
    std::fs::write(&path, "// import slot\nres\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), directory.path().to_owned());
    editor.open_fixture(&path).unwrap();
    editor.feed_text("G$a");
    let document = editor.current();
    (directory, editor, document)
}

pub(super) fn import_item() -> serde_json::Value {
    serde_json::json!({"label": "result", "additionalTextEdits": [{"range": {
        "start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
        "newText": "// imported\n"}]})
}

#[test]
fn prepared_completion_does_not_apply_after_language_service_rebinding() {
    for changed in ["server", "root", "path", "language", "namespace"] {
        let (_directory, mut editor, source) = editing();
        let delivery = prepare(&mut editor, import_item());
        let binding = editor.lsp_state.bindings.get_mut(&source).unwrap();
        match changed {
            "server" => binding.server = ServerId::new(8),
            "root" => binding.root.push("another-project"),
            "path" => binding.path.set_file_name("another.txt"),
            "language" => binding.language = "rust".into(),
            "namespace" => {
                binding.target = strop_workspace::Filesystem::Remote(
                    strop_workspace::RemoteEndpoint::parse("ssh://other.example").unwrap(),
                )
            }
            _ => unreachable!(),
        }
        editor.completion_apply_prepared(&delivery);
        assert_eq!(
            editor.doc(source).buf.text().to_string(),
            "// import slot\nres\n",
            "{changed}"
        );
    }
}
