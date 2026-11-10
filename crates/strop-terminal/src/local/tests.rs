use super::*;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn slow_greeter() -> Launch {
    Launch {
        program: "/bin/sh".into(),
        arguments: vec!["-c".into(), "sleep 0.2; printf ready; cat".into()],
        directory: PathBuf::from("/"),
        environment: Vec::<(OsString, OsString)>::new(),
    }
}

/// Feed `typed\n` right after spawn, then collect output until the
/// child's `cat` has it back. Returns everything the terminal showed.
fn typeahead(hold: bool) -> String {
    let (wake, listen) = UnixDatagram::pair().unwrap();
    listen.set_nonblocking(true).unwrap();
    let geometry = Geometry {
        columns: 80,
        rows: 24,
        revision: 1,
    };
    let mut pty = LocalPty::spawn(&slow_greeter(), geometry, hold, wake).unwrap();
    let mark = pty.feed(b"typed\n").unwrap();
    assert_eq!(
        pty.delivered() >= mark,
        !hold,
        "held input was written early"
    );
    let mut shown = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    // `cat` echoes the line after the terminal's own echo: two copies.
    while String::from_utf8_lossy(&shown).matches("typed").count() < 2 {
        assert!(
            Instant::now() < deadline,
            "typeahead never reached the child: {:?}",
            String::from_utf8_lossy(&shown)
        );
        pty.flush().unwrap();
        match pty.try_output() {
            Ok(Output::Bytes(bytes)) => shown.extend_from_slice(&bytes),
            Ok(Output::Closed) => panic!("child closed: {shown:?}"),
            Err(_) => {
                pty.wait(&listen).unwrap();
                while listen.recv(&mut [0; 8]).is_ok() {}
            }
        }
    }
    assert!(pty.delivered() >= mark);
    pty.terminate();
    String::from_utf8_lossy(&shown).into_owned()
}

#[test]
fn held_typeahead_is_delivered_after_the_first_output() {
    let shown = typeahead(true);
    let ready = shown.find("ready").expect("greeting shown");
    let typed = shown.find("typed").expect("typeahead shown");
    assert!(
        ready < typed,
        "held keys reached the terminal before the child spoke: {shown:?}"
    );
}

#[test]
fn unheld_typeahead_is_written_at_once() {
    // The contrast that makes the held case meaningful: without the hold,
    // the terminal echoes the keys before the child has spoken.
    let shown = typeahead(false);
    let ready = shown.find("ready").expect("greeting shown");
    let typed = shown.find("typed").expect("typeahead shown");
    assert!(typed < ready, "{shown:?}");
}
