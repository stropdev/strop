#!/usr/bin/env python3
"""VF07 model-fleet correspondence anchors (plans/0057 §5, SSH/SFTP/remote).

Confirms the calibrated TLA+ fleet still describes the current shipped code.
For each model this checks:

- the model file's sha256 equals the digest recorded at the last VF07 review
  (a model edit forces a mapping re-review, silently or not);
- every production anchor named in the model's reviewed contract still exists
  in the current tree (a renamed/removed anchor fails here, not in TLC).

This binds correspondence, not semantics: TLC runs live in the `model` gate
(specs/ssh-gate.sh, remote-gate.sh, save-gate.sh); the semantic review notes
live in the VERDICT table below. Run: python3 verification/check_model_anchors.py
"""

from __future__ import annotations

import hashlib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# VF07 drift review (2026-09-24, HEAD 703257e "Re-freeze the verification
# candidate", gates ssh/remote/save re-run green in the strop-model image):
#
# - The only strop-remote changes since model calibration (2a568d1, 0.19.0)
#   are the address-module move to strop-workspace (import-only in wire.rs,
#   session.rs, pool.rs, client.rs, exec.rs, run.rs), the new streaming API
#   (exec/stream.rs, additive), the save helper's refactor onto protected.py's
#   PathScope plus its NFS silly-rename cleanup fix, and the new
#   filesystem/* bundle (new surface, correspondence-tested in the test lane;
#   no wire-model claims over it).
# - The editor side moved from crates/strop to strop-engine with one rename
#   (finish_remote_refresh -> finish_refresh); OpenKey admission, cancel_open,
#   open_fresh and focus_epoch freshness survive unchanged.
# - The 0058 worker cutover did not touch strop-remote; the Python supervisor,
#   pool and wire codec are byte-identical to the calibrated revisions except
#   as above. No semantic drift found.
FLEET = {
    "specs/SftpWire.tla": {
        "sha256": "8b2f2c96d3cf24aeee1228d5f42338b3d3153db432c44aed519cce39c1c419f7",
        "anchors": {
            "crates/strop-remote/src/transport/wire.rs": [
                "const MAX_PACKET: usize = 256 * 1024",
                "const MAX_SNAPSHOT",
                "const READ_CHUNK",
                "checked_add",
                "with_capacity",
                "SFTP response belongs to a different request",
            ],
            "crates/strop-remote/src/transport/session.rs": [
                "is_cancelled",
                "async fn transfer",
            ],
        },
    },
    "specs/RemoteRead.tla": {
        "sha256": "7d091d6afc39175195e3988575c8dbd6497d82b5b621f84f6e654db8feb7787e",
        "anchors": {
            "crates/strop-engine/src/editor/io.rs": [
                "pub struct OpenKey",
                "fn request_target",
                "fn open_fresh",
                "focus_epoch",
                "finish_refresh",
            ],
            "crates/strop-core/src/process.rs": [
                "register_cancel_resource",
                "pub fn wait",
            ],
            "crates/strop-remote/src/transport/session.rs": [
                "async fn transfer",
            ],
            "crates/strop-engine/src/editor/io/navigation.rs": [
                "fn cancel_open",
            ],
        },
    },
    "specs/RemoteProcess.tla": {
        "sha256": "332dda9696affb51f8006c29c861e1723a16ba52721d028b0e670ec336ec037d",
        "anchors": {
            "crates/strop-remote/src/exec/supervisor.rs": [
                "MARK = b'STROP-SUP-v1'",
                "os.killpg(pid, sig)",
                "RELAY_CAP = 4 * 1024 * 1024",
                "CANCELLED = 251",
                "FAULT = 250",
            ],
            "crates/strop-remote/src/exec/run.rs": [
                "pub(super) fn classify",
                "STDERR_TAIL",
            ],
            "crates/strop-core/src/process.rs": [
                "register_cancel_resource",
            ],
        },
    },
    "specs/RemoteWorkspace.tla": {
        "sha256": "2cf770b08de01455e1003c610b8f8f42a1b3aade144c2289dc7707543a9e6cb6",
        "anchors": {
            "crates/strop-remote/src/pool.rs": [
                "const QUEUE_CAPACITY: usize = 32",
                "struct EndpointHandle",
                "next_epoch",
                "Weak<EndpointHandle>",
                "sync_channel(QUEUE_CAPACITY)",
            ],
            "crates/strop-engine/src/editor/remote/mod.rs": [
                "fn remote_window_complete",
            ],
            "crates/strop-engine/src/editor/lsp/state.rs": [
                "fn lsp_reply_fresh",
            ],
        },
    },
    "specs/RemoteSave.tla": {
        "sha256": "cb642cf758d7a0572ac13ae55a518cb8e5c244d70cca4022d2bc0c20cb016284",
        "anchors": {
            "crates/strop-remote/src/save/helper.py": [
                "commit_started = True",
                "os.replace(",
                "STAGE_PREFIX = b'.strop-save-'",
                "raise Refusal('conflict'",
            ],
            "crates/strop-remote/src/protected.py": [
                "class PathScope",
                "def reserved(",
                "def lock_identity(",
                "LOCK_PREFIX = b'.strop-lock-'",
            ],
            "crates/strop-remote/src/save/protocol.rs": [
                "REPLY_LIMIT",
                "deny_unknown_fields",
            ],
        },
    },
}


def main() -> int:
    failures = []
    for model, record in FLEET.items():
        digest = hashlib.sha256((ROOT / model).read_bytes()).hexdigest()
        if digest != record["sha256"]:
            failures.append(
                f"{model}: digest {digest} differs from the VF07-reviewed "
                f"{record['sha256']} — re-review the production mapping"
            )
        for source, anchors in record["anchors"].items():
            text_path = ROOT / source
            if not text_path.is_file():
                failures.append(f"{model}: anchor file missing: {source}")
                continue
            text = text_path.read_text()
            for anchor in anchors:
                if anchor not in text:
                    failures.append(f"{model}: anchor {anchor!r} gone from {source}")
    if failures:
        for failure in failures:
            print(f"FAIL: {failure}")
        return 1
    anchors = sum(len(a) for r in FLEET.values() for a in r["anchors"].values())
    print(
        f"VF07 fleet correspondence: {len(FLEET)} models, {anchors} production "
        "anchors, all digests and anchors current"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
