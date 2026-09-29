//! Stable command inventory exposed to native frontends. This mirrors the
//! engine's keymap-as-data owner; the GUI must not build its own command
//! table or infer supported actions from rendering code.

use strop_core::commands::{Binding, CommandKind, SECTIONS};

/// One supported action row, in stable table order. Planned slots remain
/// visible and inert, matching the TUI's `(soon)` presentation rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandRow {
    pub id: &'static str,
    pub keys: &'static str,
    pub description: &'static str,
    pub sections: &'static [&'static str],
    pub live: bool,
    pub contextual: bool,
}

pub fn commands() -> impl Iterator<Item = CommandRow> {
    strop_core::commands::BINDINGS.iter().map(command_row)
}

pub fn command_row(binding: &Binding) -> CommandRow {
    CommandRow {
        id: binding.id,
        keys: binding.keys,
        description: binding.desc,
        sections: binding.sections,
        live: binding.live,
        contextual: matches!(binding.kind, CommandKind::Contextual | CommandKind::Soon),
    }
}

pub fn sections() -> &'static [&'static str] {
    SECTIONS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_inventory_is_stable_nonempty_and_honest_about_planned_slots() {
        let rows: Vec<_> = commands().collect();
        assert!(!rows.is_empty());
        assert!(rows.iter().any(|row| row.live));
        assert!(rows.iter().any(|row| row.contextual));
        assert!(sections().contains(&"terminal"));
        let mut unique = std::collections::BTreeSet::new();
        for row in rows {
            unique.insert(row.id);
        }
        assert!(!unique.is_empty());
    }
}
