//! The C++ object model at Source's module boundary, written out by hand so
//! that a module can be Rust while its neighbours are still C++.
//!
//! Modules find each other through `CreateInterface` and talk through abstract
//! classes, so what crosses the boundary is a pointer to an object whose first
//! word points into a table of functions. [`VTable`] and [`Object`] lay that
//! out for an interface a Rust module implements, [`slot`] calls into one a
//! C++ module implements, and [`appsystem`], [`filesystem`] and [`tier0`] hold
//! the pieces every module needs.
//!
//! The layout is the Itanium ABI's, which clang and GCC follow on every
//! platform this engine builds with them. Each table here carries the slot
//! order `clang -Xclang -fdump-vtable-layouts` prints for the real header, and
//! `rust/tests/scenefilecache_differential.cpp` asserts the same numbers from
//! C++ member pointers. None of these interfaces declares a virtual
//! destructor; one that did would own two slots where it is declared.
//!
//! Member functions are `extern "C"` with the object first. That is the C++
//! member convention on every 64-bit target and on 32-bit Itanium targets, and
//! not on 32-bit Windows, where members are `thiscall`.

pub mod appsystem;
pub mod filesystem;
pub mod tier0;

use std::ffi::{c_char, c_int, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// `CreateInterfaceFn`.
pub type CreateInterfaceFn =
    unsafe extern "C" fn(name: *const c_char, return_code: *mut c_int) -> *mut c_void;

/// `IFACE_OK`.
pub const IFACE_OK: c_int = 0;
/// `IFACE_FAILED`.
pub const IFACE_FAILED: c_int = 1;

/// A virtual table as the compiler emits it: the two words that precede the
/// address an object points at, then the functions.
#[repr(C)]
pub struct VTable<M> {
    offset_to_top: isize,
    /// Read only by `dynamic_cast` and `typeid`, which nothing applies to an
    /// interface fetched through `CreateInterface`.
    rtti: *const c_void,
    methods: M,
}

impl<M> VTable<M> {
    pub const fn new(methods: M) -> Self {
        Self {
            offset_to_top: 0,
            rtti: std::ptr::null(),
            methods,
        }
    }

    /// The address a C++ object of this class stores in its first word.
    pub const fn address(&'static self) -> *const M {
        &raw const self.methods
    }
}

// SAFETY: a table is immutable once built and holds only function addresses.
unsafe impl<M> Sync for VTable<M> {}

/// The part of a C++ object the boundary sees: its table pointer. State lives
/// beside it in the Rust module rather than behind it, because these
/// interfaces are singletons.
#[repr(C)]
pub struct Object<M: 'static> {
    vptr: *const M,
}

impl<M> Object<M> {
    pub const fn new(table: &'static VTable<M>) -> Self {
        Self {
            vptr: table.address(),
        }
    }

    /// The pointer `CreateInterface` hands to C++.
    pub const fn as_interface(&'static self) -> *mut c_void {
        std::ptr::from_ref(self).cast_mut().cast()
    }
}

// SAFETY: the object is one immutable pointer to an immutable table.
unsafe impl<M> Sync for Object<M> {}

/// Reads entry `index` of the table a C++ object points at.
///
/// # Safety
///
/// `object` must point at a live C++ object whose class has at least
/// `index + 1` virtual functions, and `F` must be the `extern "C"` signature,
/// object first, of the one declared at `index`.
pub unsafe fn slot<F: Copy>(object: *mut c_void, index: usize) -> F {
    const { assert!(size_of::<F>() == size_of::<*const c_void>()) };
    // SAFETY: per the contract the first word is a table of at least that
    // many function addresses.
    unsafe {
        let table = *object.cast::<*const F>();
        *table.add(index)
    }
}

/// The body of an exported `CreateInterface` for a module with a fixed set of
/// interfaces. Names match exactly, as `CreateInterfaceInternal` matches them.
///
/// # Safety
///
/// `name` must be null or a NUL-terminated string, and `return_code` null or
/// writable.
pub unsafe fn create_interface(
    interfaces: &[(&CStr, *mut c_void)],
    name: *const c_char,
    return_code: *mut c_int,
) -> *mut c_void {
    let found = if name.is_null() {
        None
    } else {
        // SAFETY: non-null and terminated per the contract.
        let name = unsafe { CStr::from_ptr(name) };
        interfaces
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, object)| *object)
    };
    if !return_code.is_null() {
        // SAFETY: non-null and writable per the contract.
        unsafe {
            *return_code = if found.is_some() {
                IFACE_OK
            } else {
                IFACE_FAILED
            };
        }
    }
    found.unwrap_or(std::ptr::null_mut())
}

/// Runs the body of a function C++ calls. A panic must not unwind into C++
/// frames, so it becomes `fallback` and one line in the log.
pub fn guard<R>(fallback: R, body: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_| {
        tier0::warning("Rust module panicked inside a C++ interface call\n");
        fallback
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    struct Methods {
        first: unsafe extern "C" fn(*mut c_void) -> c_int,
        second: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    }

    unsafe extern "C" fn first(_: *mut c_void) -> c_int {
        7
    }

    unsafe extern "C" fn second(_: *mut c_void, value: c_int) -> c_int {
        value * 2
    }

    static TABLE: VTable<Methods> = VTable::new(Methods { first, second });
    static OBJECT: Object<Methods> = Object::new(&TABLE);

    #[test]
    fn an_object_points_past_the_two_word_prefix() {
        let table = std::ptr::from_ref(&TABLE) as usize;
        assert_eq!(
            TABLE.address() as usize,
            table + 2 * size_of::<*const c_void>()
        );
        // SAFETY: the object was built over that table above.
        let stored = unsafe { *OBJECT.as_interface().cast::<*const Methods>() };
        assert_eq!(stored, TABLE.address());
    }

    #[test]
    fn slots_are_called_the_way_cpp_calls_them() {
        let object = OBJECT.as_interface();
        // SAFETY: the table has two entries with these signatures.
        unsafe {
            let first: unsafe extern "C" fn(*mut c_void) -> c_int = slot(object, 0);
            let second: unsafe extern "C" fn(*mut c_void, c_int) -> c_int = slot(object, 1);
            assert_eq!(first(object), 7);
            assert_eq!(second(object, 21), 42);
        }
    }

    #[test]
    fn create_interface_matches_whole_names_only() {
        let interfaces = [(c"SceneFileCache002", OBJECT.as_interface())];
        let mut code = -1;
        // SAFETY: the names are terminated and the code is writable.
        unsafe {
            assert_eq!(
                create_interface(&interfaces, c"SceneFileCache002".as_ptr(), &mut code),
                OBJECT.as_interface()
            );
            assert_eq!(code, IFACE_OK);
            assert!(
                create_interface(&interfaces, c"SceneFileCache00".as_ptr(), &mut code).is_null()
            );
            assert_eq!(code, IFACE_FAILED);
            assert!(create_interface(
                &interfaces,
                c"SceneFileCache0022".as_ptr(),
                std::ptr::null_mut()
            )
            .is_null());
            assert!(create_interface(&interfaces, std::ptr::null(), &mut code).is_null());
        }
    }

    #[test]
    fn a_panic_becomes_the_fallback() {
        assert_eq!(guard(3, || panic!("contained")), 3);
        assert_eq!(guard(3, || 4), 4);
    }
}
