//! Unix refusal rules at the real treasury open/status boundaries.
use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

#[test]
fn unsafe_files_are_rejected_without_touching_sentinels() {
    for name in [
        "key",
        "owner.lock",
        "state.sqlite",
        "state.sqlite-wal",
        "state.sqlite-shm",
        "state.sqlite-journal",
    ] {
        for kind in [
            "symlink",
            "dangling",
            "hardlink",
            "directory",
            "permissions",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("state");
            let key = tmp.path().join("key");
            let store = Store::create(&dir, &key, 1, b"snapshot").unwrap();
            let id = store.id.clone();
            drop(store);
            let path = if name == "key" {
                key.clone()
            } else {
                dir.join(name)
            };
            if path.exists() {
                fs::remove_file(&path).unwrap();
            }
            let sentinel = tmp.path().join("sentinel");
            fs::write(&sentinel, b"untouched").unwrap();
            fs::set_permissions(&sentinel, fs::Permissions::from_mode(0o600)).unwrap();
            let missing = tmp.path().join("missing");
            match kind {
                "symlink" => symlink(&sentinel, &path).unwrap(),
                "dangling" => symlink(&missing, &path).unwrap(),
                "hardlink" => fs::hard_link(&sentinel, &path).unwrap(),
                "directory" => fs::create_dir(&path).unwrap(),
                "permissions" => {
                    fs::write(&path, b"unsafe").unwrap();
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
                }
                _ => unreachable!(),
            }
            assert!(Store::open(&dir, &key, &id).is_err(), "{name} {kind}");
            if name.starts_with("state.sqlite") {
                assert!(status(&dir).is_err(), "read-only {name} {kind}");
            }
            assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");
            assert!(!missing.exists());
        }
    }
}

#[test]
fn key_lengths_directory_permissions_and_parent_aliases() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("parent");
    fs::create_dir(&parent).unwrap();
    let dir = parent.join("state");
    let key = parent.join("key");
    let store = Store::create(&dir, &key, 1, b"snapshot").unwrap();
    let id = store.id.clone();
    let bytes = fs::read(&key).unwrap();
    drop(store);
    for length in [0, 31, 33, 64] {
        fs::write(&key, vec![0; length]).unwrap();
        assert!(Store::open(&dir, &key, &id).is_err());
    }
    fs::write(&key, bytes).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Store::open(&dir, &key, &id).is_err());
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let alias = tmp.path().join("alias");
    symlink(&parent, &alias).unwrap();
    let opened = Store::open(&alias.join("../parent/state"), &alias.join("key"), &id).unwrap();
    assert_eq!(opened.snapshot().unwrap().1.as_slice(), b"snapshot");
    assert!(Store::open(&dir, &key, &id).is_err());
    drop(opened);
    let final_alias = parent.join("state-alias");
    symlink(&dir, &final_alias).unwrap();
    assert!(Store::open(&final_alias, &key, &id).is_err());
}

#[test]
#[ignore = "invoked by ownership_is_exclusive_across_processes"]
fn ownership_child() {
    let dir = std::path::PathBuf::from(std::env::var_os("TREAZURY_TEST_OWNER_DIR").unwrap());
    let error = match Store::open(&dir.join("state"), &dir.join("key"), "unused") {
        Ok(_) => panic!("second process acquired treasury"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("state_in_use"));
}

#[test]
fn ownership_is_exclusive_across_processes() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::create(
        &tmp.path().join("state"),
        &tmp.path().join("key"),
        1,
        b"snapshot",
    )
    .unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child.env_clear().env("TREAZURY_TEST_OWNER_DIR", tmp.path());
    for (key, value) in
        std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("LLVM_"))
    {
        child.env(key, value);
    }
    let output = child
        .args([
            "--ignored",
            "--exact",
            "rotation::store::filesystem_tests::ownership_child",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let id = store.id.clone();
    drop(store);
    Store::open(&tmp.path().join("state"), &tmp.path().join("key"), &id).unwrap();
}
