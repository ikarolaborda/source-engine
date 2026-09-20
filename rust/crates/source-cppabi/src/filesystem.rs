//! `IBaseFileSystem` as a C++ module implements it, called from Rust.
//!
//! This is the transitional direction: a Rust module reading through the
//! filesystem module while that is still C++. Only the calls a whole-file
//! read needs are bound.

use crate::slot;
use std::ffi::{c_char, c_int, c_uint, c_void, CStr};
use std::ptr::NonNull;

/// `BASEFILESYSTEM_INTERFACE_VERSION`, which yields an `IBaseFileSystem *`.
pub const BASE_FILESYSTEM_INTERFACE: &CStr = c"VBaseFileSystem011";
/// `FILESYSTEM_INTERFACE_VERSION`, which yields an `IFileSystem *`.
pub const FILESYSTEM_INTERFACE: &CStr = c"VFileSystem022";
/// Where `IBaseFileSystem` sits inside `IFileSystem`, which derives from
/// `IAppSystem` first: past that base's one table pointer.
pub const BASE_IN_FILESYSTEM: usize = size_of::<*const c_void>();

/// `IAppSystem::QueryInterface`, in the table an `IFileSystem *` points at.
const QUERY_INTERFACE: usize = 2;
// Indices into `IBaseFileSystem`'s table, in declaration order.
const READ: usize = 0;
const WRITE: usize = 1;
const OPEN: usize = 2;
const CLOSE: usize = 3;
const FILE_EXISTS: usize = 10;
const GET_FILE_TIME: usize = 13;
/// `Size( FileHandle_t )`, the first of two overloads. MSVC emits adjacent
/// overloads in reverse, which puts it second there; that ordering is the
/// documented rule and has not been checked against that compiler here.
#[cfg(not(windows))]
const SIZE_OF_HANDLE: usize = 6;
#[cfg(windows)]
const SIZE_OF_HANDLE: usize = 7;

type FileHandle = *mut c_void;
type ReadFn = unsafe extern "C" fn(*mut c_void, *mut c_void, c_int, FileHandle) -> c_int;
type OpenFn =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char) -> FileHandle;
type CloseFn = unsafe extern "C" fn(*mut c_void, FileHandle);
type SizeFn = unsafe extern "C" fn(*mut c_void, FileHandle) -> c_uint;
type QueryInterfaceFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;
type WriteFn = unsafe extern "C" fn(*mut c_void, *const c_void, c_int, FileHandle) -> c_int;
type ExistsFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> bool;
type FileTimeFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i64;

/// A borrowed `IBaseFileSystem *`.
#[derive(Debug, Clone, Copy)]
pub struct BaseFileSystem(NonNull<c_void>);

// SAFETY: the pointer names a process-lifetime singleton whose methods the
// engine already calls from several threads.
unsafe impl Send for BaseFileSystem {}

impl BaseFileSystem {
    /// # Safety
    ///
    /// `object` must be null or an `IBaseFileSystem *` that outlives every use
    /// of the returned value.
    pub unsafe fn from_raw(object: *mut c_void) -> Option<Self> {
        NonNull::new(object).map(Self)
    }

    /// Steps from an `IFileSystem *` to its `IBaseFileSystem` base the way
    /// C++ code outside the class would: by asking the object, which answers
    /// `QueryInterface` for that name with the adjusted pointer. An
    /// implementation that does not answer still has the base at
    /// [`BASE_IN_FILESYSTEM`].
    ///
    /// # Safety
    ///
    /// `object` must be null or an `IFileSystem *` with the same lifetime.
    pub unsafe fn from_filesystem(object: *mut c_void) -> Option<Self> {
        let object = NonNull::new(object)?;
        // SAFETY: IFileSystem's primary base is IAppSystem, whose third
        // virtual function is QueryInterface.
        let base = unsafe {
            let query: QueryInterfaceFn = slot(object.as_ptr(), QUERY_INTERFACE);
            query(object.as_ptr(), BASE_FILESYSTEM_INTERFACE.as_ptr())
        };
        // SAFETY: the base subobject lies inside the object at that offset.
        NonNull::new(base)
            .or_else(|| Some(unsafe { object.byte_add(BASE_IN_FILESYSTEM) }))
            .map(Self)
    }

    /// Reads a whole file through the search paths of `path_id`, or `None`
    /// when it cannot be opened. A short read yields the bytes that arrived.
    pub fn read_file(&self, name: &CStr, path_id: &CStr) -> Option<Vec<u8>> {
        let this = self.0.as_ptr();
        // SAFETY: `this` is a live IBaseFileSystem per `from_raw`, and each
        // signature is the one declared at that index.
        unsafe {
            let open: OpenFn = slot(this, OPEN);
            let size: SizeFn = slot(this, SIZE_OF_HANDLE);
            let read: ReadFn = slot(this, READ);
            let close: CloseFn = slot(this, CLOSE);

            let handle = open(this, name.as_ptr(), c"rb".as_ptr(), path_id.as_ptr());
            if handle.is_null() {
                return None;
            }
            let mut bytes = vec![0u8; size(this, handle) as usize];
            let mut filled = 0;
            while filled < bytes.len() {
                let wanted = (bytes.len() - filled).min(c_int::MAX as usize) as c_int;
                let got = read(this, bytes[filled..].as_mut_ptr().cast(), wanted, handle);
                if got <= 0 {
                    break;
                }
                filled += got as usize;
            }
            close(this, handle);
            bytes.truncate(filled);
            Some(bytes)
        }
    }

    /// Whether the file is present on any search path of `path_id`.
    pub fn file_exists(&self, name: &CStr, path_id: &CStr) -> bool {
        let this = self.0.as_ptr();
        // SAFETY: `this` is a live IBaseFileSystem and the signature is the
        // one declared at that index.
        unsafe {
            let exists: ExistsFn = slot(this, FILE_EXISTS);
            exists(this, name.as_ptr(), path_id.as_ptr())
        }
    }

    /// The file's modification time, or zero when there is no such file.
    pub fn file_time(&self, name: &CStr, path_id: &CStr) -> i64 {
        let this = self.0.as_ptr();
        // SAFETY: as above.
        unsafe {
            let time: FileTimeFn = slot(this, GET_FILE_TIME);
            time(this, name.as_ptr(), path_id.as_ptr())
        }
    }

    /// Writes a whole file, returning whether every byte was taken.
    pub fn write_file(&self, name: &CStr, path_id: &CStr, bytes: &[u8]) -> bool {
        let this = self.0.as_ptr();
        // SAFETY: as above; the handle is closed on every path out.
        unsafe {
            let open: OpenFn = slot(this, OPEN);
            let write: WriteFn = slot(this, WRITE);
            let close: CloseFn = slot(this, CLOSE);
            let handle = open(this, name.as_ptr(), c"wb".as_ptr(), path_id.as_ptr());
            if handle.is_null() {
                return false;
            }
            let mut written = 0;
            while written < bytes.len() {
                let wanted = (bytes.len() - written).min(c_int::MAX as usize) as c_int;
                let count = write(this, bytes[written..].as_ptr().cast(), wanted, handle);
                if count <= 0 {
                    break;
                }
                written += count as usize;
            }
            close(this, handle);
            written == bytes.len()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Object, VTable};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A stand-in with `IBaseFileSystem`'s shape: seventeen slots, of which the
    /// four bound above are real and any other would be a wrong index.
    #[repr(C)]
    struct Methods {
        slots: [*const c_void; 17],
    }

    static CONTENT: &[u8] = b"VSIF-contents";
    static CURSOR: AtomicUsize = AtomicUsize::new(0);
    static CLOSED: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn read(
        _: *mut c_void,
        out: *mut c_void,
        size: c_int,
        _: FileHandle,
    ) -> c_int {
        let start = CURSOR.load(Ordering::SeqCst);
        // Hand the file over five bytes at a time to exercise the loop.
        let count = (CONTENT.len() - start).min(size as usize).min(5);
        // SAFETY: the caller passes a buffer of at least `size` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(CONTENT[start..].as_ptr(), out.cast::<u8>(), count)
        };
        CURSOR.store(start + count, Ordering::SeqCst);
        count as c_int
    }

    unsafe extern "C" fn open(
        _: *mut c_void,
        name: *const c_char,
        options: *const c_char,
        path_id: *const c_char,
    ) -> FileHandle {
        // SAFETY: the caller passes terminated strings.
        let (name, options, path_id) = unsafe {
            (
                CStr::from_ptr(name),
                CStr::from_ptr(options),
                CStr::from_ptr(path_id),
            )
        };
        if name == c"scenes/scenes.image" && options == c"rb" && path_id == c"GAME" {
            CURSOR.store(0, Ordering::SeqCst);
            std::ptr::dangling_mut::<u64>().cast()
        } else {
            std::ptr::null_mut()
        }
    }

    unsafe extern "C" fn close(_: *mut c_void, _: FileHandle) {
        CLOSED.fetch_add(1, Ordering::SeqCst);
    }

    unsafe extern "C" fn size(_: *mut c_void, _: FileHandle) -> c_uint {
        CONTENT.len() as c_uint
    }

    unsafe extern "C" fn wrong_slot() {
        panic!("called a slot that is not bound");
    }

    static TABLE: VTable<Methods> = VTable::new(Methods {
        slots: {
            let mut slots = [wrong_slot as *const c_void; 17];
            slots[READ] = read as *const c_void;
            slots[OPEN] = open as *const c_void;
            slots[CLOSE] = close as *const c_void;
            slots[SIZE_OF_HANDLE] = size as *const c_void;
            slots
        },
    });
    static OBJECT: Object<Methods> = Object::new(&TABLE);

    #[test]
    fn reads_a_whole_file_through_the_bound_slots() {
        // SAFETY: the stand-in has the table shape the binding expects.
        let filesystem = unsafe { BaseFileSystem::from_raw(OBJECT.as_interface()) }.unwrap();
        assert_eq!(
            filesystem
                .read_file(c"scenes/scenes.image", c"GAME")
                .unwrap(),
            CONTENT
        );
        assert_eq!(CLOSED.load(Ordering::SeqCst), 1);
        assert!(filesystem
            .read_file(c"scenes/other.image", c"GAME")
            .is_none());
        assert_eq!(CLOSED.load(Ordering::SeqCst), 1);
        // SAFETY: null is allowed and yields nothing.
        assert!(unsafe { BaseFileSystem::from_raw(std::ptr::null_mut()) }.is_none());
    }
}
