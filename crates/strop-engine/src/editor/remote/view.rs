//! A remote view's content contract is distinct from file identity and placement.
use strop_remote::{ReadLimit, ReadSelection, RemoteOffset};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RemoteView {
    Snapshot(ReadSelection),
    Follow(ReadLimit),
}
impl Default for RemoteView {
    fn default() -> Self {
        Self::Snapshot(ReadSelection::Full)
    }
}
impl RemoteView {
    pub fn selection(&self) -> ReadSelection {
        match self {
            Self::Snapshot(selection) => *selection,
            Self::Follow(limit) => ReadSelection::Tail(*limit),
        }
    }
    pub fn follow_limit(&self) -> Option<ReadLimit> {
        match self {
            Self::Follow(limit) => Some(*limit),
            Self::Snapshot(_) => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ViewError {
    #[error("byte counts must be nonnegative decimal integers")]
    Number,
    #[error("a range is START:BYTES")]
    Range,
    #[error(transparent)]
    Limit(#[from] strop_remote::ReadLimitError),
}
pub(crate) fn offset(text: &str) -> Result<RemoteOffset, ViewError> {
    Ok(RemoteOffset::new(number(text)?))
}
pub fn limit(text: &str) -> Result<ReadLimit, ViewError> {
    Ok(ReadLimit::new(number(text)?)?)
}
pub fn range(text: &str) -> Result<ReadSelection, ViewError> {
    let (start, length) = text.split_once(':').ok_or(ViewError::Range)?;
    Ok(ReadSelection::Range {
        start: offset(start)?,
        length: limit(length)?,
    })
}
fn number(text: &str) -> Result<u64, ViewError> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ViewError::Number);
    }
    text.parse().map_err(|_| ViewError::Number)
}
