//! Owner-private storage. Creation permissions are established atomically, before
//! a second principal could retain a descriptor to a subsequently populated file.
mod ownership;
pub(crate) use ownership::ExclusiveLock;

use anyhow::{Result, ensure};
use std::{fs::File, path::Path};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported() -> anyhow::Error {
    std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "private storage requires Linux or macOS",
    )
    .into()
}

pub(crate) fn create_file(path: &Path) -> Result<File> {
    #[cfg(target_os = "macos")]
    let file = macos::create_file(path)?;
    #[cfg(target_os = "linux")]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        validate_file(&file)?;
        Ok(file)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = path;
        Err(unsupported())
    }
}

pub(crate) fn open_file(path: &Path, write: bool) -> Result<File> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        validate_file(&file)?;
        Ok(file)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (path, write);
        Err(unsupported())
    }
}

pub(crate) fn validate_file(file: &File) -> Result<()> {
    validate_file_links(file, false)
}

/// Inspect a disposable sidecar without retaining a descriptor for SQLite I/O.
/// SQLite may unlink its rollback journal between this open and validation.
pub(crate) fn check_sqlite_sidecar(path: &Path) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        validate_file_links(&file, true)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = path;
        Err(unsupported())
    }
}

fn validate_file_links(file: &File, allow_unlinked: bool) -> Result<()> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "private storage must be an ordinary file"
    );
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            (metadata.nlink() == 1 || (allow_unlinked && metadata.nlink() == 0))
                && metadata.mode() & 0o077 == 0,
            "private storage must be owner-only and singly linked"
        );
        #[cfg(target_os = "macos")]
        macos::validate_acl(file)?;
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = allow_unlinked;
        Err(unsupported())
    }
}

pub(crate) fn directory(path: &Path) -> Result<File> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)?;
        ensure!(
            directory.metadata()?.mode() & 0o077 == 0,
            "private directory must be owner-only"
        );
        #[cfg(target_os = "macos")]
        macos::validate_acl(&directory)?;
        Ok(directory)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = path;
        Err(unsupported())
    }
}

pub(crate) fn create_dir(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    macos::create_dir(path)?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::DirBuilderExt;
        // POSIX ACL grants are constrained by the requested group-class mask
        // and other bits. Retain default ACLs; the private parent stays 0700.
        std::fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        directory(path)?;
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = path;
        Err(unsupported())
    }
}

/// Parent ACL validation for SQLite-created sidecars. Parent mode/namespace
/// authority belongs to the caller; this does not claim to authenticate aliases.
pub(crate) fn parent_acl(path: &Path) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let parent = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(path)?;
        #[cfg(target_os = "macos")]
        macos::validate_parent_acl(&parent)?;
        // SQLite copies the validated 0600 database mode to its sidecars. On
        // Linux this also constrains inherited POSIX ACL grants; no xattr
        // inspection/removal is needed. Keep a real directory-open check.
        drop(parent);
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = path;
        Err(unsupported())
    }
}

pub(crate) struct TempDir(std::path::PathBuf);
impl TempDir {
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn tempdir_in(parent: &Path) -> Result<TempDir> {
    loop {
        let path = parent.join(format!(".x402-private-{}", uuid::Uuid::new_v4()));
        match create_dir(&path) {
            Ok(()) => return Ok(TempDir(path)),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn tempdir() -> Result<TempDir> {
    tempdir_in(&std::env::temp_dir())
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::{
        ffi::{CString, c_void},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
        ptr,
    };
    type Native = *mut c_void;
    unsafe extern "C" {
        fn acl_init(count: libc::c_int) -> Native;
        fn acl_free(acl: Native) -> libc::c_int;
        fn acl_get_fd(fd: libc::c_int) -> Native;
        fn acl_get_entry(acl: Native, entry: libc::c_int, result: *mut Native) -> libc::c_int;
        fn acl_get_tag_type(entry: Native, tag: *mut libc::c_int) -> libc::c_int;
        fn acl_get_flagset_np(acl: Native, result: *mut Native) -> libc::c_int;
        fn acl_add_flag_np(flags: Native, flag: libc::c_int) -> libc::c_int;
        fn acl_get_flag_np(flags: Native, flag: libc::c_int) -> libc::c_int;
        fn filesec_init() -> Native;
        fn filesec_free(security: Native);
        fn filesec_set_property(
            security: Native,
            property: libc::c_int,
            value: *const c_void,
        ) -> libc::c_int;
        fn openx_np(path: *const libc::c_char, flags: libc::c_int, security: Native)
        -> libc::c_int;
        fn mkdirx_np(path: *const libc::c_char, security: Native) -> libc::c_int;
    }
    struct Acl(Native);
    impl Drop for Acl {
        fn drop(&mut self) {
            unsafe {
                acl_free(self.0);
            }
        }
    }
    struct Security(Native);
    impl Drop for Security {
        fn drop(&mut self) {
            unsafe {
                filesec_free(self.0);
            }
        }
    }
    fn checked(result: libc::c_int) -> Result<()> {
        if result == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    fn security(mode: libc::mode_t) -> Result<Security> {
        let acl = Acl(unsafe { acl_init(0) });
        if acl.0.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut flags = ptr::null_mut();
        checked(unsafe { acl_get_flagset_np(acl.0, &mut flags) })?;
        checked(unsafe { acl_add_flag_np(flags, 1 << 17) })?; // ACL_FLAG_NO_INHERIT
        let security = unsafe { filesec_init() };
        if security.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        // filesec_free requires a non-null object, including on error paths.
        let security = Security(security);
        checked(unsafe {
            filesec_set_property(security.0, 4, (&mode as *const libc::mode_t).cast())
        })?;
        checked(unsafe { filesec_set_property(security.0, 5, (&acl.0 as *const Native).cast()) })?;
        Ok(security)
    }
    pub(super) fn create_file(path: &Path) -> Result<File> {
        let path = CString::new(path.as_os_str().as_bytes())?;
        let security = security(0o600)?;
        let fd = unsafe {
            openx_np(
                path.as_ptr(),
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_EXCL
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC,
                security.0,
            )
        };
        if fd == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    pub(super) fn create_dir(path: &Path) -> Result<()> {
        let path = CString::new(path.as_os_str().as_bytes())?;
        let security = security(0o700)?;
        checked(unsafe { mkdirx_np(path.as_ptr(), security.0) })
    }
    #[cfg(test)]
    #[test]
    fn invalid_descriptor_is_an_inspection_error() {
        let error = validate_acl_fd(-1, false).unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(libc::EBADF)
        );
    }
    pub(super) fn validate_acl(file: &File) -> Result<()> {
        validate_acl_fd(file.as_raw_fd(), false)
    }
    pub(super) fn validate_parent_acl(file: &File) -> Result<()> {
        validate_acl_fd(file.as_raw_fd(), true)
    }
    fn validate_acl_fd(fd: libc::c_int, inheriting_files_only: bool) -> Result<()> {
        let acl = Acl(unsafe { acl_get_fd(fd) });
        if acl.0.is_null() {
            let error = std::io::Error::last_os_error();
            // Darwin's descriptor getter returns ENOENT when FILESEC_ACL is
            // absent after successful fstatx_np. This is not a missing path;
            // every other retrieval error still fails closed.
            if error.raw_os_error() == Some(libc::ENOENT) {
                return Ok(());
            }
            return Err(error.into());
        }
        let mut entry = ptr::null_mut();
        let mut selector = 0; // ACL_FIRST_ENTRY; ACL_NEXT_ENTRY is -1 on Darwin.
        loop {
            let result = unsafe { acl_get_entry(acl.0, selector, &mut entry) };
            if result == -1 {
                let error = std::io::Error::last_os_error();
                // acl_get_fd supplied this ACL object; Darwin uses EINVAL for
                // exhaustion, including a newly created empty ACL. acl_valid
                // itself rejects empty ACLs and cannot distinguish this case.
                if error.raw_os_error() == Some(libc::EINVAL) {
                    return Ok(());
                }
                return Err(error.into());
            }
            let mut tag = 0;
            checked(unsafe { acl_get_tag_type(entry, &mut tag) })?;
            if tag == 1 && inheriting_files_only {
                let mut flags = ptr::null_mut();
                checked(unsafe { acl_get_flagset_np(entry, &mut flags) })?;
                let inherits = unsafe { acl_get_flag_np(flags, 1 << 5) }; // ACL_ENTRY_FILE_INHERIT
                checked(inherits)?;
                ensure!(
                    inherits == 0,
                    "SQLite parent must not have file-inheriting access-granting ACL entries"
                );
            } else {
                ensure!(
                    tag == 2,
                    "private storage must not have access-granting ACL entries"
                );
            }
            selector = -1;
        }
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;
    use std::{
        io::Write,
        os::unix::fs::{PermissionsExt, symlink},
    };

    #[test]
    fn private_reopen_rejects_symlinks_hardlinks_and_public_modes() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("private");
        create_file(&path)
            .unwrap()
            .write_all(b"public synthetic bytes")
            .unwrap();
        open_file(&path, false).unwrap();
        check_sqlite_sidecar(&path).unwrap();
        let alias = temporary.path().join("alias");
        symlink(&path, &alias).unwrap();
        assert!(open_file(&alias, false).is_err());
        assert!(check_sqlite_sidecar(&alias).is_err());
        std::fs::remove_file(alias).unwrap();
        let link = temporary.path().join("hardlink");
        std::fs::hard_link(&path, &link).unwrap();
        assert!(open_file(&path, false).is_err());
        assert!(check_sqlite_sidecar(&path).is_err());
        std::fs::remove_file(link).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(open_file(&path, false).is_err());
        assert!(check_sqlite_sidecar(&path).is_err());
        assert!(check_sqlite_sidecar(temporary.path()).is_err());
    }

    #[test]
    fn sqlite_sidecar_removal_preserves_strict_private_file_validation() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("state.sqlite-journal");
        let file = create_file(&path).unwrap();
        check_sqlite_sidecar(&path).unwrap();
        // Reproduce SQLite unlinking a journal after the inspector opens it.
        std::fs::remove_file(&path).unwrap();
        validate_file_links(&file, true).unwrap();
        assert!(validate_file(&file).is_err());
        check_sqlite_sidecar(&path).unwrap();
        // Removal does not excuse unsafe permissions on the inspected descriptor.
        file.set_permissions(std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(validate_file_links(&file, true).is_err());
    }

    #[test]
    fn sqlite_children_and_disposable_preflight_are_private() {
        let temporary = tempdir().unwrap();
        sqlite_files_are_private(temporary.path());
    }

    fn sqlite_files_are_private(directory: &Path) {
        let path = directory.join("state.sqlite");
        create_file(&path).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE test(value); INSERT INTO test VALUES(1);",
        )
        .unwrap();
        for name in ["state.sqlite", "state.sqlite-wal", "state.sqlite-shm"] {
            open_file(&directory.join(name), false).unwrap();
        }
        let snapshot = tempdir_in(directory).unwrap();
        let copied = snapshot.path().join("copy");
        create_file(&copied)
            .unwrap()
            .write_all(b"synthetic preflight copy")
            .unwrap();
        open_file(&copied, false).unwrap();
        let removed = snapshot.path().to_path_buf();
        drop(snapshot);
        assert!(!removed.exists());
        drop(db);
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "PRAGMA journal_mode=DELETE; BEGIN IMMEDIATE; INSERT INTO test VALUES(2);",
        )
        .unwrap();
        open_file(&directory.join("state.sqlite-journal"), false).unwrap();
        db.execute_batch("ROLLBACK;").unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_default_acls_remain_masked_for_private_files_and_sqlite() {
        use std::os::fd::AsRawFd;
        let owner = unsafe { libc::geteuid() };
        assert_ne!(
            owner, 0,
            "native denied-read control requires an ordinary user"
        );
        let other = if owner == 65534 { 65533 } else { 65534 };
        let parent = tempdir().unwrap();
        let parent_file = directory(parent.path()).unwrap();
        // Include a named non-owner grant as well as maximal group/other
        // defaults. Private creation must mask it without deleting the ACL.
        let mut acl = 2u32.to_le_bytes().to_vec();
        for (tag, id) in [
            (1u16, u32::MAX),
            (2, other),
            (4, u32::MAX),
            (16, u32::MAX),
            (32, u32::MAX),
        ] {
            acl.extend(tag.to_le_bytes());
            acl.extend(7u16.to_le_bytes());
            acl.extend(id.to_le_bytes());
        }
        let result = unsafe {
            libc::fsetxattr(
                parent_file.as_raw_fd(),
                c"system.posix_acl_default".as_ptr(),
                acl.as_ptr().cast(),
                acl.len(),
                0,
            )
        };
        assert_eq!(
            result,
            0,
            "native ACL fixture requires POSIX ACL support: {}",
            std::io::Error::last_os_error()
        );
        directory(parent.path()).unwrap();
        parent_acl(parent.path()).unwrap();
        let child = parent.path().join("private-child");
        create_dir(&child).unwrap();
        let child = directory(&child).unwrap();
        assert!(
            unsafe {
                libc::fgetxattr(
                    child.as_raw_fd(),
                    c"system.posix_acl_default".as_ptr(),
                    std::ptr::null_mut(),
                    0,
                )
            } > 0,
            "fixture must retain the inherited default ACL"
        );
        let file = parent.path().join("private-file");
        let file = create_file(&file).unwrap();
        assert!(
            unsafe {
                libc::fgetxattr(
                    file.as_raw_fd(),
                    c"system.posix_acl_access".as_ptr(),
                    std::ptr::null_mut(),
                    0,
                )
            } > 0,
            "fixture must include an actual named access ACL masked by mode"
        );
        let denied = parent.path().join("private-file");
        std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o0)).unwrap();
        assert_eq!(
            std::fs::read(&denied).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o600)).unwrap();
        sqlite_files_are_private(parent.path());
    }

    #[cfg(target_os = "macos")]
    fn grant_inherited_access(path: &Path) {
        assert!(
            std::process::Command::new("/bin/chmod")
                .arg("+a")
                .arg("everyone allow read,write,execute,file_inherit,directory_inherit")
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn sqlite_parent_allows_only_non_inheriting_grants() {
        let parent = tempdir().unwrap();
        assert!(
            std::process::Command::new("/bin/chmod")
                .arg("+a")
                .arg("everyone allow read,write,execute")
                .arg(parent.path())
                .status()
                .unwrap()
                .success()
        );
        assert!(
            directory(parent.path()).is_err(),
            "private objects still reject grants"
        );
        parent_acl(parent.path()).unwrap();
        sqlite_files_are_private(parent.path());
        let path = parent.path().join("state.sqlite");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o0)).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        grant_inherited_access(parent.path());
        assert!(parent_acl(parent.path()).is_err());
        // Positive control: bypassing the parent guard really exposes SQLite
        // sidecars, despite an atomically private primary database.
        let exposed = parent.path().join("exposed.sqlite");
        create_file(&exposed).unwrap();
        let db = rusqlite::Connection::open(&exposed).unwrap();
        db.execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE exposed(value); INSERT INTO exposed VALUES(1);",
        )
        .unwrap();
        for name in ["exposed.sqlite-wal", "exposed.sqlite-shm"] {
            let path = parent.path().join(name);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o0)).unwrap();
            assert!(
                std::fs::read(&path).is_ok(),
                "actual inherited SQLite ACL positive control"
            );
            assert!(open_file(&path, false).is_err());
            assert!(check_sqlite_sidecar(&path).is_err());
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        drop(db);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn inherited_acl_controls_and_atomic_creation() {
        // Mode 0000 makes the owner itself a fresh-read negative control. Root
        // bypasses that control; this fixture requires an ordinary native user.
        assert_ne!(
            unsafe { libc::geteuid() },
            0,
            "run ACL controls as an ordinary user"
        );
        let parent = tempdir().unwrap();
        let no_acl = parent.path().join("no-acl");
        create_file(&no_acl).unwrap();
        for path in [parent.path(), no_acl.as_path()] {
            assert!(
                std::process::Command::new("/bin/chmod")
                    .arg("-N")
                    .arg(path)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        directory(parent.path()).unwrap();
        open_file(&no_acl, false).unwrap();
        grant_inherited_access(parent.path());
        let exposed = parent.path().join("exposed");
        std::fs::write(&exposed, b"public ACL control").unwrap();
        std::fs::set_permissions(&exposed, std::fs::Permissions::from_mode(0o0)).unwrap();
        assert_eq!(std::fs::read(&exposed).unwrap(), b"public ACL control");
        std::fs::set_permissions(&exposed, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(open_file(&exposed, false).is_err());
        assert!(directory(parent.path()).is_err());
        assert!(parent_acl(parent.path()).is_err());

        let private = parent.path().join("private");
        let mut created = create_file(&private).unwrap();
        created.write_all(b"synthetic private control").unwrap();
        open_file(&private, false).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o0)).unwrap();
        assert_eq!(
            std::fs::read(&private).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).unwrap();
        let clean = parent.path().join("clean");
        create_dir(&clean).unwrap();
        directory(&clean).unwrap();
        let child = clean.join("child");
        std::fs::write(&child, b"ordinary child").unwrap();
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o0)).unwrap();
        assert_eq!(
            std::fs::read(&child).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
}
