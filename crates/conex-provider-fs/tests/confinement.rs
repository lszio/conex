//! Confinement: no traversal, symlink escape, non-regular files or absolute paths.
use std::fs;

use conex_provider_fs::FsRoot;

#[test]
fn read_cannot_escape_through_parent_segments() {
    let temp = tempfile::tempdir().unwrap();
    let root = FsRoot::open(temp.path()).unwrap();
    assert!(root.read("../secret.md", 262144).is_err());
}

#[test]
fn parent_segment_in_the_middle_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.md"), "ok").unwrap();
    let root = FsRoot::open(temp.path()).unwrap();
    assert!(root.read("sub/../a.md", 262144).is_err());
}

#[test]
fn absolute_path_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = FsRoot::open(temp.path()).unwrap();
    assert!(root.read("/etc/passwd", 262144).is_err());
}

#[test]
fn directory_read_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("sub")).unwrap();
    let root = FsRoot::open(temp.path()).unwrap();
    assert!(root.read("sub", 262144).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.md"), "top secret").unwrap();
    symlink(
        outside.path().join("secret.md"),
        temp.path().join("escape.md"),
    )
    .unwrap();
    let root = FsRoot::open(temp.path()).unwrap();
    assert!(root.read("escape.md", 262144).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_component_escape_is_rejected() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.md"), "top secret").unwrap();
    symlink(outside.path(), temp.path().join("link")).unwrap();
    let root = FsRoot::open(temp.path()).unwrap();
    assert!(root.read("link/secret.md", 262144).is_err());
}
