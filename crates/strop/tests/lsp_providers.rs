//! #244: a server that answers a provider with `false` is not asked. The
//! shipped `strop headless` talks to a fixture server that declares the
//! given capabilities and logs every method it receives; the log, not
//! Strop's message, is the witness.
use std::fs;
use std::process::Command;

/// The four wire forms of a boolean-or-options provider, and whether each
/// enables it.
const FORMS: [(Option<&str>, bool); 4] = [
    (None, false),
    (Some("false"), false),
    (Some("true"), true),
    (Some("{}"), true),
];

struct Run {
    methods: Vec<String>,
    message: String,
}

/// Open `main.py` in a fresh editor whose Python server declares
/// `capabilities`, then type `keys`. No system servers on PATH.
fn run(capabilities: &str, keys: &str) -> Run {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let work = base.join("work");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join("pyproject.toml"), "").unwrap();
    fs::write(work.join("main.py"), "answer = 42\nprint(answer)\n").unwrap();
    let python = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("python3"))
        .find(|path| path.is_file())
        .expect("python3 for the fixture server");
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/providers_server.py"
    );
    let log = base.join("methods.log");
    let config = base.join("config/strop");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("languages.toml"),
        format!(
            "[language-server.pyright]\ncommand = {python:?}\nargs = [{script:?}, {log:?}, {capabilities:?}]\n"
        ),
    )
    .unwrap();
    let steps = base.join("steps");
    fs::write(
        &steps,
        format!("settle 20000\nkeys {keys}\nsettle 20000\nstate\n"),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_strop"))
        .current_dir(&work)
        .env("HOME", base.join("home"))
        .env("XDG_CONFIG_HOME", base.join("config"))
        .env("XDG_STATE_HOME", base.join("state"))
        .env("PATH", base.join("empty-bin"))
        .env_remove("STROP_LOG")
        .arg("headless")
        .arg(&steps)
        .arg("main.py")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let state = stdout
        .lines()
        .find_map(|line| line.strip_prefix("─── state "))
        .unwrap_or_else(|| panic!("no state in:\n{stdout}"));
    let state: serde_json::Value = serde_json::from_str(state).unwrap();
    let methods = fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    Run {
        methods,
        message: state["message"].as_str().unwrap_or_default().to_owned(),
    }
}

fn capabilities(key: &str, form: Option<&str>) -> String {
    form.map_or_else(
        || "{}".to_owned(),
        |value| format!("{{\"{key}\": {value}}}"),
    )
}

/// `keys` on `answer` asks for `method` exactly when `key` is enabled, and
/// a refusal says so.
fn admits_only_an_enabled_provider(keys: &str, key: &str, method: &str, label: &str) {
    for (form, enabled) in FORMS {
        let run = run(&capabilities(key, form), keys);
        let sent = run.methods.iter().filter(|seen| *seen == method).count();
        let refusal = format!("lsp: {label} is not supported by this server");
        let what = format!("{key} = {}", form.unwrap_or("absent"));
        assert!(
            run.methods.iter().any(|seen| seen == "initialize"),
            "{what}: the server started: {:?}",
            run.methods
        );
        if enabled {
            assert_eq!(sent, 1, "{what}: {:?}", run.methods);
            assert_ne!(run.message, refusal, "{what}");
        } else {
            assert_eq!(sent, 0, "{what}: {:?}", run.methods);
            assert_eq!(run.message, refusal, "{what}");
        }
    }
}

#[test]
fn implementation_is_asked_only_when_enabled() {
    admits_only_an_enabled_provider(
        "gI",
        "implementationProvider",
        "textDocument/implementation",
        "implementation",
    );
}

#[test]
fn declaration_is_asked_only_when_enabled() {
    admits_only_an_enabled_provider(
        "gD",
        "declarationProvider",
        "textDocument/declaration",
        "declaration",
    );
}

#[test]
fn type_definition_is_asked_only_when_enabled() {
    admits_only_an_enabled_provider(
        "gy",
        "typeDefinitionProvider",
        "textDocument/typeDefinition",
        "type definition",
    );
}

/// The control: workspace symbols already told `false` from `true`.
#[test]
fn workspace_symbols_are_asked_only_when_enabled() {
    for (value, enabled) in [("false", false), ("true", true)] {
        let run = run(
            &capabilities("workspaceSymbolProvider", Some(value)),
            "<space>Sanswer",
        );
        let sent = run.methods.iter().any(|seen| seen == "workspace/symbol");
        assert_eq!(sent, enabled, "{value}: {:?}", run.methods);
    }
}
