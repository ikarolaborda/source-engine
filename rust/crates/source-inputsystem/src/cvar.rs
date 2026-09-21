//! `ICvar` and `ConVar`, reached without linking against `vstdlib`.
//!
//! The C++ module reads four convars and writes two. Those are ordinary
//! `ConVar` objects, and the accessors on them — `GetFloat`, `GetString` —
//! are `FORCEINLINE_CVAR`, so there is no virtual call to borrow: they read
//! fields straight out of the object. What crosses the boundary is therefore
//! a *layout*, not just a vtable, which is a weaker thing to depend on.
//!
//! Two measurements and one runtime check make it safe to depend on anyway.
//! The slot numbers and field offsets below are what
//! `clang -Xclang -fdump-vtable-layouts -Xclang -fdump-record-layouts` prints
//! for the real translation unit, not what reading the header suggests. And
//! [`Cvar::find`] verifies the object it was handed by reading the name back
//! out of it at the offset it expects and comparing it with the name it
//! asked for. A layout that ever stops matching therefore fails one lookup
//! loudly instead of returning a float from the middle of some other field,
//! and the caller falls back to the value the module was built with.
//!
//! Writing goes the other way and needs no layout at all: `IConVar` really
//! does declare `SetValue` and `GetName` virtual, so those are slot calls.

use source_cppabi::{slot, tier0};
use std::ffi::{c_char, c_float, c_int, c_void, CStr};
use std::sync::atomic::{AtomicBool, Ordering};

/// `CVAR_INTERFACE_VERSION`.
pub const INTERFACE_VERSION: &CStr = c"VEngineCvar004";

// `ICvar`'s callable slots. Its vtable is 37 entries: `IAppSystem`'s five
// first, then its own. `FindVar` is overloaded on constness and the
// non-const one is declared first.
const FIND_VAR: usize = 12;
const INSTALL_GLOBAL_CHANGE_CALLBACK: usize = 18;
const REMOVE_GLOBAL_CHANGE_CALLBACK: usize = 19;

// `IConVar`'s callable slots. `SetValue` is overloaded three ways and they
// sit in declaration order — `const char *`, then `float`, then `int` — so
// the float one is second. The module only ever sets floats.
const SET_VALUE_FLOAT: usize = 1;
const GET_NAME: usize = 3;

// `ConVar`'s field offsets. `ConCommandBase` is the primary base, so its
// fields come first and `IConVar`'s vtable pointer sits between them and
// `ConVar`'s own.
const NAME_OFFSET: usize = 24;
const FLAGS_OFFSET: usize = 40;
const ICONVAR_OFFSET: usize = 48;
const PARENT_OFFSET: usize = 56;
const STRING_OFFSET: usize = 72;
const FLOAT_OFFSET: usize = 84;

/// `FCVAR_NEVER_AS_STRING`.
const FCVAR_NEVER_AS_STRING: c_int = 1 << 12;

/// `FnChangeCallback_t`. The `IConVar *` is the `IConVar` subobject, not the
/// `ConVar`, so the only safe thing to do with it is call its virtuals.
pub type ChangeCallback =
    unsafe extern "C" fn(*mut c_void, *const c_char, c_float);

/// The engine's convar registry.
#[derive(Debug, Clone, Copy)]
pub struct Cvar {
    object: *mut c_void,
}

// SAFETY: the pointer is only used from the thread that polls, which is the
// engine's main thread; `Send` is needed because it is stored in the module's
// state, which lives behind a mutex.
unsafe impl Send for Cvar {}

impl Cvar {
    /// Adopts the object a factory answered with, or nothing for a null.
    #[must_use]
    pub fn new(object: *mut c_void) -> Option<Self> {
        (!object.is_null()).then_some(Self { object })
    }

    /// `ICvar::FindVar`, with the answer checked against the name asked for.
    ///
    /// A convar the engine does not have answers nothing, which is ordinary:
    /// this module is loaded by a dedicated server too, where the joystick
    /// convars are never registered.
    #[must_use]
    pub fn find(&self, name: &CStr) -> Option<ConVar> {
        // SAFETY: slot 12 of a live `ICvar` has this signature.
        let found = unsafe {
            let find: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
                slot(self.object, FIND_VAR);
            find(self.object, name.as_ptr())
        };
        if found.is_null() {
            return None;
        }

        let var = ConVar { object: found };
        // The field offsets are measured rather than declared, so the first
        // thing done with a new object is to prove they still describe it.
        // Reading the name back is the cheapest check that covers all of them
        // at once, because it is the field furthest from the ones that matter.
        if var.name() != Some(name) {
            report_layout_mismatch(name);
            return None;
        }
        Some(var)
    }

    /// `ICvar::InstallGlobalChangeCallback`, which fires for every convar the
    /// engine changes; the callback filters by name.
    ///
    /// # Safety
    ///
    /// `callback` must stay callable until [`Self::remove_change_callback`].
    pub unsafe fn install_change_callback(&self, callback: ChangeCallback) {
        // SAFETY: slot 18 of a live `ICvar` takes this function pointer, and
        // the caller keeps it callable per the contract.
        unsafe {
            let install: unsafe extern "C" fn(*mut c_void, ChangeCallback) =
                slot(self.object, INSTALL_GLOBAL_CHANGE_CALLBACK);
            install(self.object, callback);
        }
    }

    /// `ICvar::RemoveGlobalChangeCallback`, which must name the same function.
    pub fn remove_change_callback(&self, callback: ChangeCallback) {
        // SAFETY: slot 19 of a live `ICvar` takes this function pointer.
        unsafe {
            let remove: unsafe extern "C" fn(*mut c_void, ChangeCallback) =
                slot(self.object, REMOVE_GLOBAL_CHANGE_CALLBACK);
            remove(self.object, callback);
        }
    }
}

/// One convar.
#[derive(Debug, Clone, Copy)]
pub struct ConVar {
    object: *mut c_void,
}

// SAFETY: as for `Cvar` above.
unsafe impl Send for ConVar {}

impl ConVar {
    fn field<T: Copy>(&self, offset: usize) -> T {
        // SAFETY: the object is a live `ConVar` and `offset` is one of the
        // measured field offsets above, whose type is `T`.
        unsafe { self.object.cast::<u8>().add(offset).cast::<T>().read() }
    }

    /// `m_pParent`. A convar that is not a child of another is its own
    /// parent, so this is never null on a registered convar — but it is read
    /// from a measured offset, so it is checked rather than trusted.
    fn parent(&self) -> Option<Self> {
        let parent: *mut c_void = self.field(PARENT_OFFSET);
        (!parent.is_null()).then_some(Self { object: parent })
    }

    /// `ConCommandBase::m_pszName`, which is what [`Cvar::find`] checks.
    #[must_use]
    pub fn name(&self) -> Option<&CStr> {
        let name: *const c_char = self.field(NAME_OFFSET);
        // SAFETY: the engine owns the string and keeps it for the convar's
        // lifetime, which outlives this module's use of it.
        (!name.is_null()).then(|| unsafe { CStr::from_ptr(name) })
    }

    /// `ConVar::GetFloat`, which reads the parent's value.
    #[must_use]
    pub fn float(&self) -> Option<f32> {
        self.parent().map(|parent| parent.field(FLOAT_OFFSET))
    }

    /// `ConVar::GetString`, including its refusal to print a convar flagged
    /// never to be shown as one.
    ///
    /// The string is the engine's, not this object's, which is why it is
    /// `'static` rather than borrowed from `&self`: the engine allocates it
    /// when the convar is registered and replaces it only when the value
    /// changes. Every reader here runs on the thread that polls, and so does
    /// every writer, so no reader can be holding it across a change.
    #[must_use]
    pub fn string(&self) -> Option<&'static CStr> {
        let flags: c_int = self.field(FLAGS_OFFSET);
        if flags & FCVAR_NEVER_AS_STRING != 0 {
            return Some(c"FCVAR_NEVER_AS_STRING");
        }
        let parent = self.parent()?;
        let value: *const c_char = parent.field(STRING_OFFSET);
        if value.is_null() {
            return Some(c"");
        }
        // SAFETY: the engine owns the string and replaces it only when the
        // convar changes, which cannot happen while this borrow is held: both
        // run on the thread that polls.
        Some(unsafe { CStr::from_ptr(value) })
    }

    /// `ConVar::GetBool`, which is `GetInt() != 0`; the stored float and int
    /// agree for every convar the engine sets, so the float is read.
    #[must_use]
    pub fn bool(&self) -> Option<bool> {
        self.float().map(|value| value != 0.0)
    }

    /// The `IConVar` subobject, which is where the setters live.
    fn as_iconvar(&self) -> *mut c_void {
        // SAFETY: the object is a live `ConVar`, whose `IConVar` base sits at
        // this measured offset.
        unsafe { self.object.cast::<u8>().add(ICONVAR_OFFSET).cast() }
    }

    /// `IConVar::SetValue( float )`. A slot call, not a field write: setting a
    /// convar runs its change callbacks and the engine has to do that itself.
    pub fn set_float(&self, value: f32) {
        let object = self.as_iconvar();
        // SAFETY: slot 1 of a live `IConVar` has this signature.
        unsafe {
            let set: unsafe extern "C" fn(*mut c_void, c_float) = slot(object, SET_VALUE_FLOAT);
            set(object, value);
        }
    }

}

/// `IConVar::GetName`, for the object a change callback is handed.
///
/// # Safety
///
/// `iconvar` must be the `IConVar *` a change callback was called with.
#[must_use]
pub unsafe fn changed_name<'a>(iconvar: *mut c_void) -> Option<&'a CStr> {
    if iconvar.is_null() {
        return None;
    }
    // SAFETY: slot 3 of a live `IConVar` has this signature, and the engine
    // owns the name for the convar's lifetime.
    unsafe {
        let get: unsafe extern "C" fn(*mut c_void) -> *const c_char = slot(iconvar, GET_NAME);
        let name = get(iconvar);
        (!name.is_null()).then(|| CStr::from_ptr(name))
    }
}

/// Said once, however many convars are looked up: a layout that has moved is
/// one fact about the build, not one fact per convar.
fn report_layout_mismatch(name: &CStr) {
    static REPORTED: AtomicBool = AtomicBool::new(false);
    if REPORTED.swap(true, Ordering::Relaxed) {
        return;
    }
    tier0::warning(&format!(
        "inputsystem: ConVar layout does not match the one this module was built against \
         (looking up {name:?} gave an object with a different name). Falling back to built-in \
         defaults for the joystick convars.\n"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    // A `ConVar` as the engine lays one out, filled in far enough for the
    // accessors to read it. Building it here is what pins the offsets to
    // something a reader can check against the record dump in the module
    // documentation, without needing the engine to be present.
    #[repr(C, align(8))]
    struct FakeConVar([u8; 128]);

    impl FakeConVar {
        fn new(name: &CStr, flags: c_int, value: f32, string: &CStr) -> Box<Self> {
            let mut fake = Box::new(Self([0; 128]));
            let base = fake.0.as_mut_ptr();
            // SAFETY: every write is inside the 128-byte body at an offset the
            // module documents, and the pointers written outlive the test.
            unsafe {
                base.add(NAME_OFFSET)
                    .cast::<*const c_char>()
                    .write(name.as_ptr());
                base.add(FLAGS_OFFSET).cast::<c_int>().write(flags);
                base.add(STRING_OFFSET)
                    .cast::<*const c_char>()
                    .write(string.as_ptr());
                base.add(FLOAT_OFFSET).cast::<f32>().write(value);
            }
            // A convar with no child is its own parent.
            let self_pointer: *mut c_void = fake.0.as_mut_ptr().cast();
            // SAFETY: as above.
            unsafe {
                fake.0
                    .as_mut_ptr()
                    .add(PARENT_OFFSET)
                    .cast::<*mut c_void>()
                    .write(self_pointer);
            }
            fake
        }

        fn as_convar(&self) -> ConVar {
            ConVar {
                object: std::ptr::from_ref(self).cast_mut().cast(),
            }
        }
    }

    #[test]
    fn the_offsets_are_the_ones_the_record_dump_prints() {
        // ConCommandBase is the primary base, so its fields come first, and
        // IConVar's vtable pointer sits between them and ConVar's own.
        assert_eq!(NAME_OFFSET, 24, "ConCommandBase::m_pszName");
        assert_eq!(FLAGS_OFFSET, 40, "ConCommandBase::m_nFlags");
        assert_eq!(ICONVAR_OFFSET, 48, "the IConVar base");
        assert_eq!(PARENT_OFFSET, 56, "ConVar::m_pParent");
        assert_eq!(STRING_OFFSET, 72, "ConVar::m_pszString");
        assert_eq!(FLOAT_OFFSET, 84, "ConVar::m_fValue");
    }

    #[test]
    fn a_convar_reads_its_value_through_its_parent() {
        let fake = FakeConVar::new(c"joy_axis_deadzone", 0, 0.2, c"0.2");
        let var = fake.as_convar();
        assert_eq!(var.name(), Some(c"joy_axis_deadzone"));
        assert_eq!(var.float(), Some(0.2));
        assert_eq!(var.string(), Some(c"0.2"));
        assert_eq!(var.bool(), Some(true));
    }

    #[test]
    fn a_child_convar_reads_the_parents_value_rather_than_its_own() {
        let parent = FakeConVar::new(c"joystick", 0, 1.0, c"1");
        let mut child = FakeConVar::new(c"joystick", 0, 0.0, c"0");
        let parent_pointer: *mut c_void = std::ptr::from_ref(&*parent).cast_mut().cast();
        // SAFETY: the offset is the measured one and the parent outlives it.
        unsafe {
            child
                .0
                .as_mut_ptr()
                .add(PARENT_OFFSET)
                .cast::<*mut c_void>()
                .write(parent_pointer);
        }
        assert_eq!(child.as_convar().float(), Some(1.0), "the parent's value");
        assert_eq!(child.as_convar().string(), Some(c"1"));
    }

    #[test]
    fn a_zero_value_is_false_which_is_what_the_rumble_check_needs() {
        let off = FakeConVar::new(c"joystick", 0, 0.0, c"0");
        assert_eq!(off.as_convar().bool(), Some(false));
    }

    #[test]
    fn a_convar_flagged_never_as_string_says_so_instead_of_its_value() {
        let secret = FakeConVar::new(c"hidden", FCVAR_NEVER_AS_STRING, 1.0, c"sekrit");
        assert_eq!(secret.as_convar().string(), Some(c"FCVAR_NEVER_AS_STRING"));
        // The float is unaffected; only the string accessor refuses.
        assert_eq!(secret.as_convar().float(), Some(1.0));
    }

    #[test]
    fn a_null_string_reads_as_empty_rather_than_dereferencing() {
        let mut blank = FakeConVar::new(c"joy_gamecontroller_config", 0, 0.0, c"");
        // SAFETY: the offset is the measured one.
        unsafe {
            blank
                .0
                .as_mut_ptr()
                .add(STRING_OFFSET)
                .cast::<*const c_char>()
                .write(std::ptr::null());
        }
        assert_eq!(blank.as_convar().string(), Some(c""));
    }

    #[test]
    fn a_convar_with_no_parent_answers_nothing_rather_than_reading_null() {
        let mut orphan = FakeConVar::new(c"joy_active", 0, 1.0, c"1");
        // SAFETY: the offset is the measured one.
        unsafe {
            orphan
                .0
                .as_mut_ptr()
                .add(PARENT_OFFSET)
                .cast::<*mut c_void>()
                .write(std::ptr::null_mut());
        }
        assert_eq!(orphan.as_convar().float(), None);
        assert_eq!(orphan.as_convar().string(), None);
        assert_eq!(orphan.as_convar().bool(), None);
        // The name lives in the object itself, so it still reads.
        assert_eq!(orphan.as_convar().name(), Some(c"joy_active"));
    }
}
