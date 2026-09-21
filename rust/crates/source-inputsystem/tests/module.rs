//! Loads the built module the way the engine does and calls every slot.
//!
//! The unit tests reach the functions directly; this one goes through
//! `dlopen`, `CreateInterface` and the vtable, so it is the slot *layout*
//! under test — an entry in the wrong place reads a neighbouring function
//! pointer and calls it with the wrong arguments, which nothing else here
//! would catch.
//!
//! Only the module itself is loaded. Nothing in the engine is, so the slots
//! that need the launcher or SDL are exercised for their no-device answer,
//! which is the answer the C++ gives without those too.

use std::ffi::{c_char, c_int, c_void, CStr, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

const RTLD_NOW: c_int = 2;
const RTLD_LOCAL: c_int = 4;

extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

type CreateInterfaceFn = unsafe extern "C" fn(*const c_char, *mut c_int) -> *mut c_void;

/// The cdylib Cargo built beside this test binary.
fn module_path() -> PathBuf {
    let mut path = std::env::current_exe().expect("test binary path");
    // .../target/<profile>/deps/<test>  ->  .../target/<profile>
    path.pop();
    path.pop();
    path.push(format!("{}inputsystem{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX));
    path
}

/// Reads slot `index` of the object, as a C++ caller would.
///
/// # Safety
///
/// `F` must be the `extern "C"` signature, object first, of the function the
/// interface declares at `index`.
unsafe fn slot<F: Copy>(object: *mut c_void, index: usize) -> F {
    assert_eq!(size_of::<F>(), size_of::<*const c_void>());
    // SAFETY: the object's first word is its table, per the contract above.
    unsafe {
        let table = *object.cast::<*const F>();
        *table.add(index)
    }
}

struct Module {
    object: *mut c_void,
}

impl Module {
    fn load() -> Self {
        let path = module_path();
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: the path is NUL-terminated and the flags are the documented
        // ones; a null answer is checked before use.
        let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        assert!(
            !handle.is_null(),
            "dlopen({}): {}",
            path.display(),
            // SAFETY: dlerror answers null or a NUL-terminated string.
            unsafe {
                let message = dlerror();
                if message.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(message).to_string_lossy().into_owned()
                }
            }
        );

        // SAFETY: the name is NUL-terminated; the symbol is checked for null.
        let factory = unsafe { dlsym(handle, c"CreateInterface".as_ptr()) };
        assert!(!factory.is_null(), "the module exports no CreateInterface");
        // SAFETY: the module defines it with this signature.
        let factory: CreateInterfaceFn = unsafe { std::mem::transmute(factory) };

        let mut code = -1;
        // SAFETY: both arguments are valid for the call.
        let object = unsafe { factory(c"InputSystemVersion001".as_ptr(), &raw mut code) };
        assert!(!object.is_null(), "no InputSystemVersion001");
        assert_eq!(code, 0, "IFACE_OK");

        // A name it does not implement must be refused, with the code set.
        code = -1;
        // SAFETY: as above.
        let missing = unsafe { factory(c"InputSystemVersion002".as_ptr(), &raw mut code) };
        assert!(missing.is_null(), "answered an interface it does not have");
        assert_eq!(code, 1, "IFACE_FAILED");

        Self { object }
    }

    fn int_of_int(&self, index: usize, argument: c_int) -> c_int {
        // SAFETY: the caller names a slot with this signature.
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void, c_int) -> c_int = slot(self.object, index);
            f(self.object, argument)
        }
    }

    fn int(&self, index: usize) -> c_int {
        // SAFETY: the caller names a slot with this signature.
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void) -> c_int = slot(self.object, index);
            f(self.object)
        }
    }

    fn bool_of_int(&self, index: usize, argument: c_int) -> bool {
        // SAFETY: the caller names a slot with this signature.
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void, c_int) -> bool = slot(self.object, index);
            f(self.object, argument)
        }
    }

    fn string_of_int(&self, index: usize, argument: c_int) -> &'static CStr {
        // SAFETY: the caller names a slot with this signature, and the string
        // it answers is static storage in the module.
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void, c_int) -> *const c_char =
                slot(self.object, index);
            let answer = f(self.object, argument);
            assert!(!answer.is_null());
            CStr::from_ptr(answer)
        }
    }

    fn int_of_string(&self, index: usize, argument: &CStr) -> c_int {
        // SAFETY: the caller names a slot with this signature.
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int =
                slot(self.object, index);
            f(self.object, argument.as_ptr())
        }
    }

    fn call(&self, index: usize) {
        // SAFETY: the caller names a slot with this signature.
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void) = slot(self.object, index);
            f(self.object);
        }
    }

    fn call_bool(&self, index: usize, argument: bool) {
        // SAFETY: the caller names a slot with this signature.
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void, bool) = slot(self.object, index);
            f(self.object, argument);
        }
    }
}

// The callable slot numbers, from `clang -Xclang -fdump-vtable-layouts` on
// `IInputSystem`: `IAppSystem`'s five, then its own fifty-two.
const QUERY_INTERFACE: usize = 2;
const ENABLE_INPUT: usize = 7;
const ENABLE_MESSAGE_PUMP: usize = 8;
const POLL_INPUT_STATE: usize = 9;
const IS_BUTTON_DOWN: usize = 11;
const GET_BUTTON_PRESSED_TICK: usize = 12;
const GET_BUTTON_RELEASED_TICK: usize = 13;
const GET_ANALOG_VALUE: usize = 14;
const GET_ANALOG_DELTA: usize = 15;
const GET_EVENT_COUNT: usize = 16;
const GET_EVENT_DATA: usize = 17;
const POST_USER_EVENT: usize = 18;
const GET_JOYSTICK_COUNT: usize = 19;
const SAMPLE_DEVICES: usize = 22;
const STOP_RUMBLE: usize = 24;
const RESET_INPUT_STATE: usize = 25;
const SET_PRIMARY_USER_ID: usize = 26;
const BUTTON_CODE_TO_STRING: usize = 27;
const ANALOG_CODE_TO_STRING: usize = 28;
const STRING_TO_BUTTON_CODE: usize = 29;
const STRING_TO_ANALOG_CODE: usize = 30;
const VIRTUAL_KEY_TO_BUTTON_CODE: usize = 32;
const BUTTON_CODE_TO_VIRTUAL_KEY: usize = 33;
const SCAN_CODE_TO_BUTTON_CODE: usize = 34;
const GET_POLL_COUNT: usize = 35;
const GET_HAPTICS_INTERFACE_ADDRESS: usize = 37;
const GET_TOUCH_ACCUMULATORS: usize = 40;
const STEAM_CONTROLLER_INTERFACE: usize = 42;
const GET_NUM_STEAM_CONTROLLERS: usize = 43;
const IS_STEAM_CONTROLLER_ACTIVE: usize = 44;
const IS_STEAM_CONTROLLER_CONNECTED: usize = 45;
const GET_STEAM_CONTROLLER_INDEX_FOR_SLOT: usize = 46;
const GET_RADIAL_MENU_STICK_VALUES: usize = 47;
const GET_ACTION_SET_HANDLE_BY_ENUM: usize = 49;
const GET_ACTION_SET_HANDLE_BY_NAME: usize = 50;
const GET_ACTION_ORIGIN_BY_ENUM: usize = 51;
const GET_FONT_CHARACTER: usize = 53;
const GET_DESCRIPTION: usize = 54;
const SET_SKIP_CONTROLLER_INITIALIZATION: usize = 55;

const BUTTON_CODE_LAST: c_int = 635;
const ANALOG_CODE_LAST: c_int = 10;
const BUTTON_CODE_INVALID: c_int = -1;

#[test]
fn every_conversion_slot_answers_through_the_vtable() {
    let module = Module::load();

    // Reading a name for every code at all proves the slot is the one that
    // takes an int and answers a string, at the index the dump gave.
    for code in 0..BUTTON_CODE_LAST {
        let name = module.string_of_int(BUTTON_CODE_TO_STRING, code);
        if name.is_empty() {
            continue;
        }
        // Whatever name came back must resolve to a code with that same name.
        let back = module.int_of_string(STRING_TO_BUTTON_CODE, name);
        assert_ne!(back, BUTTON_CODE_INVALID, "{name:?} at {code} did not resolve");
        assert_eq!(module.string_of_int(BUTTON_CODE_TO_STRING, back), name);
    }
    for code in 0..ANALOG_CODE_LAST {
        let name = module.string_of_int(ANALOG_CODE_TO_STRING, code);
        assert_eq!(module.int_of_string(STRING_TO_ANALOG_CODE, name), code);
    }

    assert_eq!(module.string_of_int(BUTTON_CODE_TO_STRING, 11), c"a");
    assert_eq!(module.int_of_string(STRING_TO_BUTTON_CODE, c"a"), 11);
    assert_eq!(module.int_of_string(STRING_TO_BUTTON_CODE, c"A"), 11);
    assert_eq!(
        module.int_of_string(STRING_TO_BUTTON_CODE, c"nosuchbutton"),
        BUTTON_CODE_INVALID
    );
    assert_eq!(module.string_of_int(ANALOG_CODE_TO_STRING, 0), c"MOUSE_X");

    for key in 0..256 {
        let code = module.int_of_int(VIRTUAL_KEY_TO_BUTTON_CODE, key);
        if code != 0 {
            assert_eq!(module.int_of_int(BUTTON_CODE_TO_VIRTUAL_KEY, code), key);
        }
    }
    for scan in 0..256 {
        for extended in 0..2 {
            let lparam = (scan << 16) | (extended << 24);
            let code = module.int_of_int(SCAN_CODE_TO_BUTTON_CODE, lparam);
            assert!((0..BUTTON_CODE_LAST).contains(&code), "scan {scan} -> {code}");
        }
    }
}

#[test]
fn the_resting_state_is_what_a_module_with_no_devices_reports() {
    let module = Module::load();

    // The interface is a singleton — `EXPOSE_SINGLE_INTERFACE_GLOBALVAR` in
    // the C++, one static object here — and every test in this binary shares
    // the one instance, so the poll tick and count belong to whichever test
    // polls. What is asserted here is what stays true whatever else has run:
    // nothing is held down and nothing is attached, because no test can make
    // either happen without a device.
    // The poll tick is not asserted: it is a delta against the startup tick
    // that `Init` records, and `Init` is not run here, so before it the value
    // is whatever the millisecond counter happens to read. Its behaviour is
    // covered where it is defined, in `source_input::state`.
    assert_eq!(module.int(GET_JOYSTICK_COUNT), 0);

    for code in 0..BUTTON_CODE_LAST {
        assert!(!module.bool_of_int(IS_BUTTON_DOWN, code));
        assert_eq!(module.int_of_int(GET_BUTTON_PRESSED_TICK, code), 0);
        assert_eq!(module.int_of_int(GET_BUTTON_RELEASED_TICK, code), 0);
    }
    for code in 0..ANALOG_CODE_LAST {
        assert_eq!(module.int_of_int(GET_ANALOG_VALUE, code), 0);
        assert_eq!(module.int_of_int(GET_ANALOG_DELTA, code), 0);
    }

    // SAFETY: slot 37 answers a pointer and takes nothing but the object.
    let haptics = unsafe {
        let f: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
            slot(module.object, GET_HAPTICS_INTERFACE_ADDRESS);
        f(module.object)
    };
    assert!(haptics.is_null(), "Novint haptics are Windows-only");
}

#[test]
fn the_steam_slots_answer_what_a_null_interface_forces() {
    let module = Module::load();

    // SAFETY: slot 42 answers a pointer and takes nothing but the object.
    let controller = unsafe {
        let f: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
            slot(module.object, STEAM_CONTROLLER_INTERFACE);
        f(module.object)
    };
    assert!(controller.is_null());

    // SAFETY: slot 43 answers a uint32.
    let connected = unsafe {
        let f: unsafe extern "C" fn(*mut c_void) -> u32 = slot(module.object, GET_NUM_STEAM_CONTROLLERS);
        f(module.object)
    };
    assert_eq!(connected, 0);

    // SAFETY: both slots answer a bool and take nothing but the object.
    unsafe {
        let active: unsafe extern "C" fn(*mut c_void) -> bool =
            slot(module.object, IS_STEAM_CONTROLLER_ACTIVE);
        let attached: unsafe extern "C" fn(*mut c_void) -> bool =
            slot(module.object, IS_STEAM_CONTROLLER_CONNECTED);
        assert!(!active(module.object));
        assert!(!attached(module.object));
    }

    for pad in -1..9 {
        assert_eq!(module.int_of_int(GET_STEAM_CONTROLLER_INDEX_FOR_SLOT, pad), -1);
    }

    // SAFETY: slot 49 takes a GameActionSet_t and answers a handle.
    unsafe {
        let by_enum: unsafe extern "C" fn(*mut c_void, c_int) -> u64 =
            slot(module.object, GET_ACTION_SET_HANDLE_BY_ENUM);
        let by_name: unsafe extern "C" fn(*mut c_void, *const c_char) -> u64 =
            slot(module.object, GET_ACTION_SET_HANDLE_BY_NAME);
        for set in -1..4 {
            assert_eq!(by_enum(module.object, set), 0);
        }
        assert_eq!(by_name(module.object, c"MenuControls".as_ptr()), 0);
        assert_eq!(by_name(module.object, c"NoSuchSet".as_ptr()), 0);

        let origin: unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_int =
            slot(module.object, GET_ACTION_ORIGIN_BY_ENUM);
        assert_eq!(origin(module.object, c"menu_select".as_ptr(), 0), 0);

        // The two out-parameters must be written even with nothing attached,
        // and the answer is true, which is what the C++ returns.
        let radial: unsafe extern "C" fn(*mut c_void, c_int, *mut f32, *mut f32) -> bool =
            slot(module.object, GET_RADIAL_MENU_STICK_VALUES);
        let mut x = 9.0f32;
        let mut y = 9.0f32;
        assert!(radial(module.object, 0, &raw mut x, &raw mut y));
        assert_eq!((x, y), (0.0, 0.0));
    }
}

#[test]
fn the_origin_tables_are_reachable_and_terminated() {
    let module = Module::load();

    // SAFETY: both slots take an origin and answer a `const wchar_t *`, which
    // is a pointer to NUL-terminated UTF-32 here.
    unsafe {
        let icon: unsafe extern "C" fn(*mut c_void, c_int) -> *const u32 =
            slot(module.object, GET_FONT_CHARACTER);
        let description: unsafe extern "C" fn(*mut c_void, c_int) -> *const u32 =
            slot(module.object, GET_DESCRIPTION);

        for origin in -1..45 {
            for answer in [icon(module.object, origin), description(module.object, origin)] {
                assert!(!answer.is_null(), "origin {origin}");
                // Reading to the terminator proves the pointer is a string and
                // that an origin past the table still answers one.
                let mut length = 0;
                while *answer.add(length) != 0 {
                    length += 1;
                    assert!(length < 32, "origin {origin} is not terminated");
                }
            }
        }

        assert_eq!(*icon(module.object, 1), u32::from(b'A'));
        assert_eq!(*description(module.object, 5), u32::from(b'L'));
        assert_eq!(*description(module.object, 5).add(1), u32::from(b'B'));
        assert_eq!(*icon(module.object, 39), 0, "past the table is empty");
    }
}

#[test]
fn an_event_posted_between_frames_arrives_on_the_next_poll_and_only_once() {
    let module = Module::load();

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct InputEvent {
        kind: c_int,
        tick: c_int,
        data: c_int,
        data2: c_int,
        data3: c_int,
    }
    const IE_QUIT: c_int = 100;

    // SAFETY: slot 18 takes a `const InputEvent_t &`, which is a pointer.
    unsafe {
        let post: unsafe extern "C" fn(*mut c_void, *const InputEvent) =
            slot(module.object, POST_USER_EVENT);
        let event = InputEvent {
            kind: IE_QUIT,
            tick: 1234,
            data: 7,
            ..InputEvent::default()
        };
        post(module.object, &raw const event);
    }

    // Nothing is visible yet: a post between frames lands in the queued half
    // of the double buffer, which is not the half GetEventData reads.
    assert_eq!(module.int(GET_EVENT_COUNT), 0);

    // No launcher is connected, so the poll does its state work and skips the
    // pump, which is the path a dedicated server takes too. The count is read
    // as a difference because the interface is a singleton this binary shares.
    let polls_before = module.int(GET_POLL_COUNT);
    module.call(POLL_INPUT_STATE);
    assert_eq!(module.int(GET_EVENT_COUNT), 1);
    assert_eq!(module.int(GET_POLL_COUNT), polls_before + 1);

    // SAFETY: slot 17 answers a pointer into the module's own storage.
    let data = unsafe {
        let f: unsafe extern "C" fn(*mut c_void) -> *const InputEvent =
            slot(module.object, GET_EVENT_DATA);
        f(module.object)
    };
    assert!(!data.is_null());
    // SAFETY: GetEventCount said there is one event behind the pointer, and
    // nothing has polled since, so it is still the buffer that was handed out.
    unsafe {
        assert_eq!((*data).kind, IE_QUIT);
        assert_eq!((*data).tick, 1234);
        assert_eq!((*data).data, 7);
    }

    module.call(POLL_INPUT_STATE);
    assert_eq!(module.int(GET_EVENT_COUNT), 0, "delivered once");
    assert_eq!(module.int(GET_POLL_COUNT), polls_before + 2);
}

#[test]
fn the_setters_and_the_no_device_slots_run_without_a_launcher() {
    let module = Module::load();

    // None of these has anything to talk to, and all of them must return
    // rather than reach through a null launcher or a null SDL.
    module.call_bool(ENABLE_INPUT, false);
    module.call_bool(ENABLE_MESSAGE_PUMP, false);
    module.call_bool(SET_SKIP_CONTROLLER_INITIALIZATION, true);
    module.call(SAMPLE_DEVICES);
    module.call(STOP_RUMBLE);
    module.call(RESET_INPUT_STATE);

    // SAFETY: slot 26 takes an int and answers nothing.
    unsafe {
        let set: unsafe extern "C" fn(*mut c_void, c_int) = slot(module.object, SET_PRIMARY_USER_ID);
        for user in -2..5 {
            set(module.object, user);
        }
    }

    // SAFETY: slot 40 takes a finger and two out-parameters.
    unsafe {
        let touch: unsafe extern "C" fn(*mut c_void, c_int, *mut f32, *mut f32) -> bool =
            slot(module.object, GET_TOUCH_ACCUMULATORS);
        for finger in 0..12 {
            let mut dx = 9.0f32;
            let mut dy = 9.0f32;
            assert!(touch(module.object, finger, &raw mut dx, &raw mut dy));
            assert_eq!((dx, dy), (0.0, 0.0), "finger {finger}");
        }
    }

    // QueryInterface must answer the same object the factory did.
    // SAFETY: slot 2 takes a name and answers the interface or null.
    unsafe {
        let query: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
            slot(module.object, QUERY_INTERFACE);
        assert_eq!(
            query(module.object, c"InputSystemVersion001".as_ptr()),
            module.object
        );
        assert!(query(module.object, c"NoSuchInterface".as_ptr()).is_null());
        assert!(query(module.object, std::ptr::null()).is_null());
    }
}

#[test]
fn the_module_exports_only_the_factory_and_links_no_cpp_runtime() {
    // The module is found through CreateInterface and named by nobody, so
    // that one symbol is the whole contract. Anything else exported would be
    // an accident, and any C++ runtime import would mean C++ came back.
    let path = module_path();
    let exports = std::process::Command::new("nm")
        .args([OsStr::new("-gU"), path.as_os_str()])
        .output()
        .expect("nm");
    let exports = String::from_utf8_lossy(&exports.stdout);
    let names: Vec<&str> = exports
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .collect();
    assert_eq!(names, ["_CreateInterface"], "unexpected exports");

    let imports = std::process::Command::new("nm")
        .args([OsStr::new("-u"), path.as_os_str()])
        .output()
        .expect("nm");
    let imports = String::from_utf8_lossy(&imports.stdout);
    for symbol in imports.lines().map(str::trim) {
        assert!(
            !symbol.starts_with("___cxa")
                && !symbol.starts_with("___gxx")
                && !symbol.starts_with("__Zn")
                && !symbol.starts_with("__Zd"),
            "imports the C++ runtime: {symbol}"
        );
        // SDL is resolved with dlsym against whatever the process already
        // loaded. Importing it would mean the module could bind to a second
        // SDL beside the launcher's.
        assert!(
            !symbol.starts_with("_SDL_"),
            "imports SDL instead of resolving it at runtime: {symbol}"
        );
    }

    let linked = std::process::Command::new("otool")
        .args([OsStr::new("-L"), path.as_os_str()])
        .output()
        .expect("otool");
    let linked = String::from_utf8_lossy(&linked.stdout);
    for library in ["libc++", "libtier0", "libvstdlib", "libSDL2"] {
        assert!(
            !linked.contains(library),
            "links {library}, which it should be resolving at runtime"
        );
    }
}
