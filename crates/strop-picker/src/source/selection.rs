//! The one file-selection authority (0051 §3): the same compiled
//! `FileSelectionPlan` plus the session visibility policy produce the
//! same eligible path set for file find, grep and replace. `.git/` and
//! other repository internals are protected metadata — never ordinary
//! project search results, even with `ignored:include`.

use std::path::Path;

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

/// Stream eligible workspace-relative paths. The walk honors ignore
/// files by default; hidden entries show by default (0051 R03); plan
/// predicates filter; `.git` is always excluded.
pub fn walk(
    cwd: &Path,
    plan: &FileSelectionPlan,
    policy: SelectionPolicy,
    cancelled: &impl Fn() -> bool,
    mut emit: impl FnMut(std::path::PathBuf) -> bool,
) -> Result<(), String> {
    let effective = policy.effective(plan);
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
    for result in builder.build() {
        if cancelled() {
            return Ok(());
        }
        let entry = result.map_err(|error| error.to_string())?;
        let Some(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            // only links to regular files are eligible; a broken or
            // unreadable link is skipped, never a walk-stopping error
            // (one dangling link must not empty Space f / Space /)
            let Ok(metadata) = std::fs::metadata(entry.path()) else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
        } else if !kind.is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(cwd) else {
            continue;
        };
        if protected(rel) {
            continue;
        }
        let display = display_path(rel);
        if !plan.allows(&display) {
            continue;
        }
        if !emit(rel.to_path_buf()) {
            return Ok(());
        }
    }
    Ok(())
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
