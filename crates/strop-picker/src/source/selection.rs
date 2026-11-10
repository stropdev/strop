//! The one file-selection authority (0051 §3): the same compiled
//! `FileSelectionPlan` plus the session visibility policy produce the
//! same eligible path set for file find, grep and replace. `.git/` and
//! other repository internals are protected metadata — never ordinary
//! project search results, even with `ignored:include`.

use std::path::Path;
pub(super) const PATH_LIMIT: usize = 100_000;
pub(super) const PATH_BYTES: usize = 16 * 1024 * 1024;

/// Portable display/search spelling; the native path remains the I/O identity.
pub fn display_path(path: &Path) -> std::borrow::Cow<'_, str> {
    let value = path.to_string_lossy();
    if std::path::MAIN_SEPARATOR == '/' {
        value
    } else {
        std::borrow::Cow::Owned(value.replace(std::path::MAIN_SEPARATOR, "/"))
    }
}

use crate::query::FileSelectionPlan;

/// Session/workspace visibility defaults (query qualifiers override).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionPolicy {
    /// Show unignored dotfiles/dotfolders (config `search_show_hidden`).
    pub hidden: bool,
    /// Respect .gitignore/.ignore/.rgignore (`search_respect_ignore`).
    pub respect_ignore: bool,
}

impl SelectionPolicy {
    /// The effective policy for one query: its explicit qualifiers win.
    pub fn effective(&self, plan: &FileSelectionPlan) -> SelectionPolicy {
        SelectionPolicy {
            hidden: plan.hidden.unwrap_or(self.hidden),
            // `ignored:include` INCLUDES ignored entries = do not respect
            // the ignore files; `ignored:exclude` respects them.
            respect_ignore: plan
                .ignored
                .map(|include| !include)
                .unwrap_or(self.respect_ignore),
        }
    }
}

/// A path component never treated as ordinary project content.
fn protected(rel: &Path) -> bool {
    rel.components()
        .any(|component| component.as_os_str() == ".git")
}

/// The walk both surfaces share: ignore files by default, hidden entries
/// by policy, `.git` never.
fn builder(cwd: &Path, effective: SelectionPolicy) -> ignore::WalkBuilder {
    let mut builder = ignore::WalkBuilder::new(cwd);
    builder
        .filter_entry(|entry| entry.file_name() != ".git")
        .hidden(!effective.hidden)
        .ignore(effective.respect_ignore)
        .git_ignore(effective.respect_ignore)
        .git_exclude(effective.respect_ignore);
    builder
        .git_global(effective.respect_ignore)
        .parents(effective.respect_ignore);
    builder
}

/// The workspace-relative path of an eligible entry: a regular file, or
/// a link to one, outside protected metadata, that the plan allows. A
/// broken or unreadable link is skipped, never a walk-stopping error
/// (one dangling link must not empty Space f / Space /).
fn eligible(
    entry: &ignore::DirEntry,
    cwd: &Path,
    plan: &FileSelectionPlan,
) -> Option<std::path::PathBuf> {
    let kind = entry.file_type()?;
    if kind.is_symlink() {
        if !std::fs::metadata(entry.path()).ok()?.is_file() {
            return None;
        }
    } else if !kind.is_file() {
        return None;
    }
    let rel = entry.path().strip_prefix(cwd).ok()?;
    if protected(rel) || !plan.allows(&display_path(rel)) {
        return None;
    }
    Some(rel.to_path_buf())
}

/// Stream eligible workspace-relative paths in tree order: sorting each
/// directory by name makes the file finder list the same tree the same
/// way everywhere.
pub fn walk(
    cwd: &Path,
    plan: &FileSelectionPlan,
    policy: SelectionPolicy,
    cancelled: &impl Fn() -> bool,
    mut emit: impl FnMut(std::path::PathBuf) -> bool,
) -> Result<(), String> {
    let mut builder = builder(cwd, policy.effective(plan));
    builder.sort_by_file_name(|a, b| a.cmp(b));
    for result in builder.build() {
        if cancelled() {
            return Ok(());
        }
        let entry = result.map_err(|error| error.to_string())?;
        if let Some(rel) = eligible(&entry, cwd, plan) {
            if !emit(rel) {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Why a gathered selection has no path list.
#[derive(Debug)]
pub enum CollectError {
    /// More than `PATH_LIMIT` paths or `PATH_BYTES` of them.
    Limit,
    Walk(String),
}

/// The same eligible set as [`walk`], gathered on `threads` threads in no
/// particular order (0093 §4.1): content search wants every path as soon
/// as possible and orders its results itself. A cancelled walk returns
/// what it had; the caller checks its own token.
pub fn collect(
    cwd: &Path,
    plan: &FileSelectionPlan,
    policy: SelectionPolicy,
    cancelled: &(impl Fn() -> bool + Sync),
    threads: usize,
) -> Result<Vec<std::path::PathBuf>, CollectError> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    /// A thread's finds, handed over in runs and once more when it stops.
    struct Run<'a> {
        local: Vec<std::path::PathBuf>,
        gathered: &'a parking_lot::Mutex<Vec<std::path::PathBuf>>,
    }
    impl Drop for Run<'_> {
        fn drop(&mut self) {
            self.gathered.lock().append(&mut self.local);
        }
    }
    let gathered = parking_lot::Mutex::new(Vec::new());
    let failure = parking_lot::Mutex::new(None);
    let (count, bytes) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let over = AtomicBool::new(false);
    let mut builder = builder(cwd, policy.effective(plan));
    builder.threads(threads);
    builder.build_parallel().run(|| {
        let mut run = Run {
            local: Vec::new(),
            gathered: &gathered,
        };
        let (failure, count, bytes, over) = (&failure, &count, &bytes, &over);
        Box::new(move |result| {
            if cancelled() || over.load(Ordering::Acquire) {
                return ignore::WalkState::Quit;
            }
            let entry = match result {
                Ok(entry) => entry,
                Err(error) => {
                    failure.lock().get_or_insert(error.to_string());
                    over.store(true, Ordering::Release);
                    return ignore::WalkState::Quit;
                }
            };
            let Some(rel) = eligible(&entry, cwd, plan) else {
                return ignore::WalkState::Continue;
            };
            let size = rel.as_os_str().as_encoded_bytes().len();
            if count.fetch_add(1, Ordering::AcqRel) >= PATH_LIMIT
                || bytes.fetch_add(size, Ordering::AcqRel) + size > PATH_BYTES
            {
                over.store(true, Ordering::Release);
                return ignore::WalkState::Quit;
            }
            run.local.push(rel);
            if run.local.len() >= 256 {
                run.gathered.lock().append(&mut run.local);
            }
            ignore::WalkState::Continue
        })
    });
    if let Some(error) = failure.into_inner() {
        return Err(CollectError::Walk(error));
    }
    if over.into_inner() {
        return Err(CollectError::Limit);
    }
    Ok(gathered.into_inner())
}

/// The protected-path exclusion as rg argv fragments (`--glob !.git`…).
/// Shared by every rg invocation so no surface searches repository
/// internals by accident.
pub const RG_PROTECTED_ARGS: &[&str] = &["--glob", "!.git", "--glob", "!.git/**"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::SearchQuery;

    fn collect(cwd: &Path, query: &str, policy: SelectionPolicy) -> Vec<String> {
        let parsed = SearchQuery::parse(query);
        let plan = FileSelectionPlan::compile(&parsed).unwrap();
        let mut out = Vec::new();
        walk(cwd, &plan, policy, &|| false, |rel| {
            out.push(rel.display().to_string());
            true
        })
        .unwrap();
        out
    }

    fn fixture() -> tempfile::TempDir {
        // .gitignore applies inside git repositories (ignore/rg default)
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("src/lib.py"), "x = 1\n").unwrap();
        std::fs::write(dir.path().join(".env"), "TOKEN=x\n").unwrap();
        std::fs::write(dir.path().join(".github/workflows/ci.yml"), "on: push\n").unwrap();
        std::fs::write(dir.path().join("target/build.rs"), "// build\n").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        dir
    }

    #[test]
    fn dotfiles_show_by_default_but_ignore_files_hold() {
        let dir = fixture();
        let got = collect(
            dir.path(),
            "",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert!(got.contains(&".env".to_string()), "{got:?}");
        assert!(
            got.contains(&".github/workflows/ci.yml".to_string()),
            "{got:?}"
        );
        assert!(!got.contains(&"target/build.rs".to_string()), "{got:?}");
    }

    #[test]
    fn ignored_include_surfaces_ignored_output() {
        let dir = fixture();
        let got = collect(
            dir.path(),
            "ignored:include",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert!(got.contains(&"target/build.rs".to_string()), "{got:?}");
    }

    #[test]
    fn hidden_exclude_restores_the_old_world() {
        let dir = fixture();
        let got = collect(
            dir.path(),
            "hidden:exclude",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert!(!got.contains(&".env".to_string()), "{got:?}");
        assert!(
            !got.contains(&".github/workflows/ci.yml".to_string()),
            "{got:?}"
        );
        assert!(got.contains(&"src/main.rs".to_string()), "{got:?}");
    }

    #[test]
    fn fuzzy_path_scope_filters_real_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        for path in [
            "src/modules/jweb/Parser.java",
            "src/modules/other/Parser.java",
            "modules/legacy/jweb/parser.java",
        ] {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "class Parser {}\n").unwrap();
        }
        let got = collect(
            dir.path(),
            "fpath:modules/jweb/ parser",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.iter().all(|path| path.contains("jweb")));
    }

    #[test]
    fn language_filter_only_lists_that_family() {
        let dir = fixture();
        let got = collect(
            dir.path(),
            "language:rust",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert_eq!(got, vec!["src/main.rs".to_string()]);
    }

    #[test]
    fn git_internals_are_never_results() {
        let dir = fixture();
        std::fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "[core]\n").unwrap();
        let got = collect(
            dir.path(),
            "ignored:include",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert!(!got.iter().any(|p| p.starts_with(".git/")), "{got:?}");
    }

    /// One dangling link is ineligible, never a walk-stopping error:
    /// the rest of the workspace still streams (0051 §3 boundaries).
    #[cfg(unix)]
    #[test]
    fn a_broken_symlink_never_empties_the_walk() {
        let dir = fixture();
        std::os::unix::fs::symlink("missing-target", dir.path().join("dangling")).unwrap();
        std::os::unix::fs::symlink("src/main.rs", dir.path().join("linked.rs")).unwrap();
        let got = collect(
            dir.path(),
            "",
            SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
        );
        assert!(got.contains(&"src/main.rs".to_string()), "{got:?}");
        assert!(got.contains(&"linked.rs".to_string()), "{got:?}");
        assert!(!got.contains(&"dangling".to_string()), "{got:?}");
    }
}
