//! A ratchet on the `Editor` god object (0087 A4, 0088 §4): its decomposition
//! is scheduled for F2, and until then it may not grow. If a change trips
//! this test, move the new state into an owned component instead of
//! raising a constant. A change that removes fields or `impl Editor`
//! blocks lowers the matching constant in the same patch, so the ratchet
//! only ever turns one way.

/// Fields of `pub struct Editor` in `editor/mod.rs`. Lower it, never raise it.
const EDITOR_FIELDS: usize = 113;
/// Inherent `impl Editor` blocks across the crate. Lower it, never raise it.
const EDITOR_IMPL_BLOCKS: usize = 164;

/// Removes `//` comments (doc comments included) so commented-out or
/// documented text never counts. The struct body holds no string literals.
fn without_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.find("//").map_or(line, |at| &line[..at]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Counts `name:` at the struct's own nesting level: colons inside
/// generics, attributes, tuples or `::` paths are not fields.
fn struct_fields(source: &str, header: &str) -> usize {
    let source = without_line_comments(source);
    let start = source.find(header).expect("Editor struct header") + header.len();
    let body = &source.as_bytes()[start..];
    let (mut depth, mut fields) = (1usize, 0usize);
    for (at, &byte) in body.iter().enumerate() {
        let previous = at.checked_sub(1).map(|at| body[at]);
        match byte {
            b'{' | b'(' | b'[' | b'<' => depth += 1,
            b'>' if previous == Some(b'-') => {}
            b'}' | b')' | b']' | b'>' => {
                depth -= 1;
                if depth == 0 {
                    return fields;
                }
            }
            b':' if depth == 1 && previous != Some(b':') && body.get(at + 1) != Some(&b':') => {
                fields += 1
            }
            _ => {}
        }
    }
    panic!("unterminated Editor struct");
}

fn is_inherent_editor_impl(line: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix("impl ") else {
        return false;
    };
    let path = rest.trim_end().trim_end_matches('{').trim_end();
    path.rsplit("::").next() == Some("Editor") && !path.contains(" for ")
}

fn impl_blocks(directory: &std::path::Path) -> usize {
    let mut count = 0;
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            count += impl_blocks(&path);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = without_line_comments(&std::fs::read_to_string(&path).unwrap());
            count += source
                .lines()
                .filter(|line| is_inherent_editor_impl(line))
                .count();
        }
    }
    count
}

#[test]
fn editor_field_count_may_not_grow() {
    let fields = struct_fields(include_str!("../mod.rs"), "pub struct Editor {");
    assert!(
        fields <= EDITOR_FIELDS,
        "Editor grew to {fields} fields (ratchet {EDITOR_FIELDS}): put the new state in an \
         owned component instead (0087 A4)"
    );
    assert_eq!(
        fields, EDITOR_FIELDS,
        "Editor shrank to {fields} fields: lower EDITOR_FIELDS to {fields} in this change"
    );
}

#[test]
fn editor_impl_block_count_may_not_grow() {
    let blocks = impl_blocks(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
    assert!(
        blocks <= EDITOR_IMPL_BLOCKS,
        "{blocks} `impl Editor` blocks (ratchet {EDITOR_IMPL_BLOCKS}): extend an owned \
         component instead of adding another Editor impl (0087 A4)"
    );
    assert_eq!(
        blocks, EDITOR_IMPL_BLOCKS,
        "`impl Editor` blocks dropped to {blocks}: lower EDITOR_IMPL_BLOCKS to {blocks} in \
         this change"
    );
}

#[test]
fn the_counters_see_through_comments_generics_and_paths() {
    let source = "pub struct Editor {\n\
        /// a: documented\n\
        pub(crate) a: std::collections::HashMap<u8, Vec<(u8, u8)>>,\n\
        // b: commented out\n\
        #[cfg(test)]\n\
        c: Box<dyn Fn(u8) -> Option<u8>>,\n\
        d: [u8; 2],\n\
    }\n\
    struct Other { e: u8 }";
    assert_eq!(struct_fields(source, "pub struct Editor {"), 3);
    assert!(is_inherent_editor_impl("impl Editor {"));
    assert!(is_inherent_editor_impl("    impl super::super::Editor {"));
    assert!(!is_inherent_editor_impl("impl Drop for Editor {"));
    assert!(!is_inherent_editor_impl("impl EditorState {"));
}
