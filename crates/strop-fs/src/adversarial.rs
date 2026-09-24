//! VF06 adversarial authority campaigns over the current local kernel
//! (plans/0057 §5 "Local files and operations"): symlinked ancestors,
//! concurrent modification between prepare and apply, permission/metadata
//! transitions, cooperative lock domains, reserved-namespace aliases and
//! native-byte names. Every refusal is asserted typed (FsFailureKind) and
//! effect-free on the objects it was meant to protect.
//!
//! Honest limits of this campaign: kernel-enforced EACCES paths are not
//! assertable when the lane runs as root (CAP_DAC_OVERRIDE), so permission
//! transitions are exercised through metadata re-observation (mode bits)
//! and the userspace lock-identity checks, both of which hold regardless
//! of euid. Cooperative locks are not CAS against nonparticipants; that
//! documented limit is not dressed up as a counterexample here.

use crate::guard::{self, NameLock, Parent};
use crate::local;
use crate::tests::{environment, intent, with_token};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use strop_workspace::operation::{FsFailure, FsFailureKind, OperationKind, StepOutcome};

fn refused(outcome: StepOutcome) -> FsFailure {
    match outcome {
        StepOutcome::Refused(failure) => failure,
        StepOutcome::Committed { .. } => panic!("expected a typed refusal, got a commit"),
        StepOutcome::Cancelled { detail } => {
            panic!("expected a typed refusal, got cancellation: {detail}")
        }
        StepOutcome::Unconfirmed { detail, .. } => {
            panic!("expected a typed refusal, got unconfirmed: {detail}")
        }
    }
}

fn prepare_failure(
    kind: OperationKind,
    source: Option<&Path>,
    destination: Option<&Path>,
) -> FsFailure {
    let root = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    with_token(|context, token| {
        local::prepare(
            &intent(kind, source, destination),
            false,
            &environment(&root),
            context,
            &token,
        )
        .expect_err("preparation must refuse")
    })
}

#[test]
fn symlinked_ancestors_resolve_but_the_final_entry_is_never_followed() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, root.path().join("link")).unwrap();
    with_token(|context, token| {
        // Creation through a symlinked ancestor lands in the real tree.
        let through = root.path().join("link/new.txt");
        let operation = local::prepare(
            &intent(OperationKind::CreateFile, None, Some(&through)),
            false,
            &environment(root.path()),
            context,
            &token,
        )
        .unwrap();
        assert!(matches!(
            local::execute(&operation, None, &[], context, &token),
            StepOutcome::Committed { .. }
        ));
        assert!(real.join("new.txt").is_file());
        // A symlink as the final entry is never followed: the current
        // kernel refuses link mutations with a typed Unsupported, leaving
        // the link and its target byte-identical.
        std::fs::write(real.join("target.txt"), "payload").unwrap();
        std::os::unix::fs::symlink(real.join("target.txt"), real.join("point")).unwrap();
        let failure = local::prepare(
            &intent(OperationKind::Remove, Some(&real.join("point")), None),
            false,
            &environment(root.path()),
            context,
            &token,
        )
        .expect_err("link mutations are refused, never followed");
        assert_eq!(failure.kind, FsFailureKind::Unsupported);
        assert!(real.join("point").is_symlink());
        assert_eq!(std::fs::read(real.join("target.txt")).unwrap(), b"payload");
    });
    // An occupied destination whose final entry is a symlink is occupied,
    // never followed and overwritten.
    let failure = prepare_failure(
        OperationKind::CreateFile,
        None,
        Some(&root.path().join("link")),
    );
    assert_eq!(failure.kind, FsFailureKind::Conflict);
    assert_eq!(std::fs::read(real.join("target.txt")).unwrap(), b"payload");
}

#[test]
fn source_changed_between_prepare_and_apply_is_a_typed_conflict() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, "prepared\n").unwrap();
    with_token(|context, token| {
        let operation = local::prepare(
            &intent(
                OperationKind::Rename,
                Some(&source),
                Some(&root.path().join("destination")),
            ),
            false,
            &environment(root.path()),
            context,
            &token,
        )
        .unwrap();
        // A nonparticipant rewrites the source between prepare and apply.
        std::fs::write(&source, "rewritten by a third party\n").unwrap();
        let failure = refused(local::execute(&operation, None, &[], context, &token));
        assert_eq!(failure.kind, FsFailureKind::Conflict);
        assert_eq!(
            std::fs::read(&source).unwrap(),
            b"rewritten by a third party\n"
        );
        assert!(!root.path().join("destination").exists());
    });
}

#[test]
fn parent_swapped_between_prepare_and_apply_is_a_typed_conflict() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    std::fs::create_dir(&parent).unwrap();
    std::fs::write(parent.join("file"), "contents\n").unwrap();
    with_token(|context, token| {
        let operation = local::prepare(
            &intent(
                OperationKind::Rename,
                Some(&parent.join("file")),
                Some(&parent.join("renamed")),
            ),
            false,
            &environment(root.path()),
            context,
            &token,
        )
        .unwrap();
        // The whole parent directory is replaced by a lookalike.
        std::fs::rename(&parent, root.path().join("parent-old")).unwrap();
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("file"), "contents\n").unwrap();
        let failure = refused(local::execute(&operation, None, &[], context, &token));
        assert_eq!(failure.kind, FsFailureKind::Conflict);
        assert!(
            !parent.join("renamed").exists(),
            "the swapped-in parent never receives the publication"
        );
    });
}

#[test]
fn symlink_ancestor_retarget_between_prepare_and_apply_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&first, &link).unwrap();
    with_token(|context, token| {
        let operation = local::prepare(
            &intent(
                OperationKind::CreateFile,
                None,
                Some(&link.join("created.txt")),
            ),
            false,
            &environment(root.path()),
            context,
            &token,
        )
        .unwrap();
        // The alias is silently retargeted between prepare and apply.
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&second, &link).unwrap();
        let failure = refused(local::execute(&operation, None, &[], context, &token));
        assert_eq!(failure.kind, FsFailureKind::Conflict);
        assert!(!first.join("created.txt").exists());
        assert!(!second.join("created.txt").exists());
    });
}

#[test]
fn permission_transition_between_prepare_and_apply_fails_closed() {
    // The kernel's EACCES paths are invisible to a root test lane, so the
    // transition asserted here is the mode-bit change itself: prepared
    // authority is bound to the observed metadata, and a permission
    // transition is a typed conflict, never a publication.
    let root = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    let source = root.path().join("locked-down");
    std::fs::write(&source, "owned\n").unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o644)).unwrap();
    with_token(|context, token| {
        let operation = local::prepare(
            &intent(OperationKind::Remove, Some(&source), None),
            false,
            &environment(root.path()),
            context,
            &token,
        )
        .unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o000)).unwrap();
        let failure = refused(local::execute(&operation, None, &[], context, &token));
        assert_eq!(failure.kind, FsFailureKind::Conflict);
        assert_eq!(
            std::fs::symlink_metadata(&source)
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o000,
            "the transitioned object is untouched"
        );
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o644)).unwrap();
    });
}

#[test]
fn operation_locks_are_per_name_domains_with_checked_identity() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    let lock_name = |name: &str| format!(".strop-lock-{:x}", Sha256::digest(name.as_bytes()));
    // Lock identity attacks are refused before any flock: wrong link count,
    // wrong mode, and non-regular stand-ins are all typed failures.
    let attack = std::fs::File::create(first.join(lock_name("victim"))).unwrap();
    std::fs::hard_link(
        first.join(lock_name("victim")),
        first.join(format!("{}-alias", lock_name("victim"))),
    )
    .unwrap();
    drop(attack);
    std::fs::write(first.join(lock_name("lax")), b"").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        first.join(lock_name("lax")),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    std::os::unix::fs::symlink(lock_name("victim"), first.join(lock_name("linked"))).unwrap();
    let parent = Parent::open(&first).unwrap();
    for (name, detail) in [
        ("victim", "hardlinked"),
        ("lax", "permissive"),
        ("linked", "symlinked"),
    ] {
        let failure = match NameLock::acquire(&parent, name.as_ref()) {
            Err(failure) => failure,
            Ok(_) => panic!("{detail} lock stand-in was accepted"),
        };
        assert!(
            matches!(
                failure.kind,
                FsFailureKind::Permission | FsFailureKind::Io | FsFailureKind::Conflict
            ),
            "{detail} lock stand-in is a typed refusal: {failure}"
        );
    }
    // The same name in two directories is two independent domains; a held
    // lock refuses a competing participant as Busy; release reopens the name.
    let other = Parent::open(&second).unwrap();
    let held = NameLock::acquire(&parent, "shared".as_ref()).unwrap();
    let _independent = NameLock::acquire(&other, "shared".as_ref()).unwrap();
    let competing = match NameLock::acquire(&parent, "shared".as_ref()) {
        Err(failure) => failure,
        Ok(_lock) => panic!("a competing participant acquired the held name"),
    };
    assert_eq!(competing.kind, FsFailureKind::Busy);
    held.release().unwrap();
    let reacquired = NameLock::acquire(&parent, "shared".as_ref()).unwrap();
    reacquired.release().unwrap();
    // The defeated stand-ins never poison their names either.
    std::fs::remove_file(first.join(lock_name("victim"))).unwrap();
    std::fs::remove_file(first.join(format!("{}-alias", lock_name("victim")))).unwrap();
    let clean = NameLock::acquire(&parent, "victim".as_ref()).unwrap();
    clean.release().unwrap();
}

#[test]
fn reserved_namespaces_refuse_case_and_normalization_aliases() {
    let root = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    for path in [
        PathBuf::from("relative/name"),
        root.join(".strop-lock-00"),
        root.join(".STROP-SAVE-abc"),
        // NFKC-foldable fullwidth spelling of the protected prefix.
        root.join(".\u{FF33}\u{FF34}\u{FF32}\u{FF2F}\u{FF30}-\u{FF46}\u{FF53}-x"),
        root.join("dir").join(".strop-fs-stage").join("child"),
    ] {
        let failure = guard::resolve(&path).expect_err("reserved or relative");
        assert_eq!(
            failure.kind,
            FsFailureKind::InvalidPath,
            "{path:?}: {failure}"
        );
    }
    let mut nul = root.clone().into_os_string();
    nul.push("/nul\0byte");
    let failure = guard::resolve(Path::new(&nul)).expect_err("NUL path");
    assert_eq!(failure.kind, FsFailureKind::InvalidPath);
}

#[test]
fn native_byte_and_control_names_roundtrip_exactly() {
    use std::os::unix::ffi::OsStrExt;
    let root = tempfile::tempdir().unwrap();
    let raw = std::ffi::OsStr::from_bytes(b"native-\xff\x07-name.bin");
    let destination = root.path().join(raw);
    with_token(|context, token| {
        let operation = local::prepare(
            &intent(OperationKind::CreateFile, None, Some(&destination)),
            false,
            &environment(root.path()),
            context,
            &token,
        )
        .unwrap();
        assert!(matches!(
            local::execute(&operation, None, &[], context, &token),
            StepOutcome::Committed { .. }
        ));
        // The stored spelling is exactly the requested bytes. Protocol
        // lock files are the kernel's own reserved namespace and linger
        // by design; everything else must be the created file alone.
        let names: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().as_encoded_bytes().to_vec())
            .filter(|name| !name.starts_with(b".strop-lock-"))
            .collect();
        assert_eq!(names, vec![b"native-\xff\x07-name.bin".to_vec()]);
        let removal = local::prepare(
            &intent(OperationKind::Remove, Some(&destination), None),
            false,
            &environment(root.path()),
            context,
            &token,
        )
        .unwrap();
        assert!(matches!(
            local::execute(&removal, None, &[], context, &token),
            StepOutcome::Committed { .. }
        ));
        assert!(
            std::fs::read_dir(root.path()).unwrap().all(|entry| entry
                .unwrap()
                .file_name()
                .as_encoded_bytes()
                .starts_with(b".strop-lock-")),
            "only reserved protocol locks remain"
        );
    });
}

#[test]
fn cancellation_before_publication_leaves_no_effect() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("never");
    let context = crate::ExecutionContext::native();
    let (token, handle) = strop_core::worker::CancelToken::standalone();
    let operation = local::prepare(
        &intent(OperationKind::CreateFile, None, Some(&destination)),
        false,
        &environment(root.path()),
        &context,
        &token,
    )
    .unwrap();
    handle.cancel(strop_core::worker::CancelReason::Superseded);
    assert!(matches!(
        local::execute(&operation, None, &[], &context, &token),
        StepOutcome::Cancelled { .. }
    ));
    assert!(!destination.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn cross_mount_rename_is_a_typed_unsupported_refusal() {
    // /dev/shm is a distinct tmpfs mount in the test lanes; the plan's
    // different-mounts case must produce the typed refusal, not a copy.
    let shm = Path::new("/dev/shm");
    assert!(
        shm.is_dir(),
        "the different-mount lane requires /dev/shm (tmpfs)"
    );
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("local-only");
    std::fs::write(&source, "payload\n").unwrap();
    let destination = shm.join(format!("strop-vf06-{}", std::process::id()));
    with_token(|context, token| {
        let operation = local::prepare(
            &intent(OperationKind::Rename, Some(&source), Some(&destination)),
            false,
            &environment(root.path()),
            context,
            &token,
        )
        .unwrap();
        let failure = refused(local::execute(&operation, None, &[], context, &token));
        assert_eq!(failure.kind, FsFailureKind::Unsupported);
        assert_eq!(std::fs::read(&source).unwrap(), b"payload\n");
        assert!(!destination.exists());
    });
}
