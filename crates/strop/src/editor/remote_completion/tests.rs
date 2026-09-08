//! Completion reducer tests. The worker is suppressed with a fixture
//! tape (hermetic: no real `~/.ssh` read, no client, no network) and
//! answered through the production handler — exactly how the TUI's
//! forwarded deliveries arrive.

use super::*;
use strop_core::Buffer;

fn editor() -> Editor {
    Editor::new(Buffer::from_text("x\n"))
}

fn suppress(e: &mut Editor) {
    e.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
        Err(std::io::Error::other(
            "unexpected remote-completion observation",
        ))
    }));
}

fn cand(uri: &str, directory: bool) -> RemoteCandidate {
    RemoteCandidate {
        uri: uri.to_owned(),
        directory,
    }
}

fn hosts_result(uris: &[&str]) -> RemoteCompletionResult {
    RemoteCompletionResult::Candidates {
        items: uris.iter().map(|uri| cand(uri, false)).collect(),
        source: CandidateSource::Config,
        notes: Vec::new(),
        listed_directory: None,
    }
}

fn deliver(e: &mut Editor, ticket: Ticket<RemoteCompletionKey>, result: RemoteCompletionResult) {
    e.handle_remote_completion(Completion {
        ticket,
        outcome: Outcome::Success(result),
    });
}

fn ticket_for(e: &Editor, text: &str, query: RemoteCompletionQuery) -> Ticket<RemoteCompletionKey> {
    let pending = e.remote_completion.ticket().expect("request in flight");
    assert_eq!(pending.key.text, text);
    assert_eq!(pending.key.query, query);
    pending
}

#[test]
fn host_candidates_apply_then_cycle_without_new_requests() {
    let mut e = editor();
    suppress(&mut e);
    e.feed_text(":e ssh://al");
    e.feed_text("<tab>");
    let ticket = ticket_for(
        &e,
        ":e ssh://al",
        RemoteCompletionQuery::Hosts {
            partial: "al".into(),
        },
    );
    deliver(
        &mut e,
        ticket,
        hosts_result(&["ssh://alpha.example.com", "ssh://alpine.example.com"]),
    );
    assert_eq!(e.pending.text(), ":e ssh://alpha.example.com");
    assert!(e.message.contains("alpine.example.com"));
    // The second Tab cycles the landed list; no new request exists.
    e.feed_text("<tab>");
    assert!(e.remote_completion.ticket().is_none());
    assert_eq!(e.pending.text(), ":e ssh://alpine.example.com");
    // …and wraps around.
    e.feed_text("<tab>");
    assert_eq!(e.pending.text(), ":e ssh://alpha.example.com");
}

#[test]
fn tail_range_browse_and_follow_complete_their_uri_operand() {
    let mut e = editor();
    suppress(&mut e);
    // :tail BYTES URI keeps the byte count through the apply.
    e.feed_text(":tail 64k ssh://bui");
    e.feed_text("<tab>");
    let ticket = ticket_for(
        &e,
        ":tail 64k ssh://bui",
        RemoteCompletionQuery::Hosts {
            partial: "bui".into(),
        },
    );
    deliver(&mut e, ticket, hosts_result(&["ssh://build.example.com"]));
    assert_eq!(e.pending.text(), ":tail 64k ssh://build.example.com");
    // :tail without a byte count completes too.
    e.feed_text("<esc><esc>");
    e.feed_text(":tail ssh://bui");
    e.feed_text("<tab>");
    let ticket = ticket_for(
        &e,
        ":tail ssh://bui",
        RemoteCompletionQuery::Hosts {
            partial: "bui".into(),
        },
    );
    deliver(&mut e, ticket, hosts_result(&["ssh://build.example.com"]));
    assert_eq!(e.pending.text(), ":tail ssh://build.example.com");
    // :range with a missing BYTES refuses with the shape hint.
    e.feed_text("<esc><esc>");
    e.feed_text(":range 100 ssh://bui");
    e.feed_text("<tab>");
    assert_eq!(e.message, ":range needs START BYTES before the URI");
    assert!(e.remote_completion.ticket().is_none());
    // :range START BYTES URI completes and keeps both numbers.
    e.feed_text("<esc><esc>");
    e.feed_text(":range 100 4k ssh://bui");
    e.feed_text("<tab>");
    let ticket = ticket_for(
        &e,
        ":range 100 4k ssh://bui",
        RemoteCompletionQuery::Hosts {
            partial: "bui".into(),
        },
    );
    deliver(&mut e, ticket, hosts_result(&["ssh://build.example.com"]));
    assert_eq!(e.pending.text(), ":range 100 4k ssh://build.example.com");
    // :browse completes paths like :e does.
    e.feed_text("<esc><esc>");
    e.feed_text(":browse ssh://build/var/l");
    e.feed_text("<tab>");
    let ticket = ticket_for(
        &e,
        ":browse ssh://build/var/l",
        RemoteCompletionQuery::Path {
            authority: "build".into(),
            directory: "/var".into(),
            segment: "l".into(),
        },
    );
    deliver(
        &mut e,
        ticket,
        RemoteCompletionResult::ConnectRequired {
            endpoint: "ssh://build".into(),
        },
    );
    assert!(e.message.contains("never connects"));
    // :follow with no operand is not a completion at all.
    e.feed_text("<esc><esc>");
    e.feed_text(":follow");
    e.feed_text("<tab>");
    assert!(e.remote_completion.ticket().is_none());
}

#[test]
fn late_results_cannot_replace_edited_input() {
    let mut e = editor();
    suppress(&mut e);
    e.feed_text(":e ssh://al");
    e.feed_text("<tab>");
    let ticket = e.remote_completion.ticket().expect("request in flight");
    e.feed_text("x"); // the user kept typing after the request
    deliver(
        &mut e,
        ticket,
        hosts_result(&["ssh://alpha.example.com", "ssh://alpine.example.com"]),
    );
    assert_eq!(
        e.pending.text(),
        ":e ssh://alx",
        "input must stay untouched"
    );
    assert!(e.remote_completion.ready.is_none());
    assert!(
        !e.message.contains("alpine"),
        "candidate list must not overwrite the line's state"
    );
}

#[test]
fn late_results_after_cancel_are_dropped_not_applied() {
    let mut e = editor();
    suppress(&mut e);
    e.feed_text(":e ssh://al");
    e.feed_text("<tab>");
    let ticket = e.remote_completion.ticket().expect("request in flight");
    e.feed_text("<esc><esc>"); // Esc-Esc closes the prompt
    assert!(!e.pending.is_active());
    deliver(&mut e, ticket, hosts_result(&["ssh://alpha.example.com"]));
    assert!(!e.pending.is_active(), "no prompt may be reopened");
    assert_eq!(e.buf().text().to_string(), "x\n");
}

#[test]
fn focus_change_rejects_delivery() {
    let mut e = editor();
    suppress(&mut e);
    e.feed_text(":e ssh://al");
    e.feed_text("<tab>");
    let ticket = e.remote_completion.ticket().expect("request in flight");
    e.focus_epoch += 1; // a pane switch happened after the request
    deliver(&mut e, ticket, hosts_result(&["ssh://alpha.example.com"]));
    assert_eq!(e.pending.text(), ":e ssh://al");
    assert!(e.remote_completion.ready.is_none());
}

#[test]
fn superseded_tickets_are_rejected() {
    let mut e = editor();
    suppress(&mut e);
    e.feed_text(":e ssh://al");
    e.feed_text("<tab>");
    let stale = e.remote_completion.ticket().expect("request in flight");
    e.feed_text("p");
    e.feed_text("<tab>"); // replaces the request
    let current = e.remote_completion.ticket().expect("request in flight");
    assert_ne!(stale.request, current.request);
    deliver(&mut e, stale, hosts_result(&["ssh://stale.example.com"]));
    assert_eq!(e.pending.text(), ":e ssh://alp");
    deliver(&mut e, current, hosts_result(&["ssh://alpha.example.com"]));
    assert_eq!(e.pending.text(), ":e ssh://alpha.example.com");
}

#[test]
fn path_completion_never_connects_and_says_so() {
    let mut e = editor();
    suppress(&mut e);
    e.feed_text(":e ssh://build/var/l");
    e.feed_text("<tab>");
    let ticket = ticket_for(
        &e,
        ":e ssh://build/var/l",
        RemoteCompletionQuery::Path {
            authority: "build".into(),
            directory: "/var".into(),
            segment: "l".into(),
        },
    );
    deliver(
        &mut e,
        ticket,
        RemoteCompletionResult::ConnectRequired {
            endpoint: "ssh://build".into(),
        },
    );
    assert_eq!(e.pending.text(), ":e ssh://build/var/l");
    assert!(e.message.contains("no live connection"));
    assert!(e.message.contains("never connects"));
    assert!(e.remote_completion.ready.is_none());
}

#[test]
fn directories_route_as_prefixes_files_as_full_uris() {
    let mut e = editor();
    suppress(&mut e);
    e.feed_text(":e ssh://build/var/l");
    e.feed_text("<tab>");
    let ticket = e.remote_completion.ticket().expect("request in flight");
    // Worker-sorted order: directories first, each group by URI.
    deliver(
        &mut e,
        ticket,
        RemoteCompletionResult::Candidates {
            items: vec![
                cand("ssh://build/var/log/", true),
                cand("ssh://build/var/local", false),
            ],
            source: CandidateSource::Connection,
            notes: Vec::new(),
            listed_directory: Some("ssh://build/var".into()),
        },
    );
    // Directories apply first and keep a trailing slash…
    assert_eq!(e.pending.text(), ":e ssh://build/var/log/");
    assert!(e.message.contains("log/"), "directory shown with its slash");
    e.feed_text("<tab>");
    assert_eq!(e.pending.text(), ":e ssh://build/var/local");
    e.feed_text("<tab>");
    assert_eq!(e.pending.text(), ":e ssh://build/var/log/");
    // …and the live listing feeds the fallback cache.
    assert!(e
        .remote_completion
        .cached("ssh://build/var")
        .is_some_and(|items| items.len() == 2));
    // Typing into the selected directory starts a deeper query rather than
    // cycling the original alternatives.
    e.feed_text("s<tab>");
    let ticket = ticket_for(
        &e,
        ":e ssh://build/var/log/s",
        RemoteCompletionQuery::Path {
            authority: "build".into(),
            directory: "/var/log".into(),
            segment: "s".into(),
        },
    );
    deliver(
        &mut e,
        ticket,
        RemoteCompletionResult::Candidates {
            items: vec![cand("ssh://build/var/log/syslog", false)],
            source: CandidateSource::Cache,
            notes: Vec::new(),
            listed_directory: None,
        },
    );
    assert_eq!(e.pending.text(), ":e ssh://build/var/log/syslog");
    assert!(
        e.message.contains("(cached)"),
        "cache is labeled, never live"
    );
}

#[test]
fn percent_prefixes_classify_into_the_path_segment() {
    let mut e = editor();
    suppress(&mut e);
    e.feed_text(":e ssh://build/%2");
    e.feed_text("<tab>");
    ticket_for(
        &e,
        ":e ssh://build/%2",
        RemoteCompletionQuery::Path {
            authority: "build".into(),
            directory: "/".into(),
            segment: "%2".into(),
        },
    );
    // A bare endpoint prefix stays host completion.
    e.feed_text("<esc><esc>");
    e.feed_text(":e ssh://bu");
    e.feed_text("<tab>");
    ticket_for(
        &e,
        ":e ssh://bu",
        RemoteCompletionQuery::Hosts {
            partial: "bu".into(),
        },
    );
}

#[cfg(unix)]
#[test]
fn applied_uris_keep_native_path_identity() {
    let mut e = editor();
    suppress(&mut e);
    e.feed_text(":e ssh://build/d");
    e.feed_text("<tab>");
    let ticket = e.remote_completion.ticket().expect("request in flight");
    deliver(
        &mut e,
        ticket,
        RemoteCompletionResult::Candidates {
            items: vec![cand("ssh://build/caf%FF%20menu.txt", false)],
            source: CandidateSource::Connection,
            notes: Vec::new(),
            listed_directory: None,
        },
    );
    assert_eq!(e.pending.text(), ":e ssh://build/caf%FF%20menu.txt");
    let file = RemoteFile::parse("ssh://build/caf%FF%20menu.txt").unwrap();
    assert_eq!(
        file.path().as_os_str().as_encoded_bytes(),
        b"/caf\xFF menu.txt".as_slice()
    );
}

#[test]
fn home_paths_are_refused_not_guessed() {
    let mut e = editor();
    e.feed_text(":e ssh://build/~");
    e.feed_text("<tab>");
    assert!(e.message.contains("home"));
    assert!(e.remote_completion.ticket().is_none());
    e.feed_text("<esc><esc>");
    e.feed_text(":e ssh://~/notes");
    e.feed_text("<tab>");
    assert!(e.message.contains("home"));
    assert!(e.remote_completion.ticket().is_none());
}

#[test]
fn write_commands_refuse_remote_targets() {
    let mut e = editor();
    e.feed_text(":w ssh://build/etc/passwd");
    e.feed_text("<tab>");
    assert_eq!(
        e.message,
        "remote snapshots are read-only; remote writes are not supported"
    );
    assert!(e.remote_completion.ticket().is_none());
}

#[test]
fn non_file_commands_fall_back_to_command_name_cycling() {
    let mut e = editor();
    e.feed_text(":help ssh://x/y");
    e.feed_text("<tab>");
    assert!(e.remote_completion.ticket().is_none());
    // Command-name cycling still works on the same key.
    e.feed_text("<esc><esc>");
    e.feed_text(":he");
    e.feed_text("<tab>");
    assert_eq!(e.pending.text(), ":help");
}

#[test]
fn raw_spaces_and_mid_line_cursors_refuse_with_a_hint() {
    let mut e = editor();
    e.feed_text(":e ssh://build/a b");
    e.feed_text("<tab>");
    assert!(e.message.contains("%20"), "raw space gets an escape hint");
    e.feed_text("<esc><esc>");
    e.feed_text(":e ssh://build/lo");
    e.feed_text("<esc>h"); // line's normal mode, caret moved left
    e.feed_pending(crate::editor::Key::Tab);
    assert!(e.message.contains("end of the line"));
}

#[test]
fn empty_candidates_report_the_first_note() {
    let mut e = editor();
    suppress(&mut e);
    e.feed_text(":e ssh://zz");
    e.feed_text("<tab>");
    let ticket = e.remote_completion.ticket().expect("request in flight");
    deliver(
        &mut e,
        ticket,
        RemoteCompletionResult::Candidates {
            items: Vec::new(),
            source: CandidateSource::Config,
            notes: vec!["2 hashed known_hosts entries cannot be completed".into()],
            listed_directory: None,
        },
    );
    assert_eq!(
        e.message,
        "no remote matches: 2 hashed known_hosts entries cannot be completed"
    );
}
