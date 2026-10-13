//! Test-only SQLite VFS: delegate real I/O except one explicitly armed write or
//! sync on an exact temporary database path. Never registered as the default.
use super::*;
use rusqlite::ffi;
use std::{
    collections::HashMap,
    ffi::{CStr, c_char, c_int, c_void},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy)]
pub(crate) enum Failure {
    Write,
    Sync,
}
struct Injection {
    kind: Failure,
    fired: AtomicBool,
}
struct File {
    original: usize,
    patched: usize,
    path: String,
}
static FILES: Mutex<Option<HashMap<usize, File>>> = Mutex::new(None);
static FAULTS: Mutex<Option<HashMap<String, Arc<Injection>>>> = Mutex::new(None);
static ORIGINAL: OnceLock<usize> = OnceLock::new();
const NAME: &CStr = c"treazury-test-io-failure";

pub(crate) struct Fault {
    path: String,
    injection: Arc<Injection>,
}
impl Fault {
    pub(crate) fn fired(&self) -> bool {
        self.injection.fired.load(Ordering::SeqCst)
    }
}
impl Drop for Fault {
    fn drop(&mut self) {
        FAULTS.lock().unwrap().as_mut().unwrap().remove(&self.path);
    }
}
pub(crate) fn arm(path: &Path, kind: Failure) -> Fault {
    let path = path
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(path.file_name().unwrap())
        .to_str()
        .unwrap()
        .to_owned();
    let injection = Arc::new(Injection {
        kind,
        fired: AtomicBool::new(false),
    });
    assert!(
        FAULTS
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(path.clone(), injection.clone())
            .is_none()
    );
    Fault { path, injection }
}
pub(crate) fn install(store: &mut Store) {
    ORIGINAL.get_or_init(|| unsafe {
        // SQLite owns the default VFS for process lifetime. The registered copy
        // also lives for process lifetime, and uses the native file layout.
        let default = ffi::sqlite3_vfs_find(std::ptr::null());
        assert!(!default.is_null());
        let mut vfs = Box::new(std::ptr::read(default));
        vfs.zName = NAME.as_ptr();
        vfs.pNext = std::ptr::null_mut();
        vfs.xOpen = Some(open);
        assert_eq!(
            ffi::sqlite3_vfs_register(Box::into_raw(vfs), 0),
            ffi::SQLITE_OK
        );
        default as usize
    });
    let path = store.db.path().unwrap().to_owned();
    let db = Connection::open_with_flags_and_vfs(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        NAME.to_str().unwrap(),
    )
    .unwrap();
    configure(&db).unwrap();
    evidence::initialize_runtime(&db).unwrap();
    store.db = db;
}
unsafe extern "C" fn open(
    _: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    output: *mut c_int,
) -> c_int {
    unsafe {
        let default = *ORIGINAL.get().unwrap() as *mut ffi::sqlite3_vfs;
        let code = ((*default).xOpen.unwrap())(default, name, file, flags, output);
        if code != ffi::SQLITE_OK {
            return code;
        }
        let original = (*file).pMethods;
        let mut methods = Box::new(std::ptr::read(original));
        methods.xWrite = Some(write);
        methods.xSync = Some(sync);
        methods.xClose = Some(close);
        let patched = Box::into_raw(methods);
        let path = if name.is_null() {
            String::new()
        } else {
            CStr::from_ptr(name).to_string_lossy().into_owned()
        };
        FILES
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(
                file as usize,
                File {
                    original: original as usize,
                    patched: patched as usize,
                    path,
                },
            );
        (*file).pMethods = patched;
        code
    }
}
fn operation(
    file: *mut ffi::sqlite3_file,
    kind: Failure,
) -> (*const ffi::sqlite3_io_methods, bool) {
    let files = FILES.lock().unwrap();
    let entry = &files.as_ref().unwrap()[&(file as usize)];
    let faults = FAULTS.lock().unwrap();
    let fail = faults
        .as_ref()
        .and_then(|f| f.get(&entry.path))
        .is_some_and(|f| {
            matches!(
                (kind, f.kind),
                (Failure::Write, Failure::Write) | (Failure::Sync, Failure::Sync)
            ) && !f.fired.swap(true, Ordering::SeqCst)
        });
    (entry.original as *const ffi::sqlite3_io_methods, fail)
}
unsafe extern "C" fn write(
    file: *mut ffi::sqlite3_file,
    data: *const c_void,
    size: c_int,
    offset: i64,
) -> c_int {
    let (original, fail) = operation(file, Failure::Write);
    if fail {
        ffi::SQLITE_IOERR_WRITE
    } else {
        unsafe { ((*original).xWrite.unwrap())(file, data, size, offset) }
    }
}
unsafe extern "C" fn sync(file: *mut ffi::sqlite3_file, flags: c_int) -> c_int {
    let (original, fail) = operation(file, Failure::Sync);
    if fail {
        ffi::SQLITE_IOERR_FSYNC
    } else {
        unsafe { ((*original).xSync.unwrap())(file, flags) }
    }
}
unsafe extern "C" fn close(file: *mut ffi::sqlite3_file) -> c_int {
    let entry = FILES
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .remove(&(file as usize))
        .unwrap();
    unsafe {
        (*file).pMethods = entry.original as *const ffi::sqlite3_io_methods;
        let result = ((*(*file).pMethods).xClose.unwrap())(file);
        drop(Box::from_raw(entry.patched as *mut ffi::sqlite3_io_methods));
        result
    }
}
