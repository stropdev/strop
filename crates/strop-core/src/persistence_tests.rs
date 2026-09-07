use crate::Buffer;

#[test]
#[cfg(unix)]
fn predictable_staging_symlink_cannot_redirect_a_save() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("private.txt");
    let sentinel = directory.path().join("unrelated.txt");
    std::fs::write(&file, "secret\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&sentinel, "untouched").unwrap();
    let planted = directory
        .path()
        .join(format!(".strop-tmp-{}-private.txt", std::process::id()));
    symlink(&sentinel, &planted).unwrap();
    let mut buffer = Buffer::open(&file).unwrap();
    buffer.edit().insert(0, "changed ").unwrap();
    let receipt = buffer.prepare_save(None, false).unwrap().execute().unwrap();
    assert!(buffer.accept_save(receipt));
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "untouched");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "changed secret\n");
    assert!(!std::fs::symlink_metadata(&file)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
