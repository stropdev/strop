use super::{Edit, Request, Trigger};
use crate::editor::{Document, Editor};
use strop_core::id::{BufferRevision, ByteOffset, DocumentId};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Mapping {
    pub source: ByteOffset,
    pub view: ByteOffset,
    pub first: ByteOffset,
    pub last: ByteOffset,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Source {
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub request: Request,
    pub mappings: Vec<Mapping>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Batch {
    pub trigger: Trigger,
    pub sources: Vec<Source>,
}

impl Source {
    pub(crate) fn cursors(&self) -> Vec<usize> {
        self.mappings
            .iter()
            .map(|mapping| mapping.source.get())
            .collect()
    }

    pub(crate) fn project(&self, edits: Vec<Edit>) -> Result<Vec<Edit>, String> {
        if self
            .mappings
            .iter()
            .all(|mapping| mapping.source == mapping.view && mapping.first.get() == 0)
        {
            return Ok(edits);
        }
        let mut projected = Vec::new();
        for mapping in &self.mappings {
            let Some(edit) = edits.iter().find(|edit| edit.cursor == mapping.source) else {
                continue;
            };
            if edit.start < mapping.first || edit.end > mapping.last {
                return Err("indentation crosses an excerpt boundary".into());
            }
            let project = |at: ByteOffset| {
                ByteOffset::new(
                    mapping
                        .view
                        .get()
                        .saturating_add_signed(at.get() as isize - mapping.source.get() as isize),
                )
            };
            projected.push(Edit {
                cursor: mapping.view,
                start: project(edit.start),
                end: project(edit.end),
                text: edit.text.clone(),
                landing: edit.landing,
            });
        }
        Ok(projected)
    }
}

impl Batch {
    pub(crate) fn order(edits: &mut Vec<Edit>) -> Result<(), String> {
        edits.sort_by_key(|edit| (edit.start, edit.end));
        // Stacked cursors edit once. Multiple distinct edits must not consume
        // one another's pre-edit coordinates.
        edits.dedup_by(|right, left| right.cursor == left.cursor);
        if edits
            .windows(2)
            .any(|pair| pair[0].end > pair[1].start || pair[0].start == pair[1].start)
        {
            return Err("indentation cursors overlap".into());
        }
        Ok(())
    }
}

fn request(document: &Document, trigger: Trigger) -> Request {
    let path = document.syntax_path();
    let rope = document.buf.text();
    let first: Option<String> = (rope.get_byte(0) == Some(b'#') && rope.get_byte(1) == Some(b'!'))
        .then(|| {
            rope.chars()
                .take(256)
                .take_while(|&ch| ch != '\n')
                .collect()
        });
    let detected =
        strop_syntax::languages::detect(path.unwrap_or(std::path::Path::new("")), first.as_deref());
    use strop_syntax::languages::LanguageId;
    let supported = detected.as_ref().is_some_and(|spec| {
        matches!(
            spec.id,
            LanguageId::Rust
                | LanguageId::Python
                | LanguageId::JavaScript
                | LanguageId::TypeScript
                | LanguageId::Go
                | LanguageId::C
                | LanguageId::Cpp
                | LanguageId::Json
                | LanguageId::Java
                | LanguageId::CSharp
                | LanguageId::Css
        )
    });
    let syntax_path = if supported {
        Some(path.unwrap_or(std::path::Path::new("")).to_path_buf())
    } else if path.is_none() && detected.is_none() {
        Some(std::path::PathBuf::from("scratch.rs"))
    } else {
        None
    };
    let end = document.buf.line_end(0);
    let newline = if document.buf.byte_at(end) == Some(b'\r')
        && document.buf.byte_at(end + 1) == Some(b'\n')
    {
        "\r\n"
    } else {
        "\n"
    };
    Request {
        trigger,
        syntax_path,
        colons: detected.map_or(strop_syntax::ColonPolicy::None, |spec| {
            spec.id.colon_policy()
        }),
        unit: document.indent.unit(),
        newline: newline.into(),
    }
}

impl Editor {
    pub(crate) fn indentation_request(&self, trigger: Trigger) -> Option<Batch> {
        if self.buf().readonly {
            return None;
        }
        let mut sources: Vec<Source> = Vec::new();
        for view in self.all_cursors() {
            let (source, at) = self.source_edit_position(self.current(), view)?;
            let document = self.docs.get(source)?;
            if document.buf.readonly {
                return None;
            }
            let (first, last) = if let Some(collection) = self.collections.get(&self.current()) {
                let index = collection
                    .excerpts
                    .partition_point(|excerpt| excerpt.view_start <= view);
                let excerpt = collection.excerpts.get(index.checked_sub(1)?)?;
                (excerpt.start, excerpt.end)
            } else {
                (0, document.buf.len_bytes())
            };
            let mapping = Mapping {
                source: ByteOffset::new(at),
                view: ByteOffset::new(view),
                first: ByteOffset::new(first),
                last: ByteOffset::new(last),
            };
            if let Some(target) = sources.iter_mut().find(|target| target.document == source) {
                target.mappings.push(mapping);
            } else {
                sources.push(Source {
                    document: source,
                    revision: document.buf.revision(),
                    request: request(document, trigger),
                    mappings: vec![mapping],
                });
            }
        }
        Some(Batch { trigger, sources })
    }
}
