//! The engine's log, reached without linking against it.
//!
//! `tier0` exports `Warning` and `Error` as unmangled variadic C functions. A
//! Rust module looks them up in the running process, so it has no C++ library
//! to link, and outside the engine, as in a test, the line goes to stderr.

use std::ffi::CString;

/// Writes one line to the engine's console.
pub fn message(message: &str) {
    emit(c"Msg", message);
}

/// Writes one line to the engine's console as a warning.
pub fn warning(message: &str) {
    emit(c"Warning", message);
}

/// The engine's fatal error: inside the engine this reports the message and
/// ends the process, as the C++ it replaces did. Outside it, where there is
/// no `tier0`, it returns after writing the line, so callers still need a way
/// to carry on.
pub fn error(message: &str) {
    emit(c"Error", message);
}

fn emit(function: &std::ffi::CStr, message: &str) {
    let Ok(line) = CString::new(message) else {
        return;
    };
    match imp::log_fn(function) {
        // SAFETY: both are `void f( const char *fmt, ... )`, and the format
        // consumes exactly the one string passed. The type is variadic
        // because on Apple arm64 variadic arguments travel differently.
        Some(log) => unsafe { log(c"%s".as_ptr(), line.as_ptr()) },
        None => eprint!("{message}"),
    }
}

#[cfg(unix)]
mod imp {
    use std::ffi::{c_char, c_int, c_void, CStr};

    pub type LogFn = unsafe extern "C" fn(format: *const c_char, ...);

    extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    const RTLD_LAZY: c_int = 1;
    #[cfg(target_vendor = "apple")]
    const RTLD_NOLOAD: c_int = 0x10;
    #[cfg(target_os = "freebsd")]
    const RTLD_NOLOAD: c_int = 0x2000;
    #[cfg(not(any(target_vendor = "apple", target_os = "freebsd")))]
    const RTLD_NOLOAD: c_int = 0x4;
    #[cfg(target_vendor = "apple")]
    const TIER0: &CStr = c"libtier0.dylib";
    #[cfg(not(target_vendor = "apple"))]
    const TIER0: &CStr = c"libtier0.so";

    /// dyld answers a bare file name by searching its library paths and then
    /// asking whether the file it found is loaded. A process that loaded
    /// `tier0` from anywhere else, as one started without those paths set
    /// does, is told no. The images it did load are listed by path.
    #[cfg(target_vendor = "apple")]
    unsafe fn loaded_by_path() -> *mut c_void {
        // SAFETY: the index is in range and dyld owns the returned string.
        unsafe {
            for index in 0..super::images::_dyld_image_count() {
                let path = super::images::_dyld_get_image_name(index);
                if !path.is_null()
                    && CStr::from_ptr(path)
                        .to_bytes()
                        .ends_with(b"/libtier0.dylib")
                {
                    return dlopen(path, RTLD_LAZY | RTLD_NOLOAD);
                }
            }
        }
        std::ptr::null_mut()
    }

    #[cfg(not(target_vendor = "apple"))]
    unsafe fn loaded_by_path() -> *mut c_void {
        std::ptr::null_mut()
    }

    /// Resolved against `tier0` only, and only if the process already has it:
    /// asking every loaded image for a name as plain as `Error` could find
    /// someone else's.
    pub fn log_fn(name: &CStr) -> Option<LogFn> {
        // SAFETY: a lookup of an already loaded image and a symbol in it.
        let symbol = unsafe {
            let mut tier0 = dlopen(TIER0.as_ptr(), RTLD_LAZY | RTLD_NOLOAD);
            if tier0.is_null() {
                tier0 = loaded_by_path();
            }
            if tier0.is_null() {
                return None;
            }
            dlsym(tier0, name.as_ptr())
        };
        // SAFETY: tier0 defines both symbols with this signature.
        (!symbol.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, LogFn>(symbol) })
    }
}

#[cfg(all(unix, target_vendor = "apple"))]
mod images {
    use std::ffi::c_char;

    extern "C" {
        pub fn _dyld_image_count() -> u32;
        pub fn _dyld_get_image_name(index: u32) -> *const c_char;
    }
}

#[cfg(not(unix))]
mod imp {
    use std::ffi::{c_char, CStr};

    pub type LogFn = unsafe extern "C" fn(format: *const c_char, ...);

    pub fn log_fn(_: &CStr) -> Option<LogFn> {
        None
    }
}
