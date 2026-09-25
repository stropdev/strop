//! The container image's platform (0058 WK08): deployment binds the
//! worker artifact to the endpoint's exact target, and the target's
//! arch half is an image fact — `docker image inspect` reports it.
//! (The libc half is deployment policy: WK05's Linux worker is the
//! static musl artifact, which runs on any Linux image regardless of
//! the image's own libc; that mapping lives in the deploy crate, never
//! guessed here.)

use crate::engine::{capture, stderr_tail, EngineRef};
use crate::ContainerError;
use strop_core::worker::CancelToken;

/// Budget for one image inspect, mirroring the engine's metadata
/// commands.
const INSPECT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);
const INSPECT_LIMIT: u64 = 4 * 1024 * 1024;

/// The image's declared platform, exactly as the engine recorded it
/// (e.g. `linux`/`amd64`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlatform {
    pub os: String,
    pub arch: String,
}

#[derive(serde::Deserialize)]
struct ImageRecord {
    #[serde(rename = "Os")]
    os: String,
    #[serde(rename = "Architecture")]
    architecture: String,
}

/// Inspect one image reference (as `ContainerIdentity.image` carries
/// it) for its platform. A missing image or a malformed answer is a
/// typed failure, never a guessed architecture.
pub fn image_platform(
    engine: &EngineRef,
    image: &str,
    token: &CancelToken,
) -> Result<ImagePlatform, ContainerError> {
    if image.is_empty() || image.starts_with('-') || image.contains([' ', '\n', '\0']) {
        return Err(ContainerError::PoisonedName {
            name: image.to_string(),
        });
    }
    let output = capture(
        engine,
        &["image", "inspect", image, "--format", "json"],
        INSPECT_LIMIT,
        INSPECT_DEADLINE,
        token,
    )?;
    if output.code != Some(0) {
        return Err(ContainerError::Io {
            detail: format!(
                "docker image inspect {image} failed: {}",
                stderr_tail(&output.stderr)
            ),
        });
    }
    // `--format json` emits one object; the array form arrives without
    // the flag on older CLIs — both decode through the same record.
    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim();
    let record: ImageRecord = if let Some(inner) = trimmed
        .strip_prefix('[')
        .and_then(|body| body.strip_suffix(']'))
    {
        serde_json::from_str(inner.trim()).map_err(|error| ContainerError::Protocol {
            detail: format!("image inspect {image}: {error}"),
        })?
    } else {
        serde_json::from_str(trimmed).map_err(|error| ContainerError::Protocol {
            detail: format!("image inspect {image}: {error}"),
        })?
    };
    Ok(ImagePlatform {
        os: record.os,
        arch: record.architecture,
    })
}
