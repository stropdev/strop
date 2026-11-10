//! Where strop deliberately differs from Neovim. Each entry is a case
//! name, or a prefix ending in `*`, and the reason. The list is capped:
//! a new divergence means fixing one first.

/// The corpus may never shrink below this (0095 §4).
pub const MINIMUM_CASES: usize = 90;

pub const MAXIMUM_KNOWN: usize = 10;

pub const KNOWN: &[(&str, &str)] = &[(
    "daw on line-ending blanks is a no-op",
    "vim's FAILED current_word still moves the cursor one column (its FAIL path \
     returns after incl()); strop refuses without moving the cursor. Text agrees; \
     only the cursor side effect of a refused command differs.",
)];

/// The index of the entry that explains this case, if one does.
pub fn reason(name: &str) -> Option<usize> {
    KNOWN
        .iter()
        .position(|(pattern, _)| match pattern.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => name == *pattern,
        })
}
