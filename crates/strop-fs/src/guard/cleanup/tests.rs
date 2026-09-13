use super::*;
use crate::guard::NameLock;
use crate::tests::with_token;

fn fixture() -> (tempfile::TempDir, Parent, Parent, OsString) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("directory");
    std::fs::create_dir(&path).unwrap();
    let parent = Parent::open(root.path()).unwrap();
    let directory = Parent::open(&path).unwrap();
    let lock = NameLock::acquire(&directory, OsStr::new("old-child")).unwrap();
    lock.release().unwrap();
    let name = std::fs::read_dir(&path)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name();
    (root, parent, directory, name)
}

#[test]
fn completed_lock_ownership_does_not_survive_an_inherited_descriptor() {
    let (_root, _parent, directory, _name) = fixture();
    let lock = NameLock::acquire(&directory, OsStr::new("old-child")).unwrap();
    // dup shares the open-file description exactly as a fork inheritance does.
    let inherited = lock.file.try_clone().unwrap();
    lock.release().unwrap();
    let next = NameLock::acquire(&directory, OsStr::new("old-child"));
    assert!(next.is_ok(), "completed operation retained its flock");
    next.unwrap().release().unwrap();
    drop(inherited);
}

#[test]
fn an_active_lock_is_never_unlinked() {
    let (_root, parent, directory, name) = fixture();
    let held = NameLock::acquire(&directory, OsStr::new("old-child")).unwrap();
    with_token(|token| {
        let approved = observation::stat(&directory.path).unwrap().unwrap();
        let captured =
            DirectoryLocks::capture(&parent, OsStr::new("directory"), &approved, &token).unwrap();
        let error = captured
            .prune(&parent, OsStr::new("directory"), &token)
            .err()
            .unwrap();
        assert_eq!(error.kind, FsFailureKind::Busy);
        assert!(directory.path.join(name).exists());
    });
    held.release().unwrap();
}

#[test]
fn replacement_of_a_captured_lock_is_not_adopted_for_cleanup() {
    let (root, parent, directory, name) = fixture();
    with_token(|token| {
        let approved = observation::stat(&directory.path).unwrap().unwrap();
        let captured =
            DirectoryLocks::capture(&parent, OsStr::new("directory"), &approved, &token).unwrap();
        std::fs::rename(directory.path.join(&name), root.path().join("retired-lock")).unwrap();
        let replacement = NameLock::acquire(&directory, OsStr::new("old-child")).unwrap();
        replacement.release().unwrap();
        let error = captured
            .prune(&parent, OsStr::new("directory"), &token)
            .err()
            .unwrap();
        assert_eq!(error.kind, FsFailureKind::Conflict);
        assert!(directory.path.join(name).exists());
    });
}

#[test]
fn lock_shaped_user_data_and_unknown_nfs_entries_are_preserved() {
    let (_root, parent, directory, name) = fixture();
    with_token(|token| {
        std::fs::write(directory.path.join(&name), "must remain").unwrap();
        let approved = observation::stat(&directory.path).unwrap().unwrap();
        assert!(
            DirectoryLocks::capture(&parent, OsStr::new("directory"), &approved, &token).is_err()
        );
        assert_eq!(
            std::fs::read(directory.path.join(&name)).unwrap(),
            b"must remain"
        );
        std::fs::write(directory.path.join(&name), "").unwrap();
        std::fs::write(directory.path.join(".nfs-unowned"), "not ours").unwrap();
        let approved = observation::stat(&directory.path).unwrap().unwrap();
        assert!(
            DirectoryLocks::capture(&parent, OsStr::new("directory"), &approved, &token).is_err()
        );
        assert_eq!(
            std::fs::read(directory.path.join(".nfs-unowned")).unwrap(),
            b"not ours"
        );
        assert!(directory.path.join(name).exists());
    });
}

#[test]
fn new_arrivals_are_not_rescanned_or_deleted() {
    let (_root, parent, directory, name) = fixture();
    with_token(|token| {
        let approved = observation::stat(&directory.path).unwrap().unwrap();
        let captured =
            DirectoryLocks::capture(&parent, OsStr::new("directory"), &approved, &token).unwrap();
        std::fs::write(directory.path.join("arrival"), "keep").unwrap();
        let (_held, count) = captured
            .prune(&parent, OsStr::new("directory"), &token)
            .unwrap();
        assert_eq!(count, 1);
        assert!(!directory.path.join(name).exists());
        assert!(rustix::fs::unlinkat(&parent.file, "directory", AtFlags::REMOVEDIR).is_err());
        assert_eq!(
            std::fs::read(directory.path.join("arrival")).unwrap(),
            b"keep"
        );
    });
}
