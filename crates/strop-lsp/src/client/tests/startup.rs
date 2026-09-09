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

#[test]
fn startup_notifications_do_not_kill_the_client() {
    // 0.19.0 tore the connection down here: pyright sends
    // window/logMessage on every startup, and the router's default
    // catch-all broke the mainloop on any unregistered notification.
    run(async {
        let (client, rx, mut wire) = Wire::production();
        wire.notify(
            "window/logMessage",
            json!({ "type": 3, "message": "Pyright language server 1.1.411 starting" }),
        )
        .await;
        wire.notify("custom/serverSpecific", json!({ "payload": 1 }))
            .await;
        wire.notify(
            "window/showMessage",
            json!({ "type": 2, "message": "a user-facing notice" }),
        )
        .await;
        let LspEvent::ServerMessage { text, .. } = event(&rx).await else {
            panic!("showMessage reaches the user")
        };
        assert_eq!(text, "a user-facing notice");
        // The connection survived: a real request still round-trips.
        let mut docs = Documents::default();
        let document = docs.insert(());
        let path = Path::new("/workspace/a.rs");
        assert!(client.did_open(
            document,
            BufferRevision::new(0),
            path,
            "rust",
            Rope::from_str("a😀z")
        ));
        let wanted = ask(&client, document, 0);
        initialize(&client);
        let _open = wire.next().await;
        let request = wire.next().await;
        wire.answer(&request, "alive").await;
        let LspEvent::HoverText { context, text } = event(&rx).await else {
            panic!("hover after startup notifications")
        };
        assert_eq!(text, "alive");
        assert_eq!(context.stamp, wanted);
        wire.stop().await;
    });
}
