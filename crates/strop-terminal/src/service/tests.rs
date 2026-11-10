use super::*;
use strop_core::theme::ThemeId;

fn service() -> (Service, mpsc::Receiver<Request>, UnixDatagram) {
    let (sender, receiver) = mpsc::sync_channel(MAX_COMMANDS);
    let (wake, listening) = UnixDatagram::pair().unwrap();
    wake.set_nonblocking(true).unwrap();
    listening.set_nonblocking(true).unwrap();
    let session = SessionId::new(1);
    (
        Service {
            sender,
            wake,
            budget: Arc::new(Budget::default()),
            mailbox: Arc::new(Mailbox::new(session, |_| {})),
            cancel: None,
            stopped: false,
        },
        receiver,
        listening,
    )
}

#[test]
fn theme_requests_coalesce_and_wake_even_when_child_input_is_full() {
    let (service, receiver, listening) = service();
    service.budget.lifecycle.store(RUNNING, Ordering::Release);
    let permits: Vec<_> = (0..MAX_COMMANDS)
        .map(|_| service.budget.reserve(0).unwrap())
        .collect();
    assert!(matches!(service.focus(true), Err(Error::InputFull)));
    for theme in [
        ThemeId::PaperDark,
        ThemeId::Vellum,
        ThemeId::Paper,
        ThemeId::Slate,
    ] {
        service.set_theme(theme).unwrap();
    }
    let selected = service.budget.theme.load(Ordering::Acquire);
    assert_eq!(ThemeId::ALL[usize::from(selected - 1)], ThemeId::Slate);
    assert_eq!(service.budget.count.load(Ordering::Acquire), MAX_COMMANDS);
    assert_eq!(service.budget.bytes.load(Ordering::Acquire), 0);
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    let mut wake = [0];
    assert_eq!(listening.recv(&mut wake).unwrap(), 1);
    drop(permits);
    assert!(
        service.pending(),
        "appearance remains owned until publication"
    );
    service.budget.theme.store(0, Ordering::Release);
    assert!(!service.pending());
}

#[test]
fn startup_accepts_the_latest_theme_but_closed_services_reject_it() {
    let (mut service, _receiver, _listening) = service();
    service.set_theme(ThemeId::Vellum).unwrap();
    service.set_theme(ThemeId::Carbide).unwrap();
    let selected = service.budget.theme.load(Ordering::Acquire);
    assert_eq!(ThemeId::ALL[usize::from(selected - 1)], ThemeId::Carbide);
    service.stop();
    assert!(matches!(
        service.set_theme(ThemeId::Slate),
        Err(Error::Closed)
    ));
    service.budget.lifecycle.store(FINISHED, Ordering::Release);
    assert!(matches!(
        service.set_theme(ThemeId::Hone),
        Err(Error::Closed)
    ));
}

/// A whole local session through the public service: input reaches the
/// child, its output reaches the frame, and the recorded exit settles
/// the phase only after the output that preceded it.
#[test]
fn a_local_session_echoes_input_and_settles_with_its_exit_code() {
    let (notify, notified) = mpsc::channel();
    let launch = Launch {
        program: "/bin/sh".into(),
        arguments: vec![
            "-c".into(),
            r#"read line; printf 'got:%s' "$line"; exit 3"#.into(),
        ],
        directory: "/".into(),
        environment: Vec::new(),
    };
    let geometry = Geometry {
        columns: 40,
        rows: 5,
        revision: 1,
    };
    let mut service = Service::start(
        SessionId::new(7),
        Target::Local,
        launch,
        geometry,
        0,
        None,
        move |session| {
            let _ = notify.send(session);
        },
    )
    .unwrap();
    service.input(Input::Text("hello\r".into())).unwrap();
    let mut text = String::new();
    let mut acknowledged = 0;
    let phase = loop {
        notified
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the session never settled");
        let Some(update) = service.take_update() else {
            continue;
        };
        acknowledged = acknowledged.max(update.acknowledged_input);
        if let Some(frame) = &update.frame {
            text = frame.projection.to_string();
        }
        if !update.phase.live() {
            break update.phase;
        }
    };
    assert_eq!(
        phase,
        Phase::Exited {
            code: Some(3),
            signal: None
        }
    );
    assert!(text.contains("got:hello"), "{text:?}");
    assert!(acknowledged >= 1, "the input was never acknowledged");
    service.stop();
}
