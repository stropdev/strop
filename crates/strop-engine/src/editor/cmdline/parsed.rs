//! What the open line means, parsed once per change of its text (plan
//! 0110 §4 step 7): the `:s` specification, the lines a range names, the
//! `/` pattern compiled. Preview, Enter's resolution and the renderer
//! read this; nothing parses or compiles per frame.

use strop_core::id::BufferRevision;
use strop_grammar::{CompiledQuery, QueryError};

use crate::editor::ex::{SubstituteError, SubstituteSpec};

/// The parse of one revision of the line.
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    /// The line revision this was parsed from.
    pub revision: Option<BufferRevision>,
    /// The line is a `:s` (`None`: it is not).
    pub substitute: Option<Result<SubstituteSpec, SubstituteError>>,
    /// The zero-based lines a typed range names, or why it names none.
    pub range: Option<Result<(usize, usize), String>>,
    /// A `/` or `?` pattern, compiled.
    pub search: Option<Result<CompiledQuery, QueryError>>,
}
