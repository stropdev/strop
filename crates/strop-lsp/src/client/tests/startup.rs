use super::*;

#[test]
fn preinit_close_cancels_queued_requests_with_terminal_notes() {
    run(async {
        let (client, rx, wire) = Wire::new();
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("x")
        ));
        let wanted = ask(&client, document, 0);
        client.did_close(document, path);
        let LspEvent::Note { context, .. } = event(&rx).await else {
            panic!("note")
        };
        assert_eq!(context.stamp, wanted);
        initialize(&client);
        // Nothing was ever framed: the open was cancelled pre-init.
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        wire.stop().await;
    });
}
