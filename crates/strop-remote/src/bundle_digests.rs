//! VF08 digest-pinning campaign (plans/0057 §5, "The Python supervisor/editor
//! boundary is mandatory"): the shipped helper bundle bytes are reproducible
//! from the tree parts and bound to the recorded sha256 digests in
//! `verification/helper-digests.json`. A helper edit without a re-pinned,
//! re-reviewed digest record fails this campaign. The pins bind bytes and
//! protocol versions; semantic correspondence lives in the save/filesystem/
//! exec seam tests, not here.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn pins() -> Value {
    let text = std::fs::read_to_string(root().join("verification/helper-digests.json"))
        .expect("verification/helper-digests.json is the VF08 digest record");
    serde_json::from_str(&text).unwrap()
}

fn sha256(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// Reproduce one bundle from its recorded tree parts and check both the
/// reproduction and the shipped constant against the recorded digests.
fn assert_bundle(pins: &Value, name: &str, shipped: &str) {
    let record = &pins["bundles"][name];
    let separator = record["separator"].as_str().unwrap_or("\n");
    let mut assembled = String::new();
    for (index, part) in record["parts"].as_array().unwrap().iter().enumerate() {
        let part = part.as_str().unwrap();
        let bytes =
            std::fs::read(root().join(part)).unwrap_or_else(|error| panic!("{part}: {error}"));
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(
            sha256(&text),
            record["part_sha256"][part].as_str().unwrap(),
            "{part} drifted from its VF08 pin"
        );
        if index > 0 {
            assembled.push_str(separator);
        }
        assembled.push_str(&text);
    }
    assert_eq!(
        assembled, shipped,
        "the shipped {name} bundle is not the documented concatenation of its parts"
    );
    assert_eq!(
        sha256(shipped),
        record["bundle_sha256"].as_str().unwrap(),
        "the shipped {name} bundle digest drifted; re-review and re-pin \
         verification/helper-digests.json"
    );
}

#[test]
fn save_bundle_bytes_reproduce_from_the_tree_and_match_their_pin() {
    assert_bundle(&pins(), "save", crate::save::protocol::HELPER);
}

#[test]
fn filesystem_bundle_bytes_reproduce_from_the_tree_and_match_their_pin() {
    assert_bundle(&pins(), "filesystem", crate::filesystem::HELPER);
}

#[test]
fn supervisor_source_matches_its_pin_and_embedded_block() {
    let pins = pins();
    let record = &pins["bundles"]["supervisor"];
    let source = crate::exec::supervisor::SOURCE;
    // The embedded program is authored inline: the shipped constant must be
    // verbatim text inside its owning source file, and the pair must match
    // the recorded digest.
    let file =
        std::fs::read_to_string(root().join(record["embedded_in"].as_str().unwrap())).unwrap();
    assert!(
        file.contains(source),
        "the supervisor constant is not verbatim in its source file"
    );
    assert_eq!(
        sha256(source),
        record["bundle_sha256"].as_str().unwrap(),
        "the embedded supervisor source drifted; re-review and re-pin \
         verification/helper-digests.json"
    );
    assert_eq!(
        pins["protocol_versions"]["save"].as_u64().unwrap(),
        1,
        "save helper protocol version pin"
    );
    assert_eq!(
        pins["protocol_versions"]["exec_spec"].as_u64().unwrap(),
        u64::from(crate::exec::spec::VERSION),
        "exec spec version pin"
    );
}
