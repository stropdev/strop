//! Pure captured platform context. Workers never change process-global environment.
use std::path::PathBuf;
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Environment {
    #[serde(with = "strop_core::path_serde::option")]
    pub home: Option<PathBuf>,
    #[serde(with = "strop_core::path_serde::option")]
    pub data_home: Option<PathBuf>,
}
impl Environment {
    pub fn capture() -> Self {
        Self {
            home: std::env::var_os("HOME").map(PathBuf::from),
            data_home: std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        }
    }
}
