//! The smallest Objective-C runtime binding the renderer needs.
//!
//! Metal is an Objective-C API, so reaching it from Rust means sending
//! selectors. Rather than take a dependency for that, this declares the three
//! runtime entry points actually used and wraps them so the rest of the
//! renderer never touches `objc_msgSend` directly.

use std::ffi::{c_char, c_void, CStr};

/// An Objective-C object pointer. Opaque on purpose: nothing outside this
/// module may dereference one.
pub type Id = *mut c_void;

/// A registered selector.
///
/// The pointer is read by the runtime, not by Rust: it reaches `objc_msgSend`
/// as the second argument of the transmuted call, which the compiler cannot
/// see through.
#[derive(Clone, Copy)]
pub struct Sel(#[allow(dead_code)] *const c_void);

// SAFETY: a selector is an interned, immutable, process-wide constant that the
// runtime never frees, so sharing one across threads is sound.
unsafe impl Send for Sel {}
unsafe impl Sync for Sel {}

#[link(name = "objc")]
extern "C" {
    fn sel_registerName(name: *const c_char) -> *const c_void;
    fn objc_getClass(name: *const c_char) -> Id;
    fn objc_retain(object: Id) -> Id;
    fn objc_release(object: Id);
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);

    // Declared without a signature because every call site transmutes it to
    // the signature of the method being sent, which is how the runtime's
    // calling convention works.
    fn objc_msgSend();
}

/// Registers a selector from a NUL-terminated name.
pub fn selector(name: &CStr) -> Sel {
    // SAFETY: the pointer comes from a CStr, so it is a valid NUL-terminated
    // C string for the duration of the call, which is all the runtime needs.
    Sel(unsafe { sel_registerName(name.as_ptr()) })
}

/// The runtime's untyped message entry point, for the call shapes that are
/// used once and transmuted at the call site rather than wrapped here.
pub fn msg_send_ptr() -> *const () {
    objc_msgSend as *const ()
}

/// Takes ownership of an object the caller does not already own, such as one
/// returned autoreleased from a factory method.
///
/// # Safety
/// `object` must be a live object or null.
pub unsafe fn retain(object: Id) -> Option<Owned> {
    if object.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees the object is live, and retaining it
    // gives this side a reference to balance in `Owned`'s drop.
    unsafe {
        objc_retain(object);
        Owned::from_owned(object)
    }
}

/// Looks up a class by name, or null when the framework is not loaded.
pub fn class(name: &CStr) -> Id {
    // SAFETY: the pointer comes from a CStr, so it is a valid NUL-terminated
    // C string for the duration of the call.
    unsafe { objc_getClass(name.as_ptr()) }
}

/// Drains everything autoreleased during its lifetime.
///
/// Metal's factory methods hand back autoreleased objects, so without a pool
/// in scope they would accumulate for the life of the thread.
pub struct AutoreleasePool(*mut c_void);

impl AutoreleasePool {
    pub fn new() -> Self {
        // SAFETY: pushing a pool is always valid and is balanced in Drop.
        Self(unsafe { objc_autoreleasePoolPush() })
    }
}

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        // SAFETY: this pops exactly the pool this value pushed, and Rust's
        // drop order guarantees pools nest correctly.
        unsafe { objc_autoreleasePoolPop(self.0) };
    }
}

/// Sends a selector that takes no arguments and returns an object.
///
/// # Safety
/// `receiver` must be a live object that responds to `sel` with an object
/// return, or null.
pub unsafe fn send_id(receiver: Id, sel: Sel) -> Id {
    if receiver.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: transmuting the runtime's untyped entry point to the signature
    // of the method being sent is the documented way to call it; the caller
    // guarantees the method matches this signature.
    let send: extern "C" fn(Id, Sel) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    send(receiver, sel)
}

/// Sends a selector taking one object argument and returning an object.
///
/// # Safety
/// As [`send_id`], and `argument` must match the method's parameter.
pub unsafe fn send_id_with_id(receiver: Id, sel: Sel, argument: Id) -> Id {
    if receiver.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: as above, with the signature of the method being sent.
    let send: extern "C" fn(Id, Sel, Id) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    send(receiver, sel, argument)
}

/// Sends a selector taking one integer argument and returning an object.
///
/// # Safety
/// As [`send_id`], and `argument` must match the method's parameter.
pub unsafe fn send_id_with_usize(receiver: Id, sel: Sel, argument: u64) -> Id {
    if receiver.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: as above, with the signature of the method being sent.
    let send: extern "C" fn(Id, Sel, u64) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    send(receiver, sel, argument)
}

/// Sends a selector that takes no arguments and returns nothing.
///
/// # Safety
/// As [`send_id`], for a method with no return value.
pub unsafe fn send_void(receiver: Id, sel: Sel) {
    if receiver.is_null() {
        return;
    }
    // SAFETY: as above, with the signature of the method being sent.
    let send: extern "C" fn(Id, Sel) = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    send(receiver, sel)
}

/// Sends a selector taking one object argument and returning nothing.
///
/// # Safety
/// As [`send_id`], and `argument` must match the method's parameter.
pub unsafe fn send_void_with_id(receiver: Id, sel: Sel, argument: Id) {
    if receiver.is_null() {
        return;
    }
    // SAFETY: as above, with the signature of the method being sent.
    let send: extern "C" fn(Id, Sel, Id) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    send(receiver, sel, argument)
}

/// Sends a selector taking one integer argument and returning nothing.
///
/// # Safety
/// As [`send_id`], and `argument` must match the method's parameter.
pub unsafe fn send_void_with_usize(receiver: Id, sel: Sel, argument: u64) {
    if receiver.is_null() {
        return;
    }
    // SAFETY: as above, with the signature of the method being sent.
    let send: extern "C" fn(Id, Sel, u64) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    send(receiver, sel, argument)
}

/// Sends a selector taking a C string and returning an object, which is how
/// `NSString` is built from UTF-8 bytes.
///
/// # Safety
/// As [`send_id`], and `text` must be a NUL-terminated C string.
pub unsafe fn send_id_with_cstr(receiver: Id, sel: Sel, text: *const c_char) -> Id {
    if receiver.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: as above, with the signature of the method being sent.
    let send: extern "C" fn(Id, Sel, *const c_char) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    send(receiver, sel, text)
}

/// Sends a selector taking one argument passed by value and returning nothing.
///
/// # Safety
/// As [`send_id`], and `T` must be the method's parameter type laid out the
/// way C lays it out, so that the platform calling convention matches.
pub unsafe fn send_void_with_value<T>(receiver: Id, sel: Sel, argument: T) {
    if receiver.is_null() {
        return;
    }
    // SAFETY: as above. Rust passes a `repr(C)` type in an `extern "C"` call
    // exactly as C would, which is what the runtime entry point expects.
    let send: extern "C" fn(Id, Sel, T) = unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    send(receiver, sel, argument)
}

/// Sends a selector that takes no arguments and returns a C string.
///
/// # Safety
/// `receiver` must be a live object that responds to `sel` with a C string
/// return, or null.
pub unsafe fn send_cstr(receiver: Id, sel: Sel) -> *const c_char {
    if receiver.is_null() {
        return std::ptr::null();
    }
    // SAFETY: as above, with the return type of the method being sent.
    let send: extern "C" fn(Id, Sel) -> *const c_char =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    send(receiver, sel)
}

/// Owns a reference to an Objective-C object and releases it on drop.
pub struct Owned(Id);

impl Owned {
    /// Takes ownership of an object that is already retained for the caller,
    /// which is what the Create rule and `new`-prefixed methods return.
    ///
    /// # Safety
    /// `object` must be null or carry a reference the caller is responsible
    /// for releasing exactly once.
    pub unsafe fn from_owned(object: Id) -> Option<Self> {
        if object.is_null() {
            return None;
        }
        Some(Self(object))
    }

    pub fn as_id(&self) -> Id {
        self.0
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the constructors guarantee this holds exactly one reference
        // that has not been released, and the pointer is non-null.
        unsafe { objc_release(self.0) };
    }
}

// SAFETY: the objects the renderer holds this way are Metal devices and queues,
// which Apple documents as safe to use from multiple threads; the reference
// count itself is maintained atomically by the runtime.
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}
