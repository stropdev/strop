//! Compilation is deterministic; native cancellation and program caches never
//! cross the recording boundary. Replay reconstructs the checked query itself.
use super::CompiledQuery;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct QueryRef<'a> {
    source: &'a str,
    whole_word: bool,
}
#[derive(Deserialize)]
struct QueryRecord {
    source: String,
    whole_word: bool,
}
impl Serialize for CompiledQuery {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        QueryRef {
            source: self.source(),
            whole_word: self.whole_word(),
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for CompiledQuery {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let record = QueryRecord::deserialize(deserializer)?;
        Self::compile(&record.source, record.whole_word).map_err(serde::de::Error::custom)
    }
}
