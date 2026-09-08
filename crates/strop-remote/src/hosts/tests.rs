//! Enumeration tests: fixture directories only — no real `~/.ssh`, no
//! `ssh` binary, no network. The no-subprocess/no-auth property is
//! structural (the module spawns nothing); these tests pin the data
//! behavior and the honest notes.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Fixture directory under the system temp dir; removed on drop so
/// tests never depend on (or leak into) a real `~/.ssh`.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "strop-hosts-{}-{}-{tag}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&path).unwrap();
        Dir(path)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ssh_fixture(tag: &str, config: &str) -> (Dir, HostSources) {
    let dir = Dir::new(tag);
    let ssh = dir.join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(ssh.join("config"), config).unwrap();
    let mut sources = HostSources::default();
    sources
        .push_config(ssh.join("config"))
        .set_include_base(ssh.clone());
    (dir, sources)
}

fn tokens(sources: &HostSources, typed: &str) -> Vec<String> {
    enumerate_hosts(sources, &[]).complete(typed)
}

#[test]
fn config_blocks_qualify_literal_aliases_only() {
    let (_dir, sources) = ssh_fixture(
        "blocks",
        "Host alpha\n  User bob\n  Port 2222\nHost beta gamma\nHost !dropped *.wild\n",
    );
    assert_eq!(tokens(&sources, ""), ["beta", "bob@alpha:2222", "gamma"]);
    assert_eq!(tokens(&sources, "al"), ["bob@alpha:2222"]);
    assert_eq!(tokens(&sources, "g"), ["gamma"]);
    assert_eq!(tokens(&sources, "dropped"), Vec::<String>::new());
    assert_eq!(tokens(&sources, "*.w"), Vec::<String>::new());
}

#[test]
fn typed_user_and_port_digits_are_respected() {
    let (_dir, sources) = ssh_fixture("typing", "Host alpha\n  User bob\n  Port 2222\n");
    assert_eq!(tokens(&sources, "carol@al"), ["carol@alpha:2222"]);
    assert_eq!(tokens(&sources, "alpha:2"), ["bob@alpha:2222"]);
    assert_eq!(
        tokens(&sources, "alpha:9"),
        Vec::<String>::new(),
        "a typed port prefix must extend the configured port, not reset it"
    );
}

#[test]
fn global_user_and_port_qualify_their_file() {
    let (_dir, sources) = ssh_fixture("globals", "User carol\nPort 2200\nHost delta\n");
    assert_eq!(tokens(&sources, "de"), ["carol@delta:2200"]);
}

#[test]
fn match_blocks_contribute_nothing_and_exec_never_runs() {
    let dir = Dir::new("match");
    let mut sources = HostSources::default();
    sources.push_config(dir.write(
        "config",
        "Match host secret.example.com exec \"touch pwned\"\nHost visible\n",
    ));
    assert_eq!(tokens(&sources, ""), ["visible"]);
    assert_eq!(tokens(&sources, "secret"), Vec::<String>::new());
    assert!(
        !dir.join("pwned").exists(),
        "enumeration must not run Match exec"
    );
}

#[test]
fn include_glob_relative_and_cycles_are_bounded() {
    let (dir, sources) = ssh_fixture(
        "include",
        "Include conf.d/*.conf extra\nHost cycle-a\nInclude cycle.config\n",
    );
    let ssh = dir.join(".ssh");
    std::fs::create_dir_all(ssh.join("conf.d")).unwrap();
    std::fs::write(ssh.join("conf.d/b.conf"), "Host from-glob\n").unwrap();
    std::fs::write(ssh.join("extra"), "Host from-extra\n").unwrap();
    std::fs::write(ssh.join("cycle.config"), "Include config\nHost cyc\n").unwrap();
    let enumeration = enumerate_hosts(&sources, &[]);
    assert_eq!(
        enumeration.complete(""),
        ["cyc", "cycle-a", "from-extra", "from-glob"]
    );
    assert!(
        enumeration
            .notes()
            .iter()
            .any(|note| note.contains("cycle")),
        "the include cycle must be an explicit note: {:?}",
        enumeration.notes()
    );
}

#[test]
fn relative_include_without_base_is_a_note() {
    let dir = Dir::new("relative");
    let config = dir.write("config", "Include partners.conf\nHost lonely\n");
    let mut sources = HostSources::default();
    sources.push_config(config);
    std::fs::write(dir.join("partners.conf"), "Host partner\n").unwrap();
    let enumeration = enumerate_hosts(&sources, &[]);
    assert_eq!(enumeration.complete(""), ["lonely"]);
    assert!(enumeration
        .notes()
        .iter()
        .any(|note| note.contains("without an include base")));
}

#[test]
fn known_hosts_variants() {
    let dir = Dir::new("known");
    let known = dir.write(
        "known_hosts",
        concat!(
            "plain.example.com ssh-ed25519 AAAA data\n",
            "[bracketed.example.com]:2222 ssh-ed25519 AAAA data\n",
            "|1|c2FsdA==|aGFzaA== ssh-ed25519 AAAA data\n",
            "*.wild.example.com,mixed.example.com ssh-ed25519 AAAA data\n",
            "@revoked revoked.example.com ssh-ed25519 AAAA data\n",
        ),
    );
    let mut sources = HostSources::default();
    sources.push_known_hosts(known);
    let enumeration = enumerate_hosts(&sources, &[]);
    assert_eq!(
        enumeration.complete(""),
        [
            "bracketed.example.com:2222",
            "mixed.example.com",
            "plain.example.com"
        ]
    );
    assert!(enumeration
        .notes()
        .iter()
        .any(|note| note.contains("hashed")));
}

#[test]
fn inadmissible_names_are_skipped_not_guessed() {
    let (_dir, sources) = ssh_fixture("inadmissible", "Host =bad -worse okay\n");
    let enumeration = enumerate_hosts(&sources, &[]);
    assert_eq!(enumeration.complete(""), ["okay"]);
    assert!(enumeration
        .notes()
        .iter()
        .any(|note| note.contains("not valid endpoints")));
}

#[test]
fn history_is_admitted_first_and_deduplicates_identical_tokens() {
    let (_dir, sources) = ssh_fixture("history", "Host hist.example.com\n  User config-user\n");
    let history = vec![
        HostCandidate::new(
            "hist.example.com".into(),
            Some("live-user".into()),
            None,
            CandidateOrigin::History,
        ),
        HostCandidate::new(
            "dup.example.com".into(),
            None,
            None,
            CandidateOrigin::History,
        ),
    ];
    let enumeration = enumerate_hosts(&sources, &history);
    // Different users on one host are two honest endpoints; both surface.
    assert_eq!(
        enumeration.complete("hist"),
        ["config-user@hist.example.com", "live-user@hist.example.com"]
    );
    // History is admitted first, so an identical config token dedups away.
    assert_eq!(enumeration.complete("dup"), ["dup.example.com"]);
    assert_eq!(
        enumeration.candidates()[0].origin(),
        CandidateOrigin::History
    );
    assert!(
        enumeration
            .candidates()
            .iter()
            .filter(|candidate| candidate.host() == "dup.example.com")
            .count()
            == 1
    );
}

#[test]
fn unreadable_input_is_a_note_not_a_failure() {
    let dir = Dir::new("unreadable");
    std::fs::create_dir(dir.join("config")).unwrap();
    // A directory cannot be read as a config file: an honest error,
    // never a panic — and the remaining sources still complete.
    let mut sources = HostSources::default();
    sources
        .push_config(dir.join("config"))
        .push_known_hosts(dir.write("known_hosts", "rescued.example.com ssh-ed25519 AAAA\n"));
    let enumeration = enumerate_hosts(&sources, &[]);
    assert_eq!(enumeration.complete(""), ["rescued.example.com"]);
    assert!(!enumeration.notes().is_empty());
}

#[test]
fn missing_files_are_silently_absent() {
    let mut sources = HostSources::default();
    sources
        .push_config(PathBuf::from("/nonexistent/strop/config"))
        .push_known_hosts(PathBuf::from("/nonexistent/strop/known_hosts"));
    let enumeration = enumerate_hosts(&sources, &[]);
    assert!(enumeration.candidates().is_empty());
    assert!(
        enumeration.notes().is_empty(),
        "absence is normal, not a diagnostic: {:?}",
        enumeration.notes()
    );
}

#[test]
fn quoted_tokens_and_comments_tokenize_like_ssh() {
    let (_dir, sources) = ssh_fixture(
        "tokenize",
        "# full-line comment\nHost \"quoted alias\" plain\nInclude ignored-by-this-test\n",
    );
    let enumeration = enumerate_hosts(&sources, &[]);
    // A space inside quotes makes one token — not a valid endpoint,
    // so it is skipped rather than split into two guesses.
    assert_eq!(enumeration.complete(""), ["plain"]);
}

#[test]
fn discover_lists_standard_locations_and_survives_no_home() {
    let home = Path::new("/home/nobody");
    let sources = HostSources::discover(Some(home));
    assert_eq!(
        sources.config,
        vec![
            home.join(".ssh/config"),
            PathBuf::from("/etc/ssh/ssh_config"),
        ]
    );
    assert_eq!(
        sources.known_hosts,
        vec![
            home.join(".ssh/known_hosts"),
            PathBuf::from("/etc/ssh/ssh_known_hosts"),
        ]
    );
    assert_eq!(sources.include_base, Some(home.join(".ssh")));
    // Pure construction: a nonexistent home (or none at all) reads
    // nothing and still names the system files.
    let no_home = HostSources::discover(None);
    assert_eq!(no_home.config, vec![PathBuf::from("/etc/ssh/ssh_config")]);
    assert!(no_home.include_base.is_none());
}
