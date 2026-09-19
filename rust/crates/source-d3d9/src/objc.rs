//! The Objective-C runtime binding the device reaches Metal through.
//!
//! The device sends a few hundred distinct selectors, several of them tens of
//! thousands of times a frame, so this is built around one macro, [`msg!`],
//! that states a call's argument and return types where it is made and caches
//! its selector at the call site. Nothing here depends on a crate: the
//! workspace takes no external dependencies, and the runtime's C entry points
//! are all that is needed.

use std::ffi::{c_char, c_void, CStr};

/// An Objective-C object pointer.
pub type Id = *mut c_void;
/// A registered selector.
pub type Sel = *const c_void;

pub type NSUInteger = u64;
pub type NSInteger = i64;
pub type CGFloat = f64;

#[link(name = "objc")]
extern "C" {
    pub fn sel_registerName(name: *const c_char) -> Sel;
    pub fn objc_getClass(name: *const c_char) -> Id;
    pub fn objc_retain(object: Id) -> Id;
    pub fn objc_release(object: Id);
    pub fn objc_autoreleasePoolPush() -> *mut c_void;
    pub fn objc_autoreleasePoolPop(pool: *mut c_void);
    pub fn objc_msgSend();
}

#[link(name = "Metal", kind = "framework")]
extern "C" {
    pub fn MTLCreateSystemDefaultDevice() -> Id;
}

#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

#[link(name = "Foundation", kind = "framework")]
extern "C" {}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

/// Sends a message, caching the selector where the call is written.
///
/// `msg![ret: T; receiver, "selector:with:", a: A, b: B]`. The argument types
/// are stated rather than inferred because the runtime's entry point is
/// untyped: it is transmuted to the signature written here, and a wrong type
/// is a wrong calling convention rather than a compile error.
#[macro_export]
macro_rules! msg {
    (ret: $ret:ty; $obj:expr, $sel:literal $(, $arg:expr => $ty:ty)* $(,)?) => {{
        static SEL: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
            std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
        let mut sel = SEL.load(std::sync::atomic::Ordering::Relaxed);
        if sel.is_null() {
            sel = $crate::objc::sel_registerName(concat!($sel, "\0").as_ptr().cast()) as *mut std::ffi::c_void;
            SEL.store(sel, std::sync::atomic::Ordering::Relaxed);
        }
        let send: unsafe extern "C" fn($crate::objc::Id, $crate::objc::Sel $(, $ty)*) -> $ret =
            std::mem::transmute($crate::objc::objc_msgSend as *const ());
        send($obj, sel as $crate::objc::Sel $(, $arg)*)
    }};
    ($obj:expr, $sel:literal $(, $arg:expr => $ty:ty)* $(,)?) => {
        $crate::msg![ret: (); $obj, $sel $(, $arg => $ty)*]
    };
}

/// Looks up a class by name. Null when the framework that defines it is not
/// loaded.
pub fn class(name: &CStr) -> Id {
    // SAFETY: the pointer is a valid NUL-terminated string for the call.
    unsafe { objc_getClass(name.as_ptr()) }
}

/// Drains everything autoreleased during its lifetime. Pools are per thread
/// and must nest, which a scoped value guarantees.
pub struct AutoreleasePool(*mut c_void);

impl AutoreleasePool {
    pub fn new() -> Self {
        // SAFETY: pushing a pool is always valid and is balanced in Drop.
        Self(unsafe { objc_autoreleasePoolPush() })
    }
}

impl Default for AutoreleasePool {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        // SAFETY: pops exactly the pool this value pushed.
        unsafe { objc_autoreleasePoolPop(self.0) };
    }
}

/// An owned reference to an Objective-C object, released on drop.
pub struct Owned(Id);

impl Owned {
    /// Takes over a reference the caller already holds, which is what `new`,
    /// `alloc` and `copy` methods return.
    ///
    /// # Safety
    /// `object` must be null or carry a reference to release exactly once.
    pub unsafe fn from_owned(object: Id) -> Option<Self> {
        if object.is_null() {
            None
        } else {
            Some(Self(object))
        }
    }

    /// Retains an object the caller does not own, such as an autoreleased one.
    ///
    /// # Safety
    /// `object` must be null or a live object.
    pub unsafe fn retain(object: Id) -> Option<Self> {
        if object.is_null() {
            None
        } else {
            // SAFETY: the caller guarantees the object is live.
            Some(Self(unsafe { objc_retain(object) }))
        }
    }

    pub fn id(&self) -> Id {
        self.0
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: holds exactly one unreleased reference to a non-null object.
        unsafe { objc_release(self.0) };
    }
}

/// An `NSString` built from UTF-8 text.
pub fn ns_string(text: &str) -> Option<Owned> {
    let bytes = text.as_bytes();
    // SAFETY: `alloc` then `initWithBytes:length:encoding:` is NSString's
    // designated way to copy a buffer; 4 is NSUTF8StringEncoding, and the
    // bytes are valid for the call, which copies them.
    unsafe {
        let alloc = msg![ret: Id; class(c"NSString"), "alloc"];
        let string = msg![ret: Id; alloc, "initWithBytes:length:encoding:",
            bytes.as_ptr() => *const u8, bytes.len() as NSUInteger => NSUInteger, 4 => NSUInteger];
        Owned::from_owned(string)
    }
}

/// The UTF-8 text of an `NSString`, or of an object's description.
///
/// # Safety
/// `string` must be null or a live `NSString`.
pub unsafe fn string_from_ns(string: Id) -> String {
    if string.is_null() {
        return String::new();
    }
    // SAFETY: `UTF8String` returns a NUL-terminated buffer that lives as long
    // as the string does, which is at least this call.
    unsafe {
        let text = msg![ret: *const c_char; string, "UTF8String"];
        if text.is_null() {
            String::new()
        } else {
            CStr::from_ptr(text).to_string_lossy().into_owned()
        }
    }
}

/// The localized description of an `NSError`.
///
/// # Safety
/// `error` must be null or a live `NSError`.
pub unsafe fn error_text(error: Id) -> String {
    if error.is_null() {
        return String::from("unknown error");
    }
    // SAFETY: every NSError answers `localizedDescription` with an NSString.
    unsafe { string_from_ns(msg![ret: Id; error, "localizedDescription"]) }
}
