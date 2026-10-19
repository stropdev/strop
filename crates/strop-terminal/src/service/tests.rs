use super::*;
use strop_core::theme::ThemeId;

fn service() -> (Service, mpsc::Receiver<Request>, UnixDatagram) {
    let (sender, receiver) = mpsc::sync_channel(MAX_COMMANDS);
    let (wake, listening) = UnixDatagram::pair().unwrap();
    wake.set_nonblocking(true).unwrap();
    listening.set_nonblocking(true).unwrap();
    let session = SessionId::from_request(strop_core::worker::WorkerId::new(1));
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
    for theme in ThemeId::ALL {
        service.set_theme(theme).unwrap();
    }
    let selected = service.budget.theme.load(Ordering::Acquire);
    assert_eq!(ThemeId::ALL[usize::from(selected - 1)], ThemeId::StropInk);
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
    service.set_theme(ThemeId::Light).unwrap();
    service.set_theme(ThemeId::CatppuccinMocha).unwrap();
    let selected = service.budget.theme.load(Ordering::Acquire);
    assert_eq!(
        ThemeId::ALL[usize::from(selected - 1)],
        ThemeId::CatppuccinMocha
    );
    service.stop();
    assert!(matches!(
        service.set_theme(ThemeId::Nord),
        Err(Error::Closed)
    ));
    service.budget.lifecycle.store(FINISHED, Ordering::Release);
    assert!(matches!(
        service.set_theme(ThemeId::Dark),
        Err(Error::Closed)
    ));
}
