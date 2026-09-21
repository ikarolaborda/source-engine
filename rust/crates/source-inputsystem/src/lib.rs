//! The `inputsystem` engine module, with no C++ in it.
//!
//! The engine loads this library by name, asks its `CreateInterface` for
//! `InputSystemVersion001`, and from then on calls it as an `IInputSystem`:
//! 57 virtual functions, `IAppSystem`'s five and its own fifty-two. It
//! replaces `inputsystem/`'s five translation units whole. The behaviour is
//! [`source_input`]; what is here is the vtable, the calls out to the
//! launcher and to SDL, and the two SDL event watches.
//!
//! Three things shape it, and all three are recorded in
//! `docs/rust-port/inputsystem-boundary.md`:
//!
//! * **The launcher owns the event pump.** Keyboard, mouse, focus and quit
//!   arrive as `CCocoaEvent`s from `SDLMgrInterface001`. SDL is only asked for
//!   game controllers and touch, through watches the launcher's pump invokes.
//!   So the lock is dropped around the pump: the watches re-enter this module
//!   from inside it.
//! * **The Steam controller path is inert.** `SteamControllerInterface()`
//!   returns null because `CSteamAPIContext::Init` fails at its first line
//!   against the `steam_api` stub this tree links, so the Steam slots answer
//!   what the C++ answers with a null pointer. The two origin tables are the
//!   exception: the UI reads them either way, so they are ported whole.
//! * **`GetEventData` hands out an interior pointer.** Writes go to the slot
//!   `m_bIsPolling` selects, reads always come from `CURRENT`, so an event a
//!   caller posts between frames lands in `QUEUED` and cannot move the buffer
//!   the engine is still reading.

mod launcher;
mod sdl;

use launcher::{CocoaEvent, LauncherMgr, EVENT_BATCH};
use source_cppabi::appsystem::{AppSystemMethods, InitReturnVal};
use source_cppabi::{create_interface, guard, tier0, CreateInterfaceFn, Object, VTable};
use source_input::codes;
use source_input::keymap::cocoa_virtual_key_to_button_code;
use source_input::state::{self, InputCore, InputEvent};
use source_input::steam;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// `INPUTSYSTEM_INTERFACE_VERSION`.
pub const INTERFACE_VERSION: &CStr = c"InputSystemVersion001";

type This = Object<Methods>;

/// `IInputSystem`: `IAppSystem`'s five, then its own fifty-two in declaration
/// order, as `clang -Xclang -fdump-vtable-layouts` prints them.
#[repr(C)]
struct Methods {
    app_system: AppSystemMethods<This>,
    attach_to_window: unsafe extern "C" fn(*mut This, *mut c_void),
    detach_from_window: unsafe extern "C" fn(*mut This),
    enable_input: unsafe extern "C" fn(*mut This, bool),
    enable_message_pump: unsafe extern "C" fn(*mut This, bool),
    poll_input_state: unsafe extern "C" fn(*mut This),
    get_poll_tick: unsafe extern "C" fn(*mut This) -> c_int,
    is_button_down: unsafe extern "C" fn(*mut This, c_int) -> bool,
    get_button_pressed_tick: unsafe extern "C" fn(*mut This, c_int) -> c_int,
    get_button_released_tick: unsafe extern "C" fn(*mut This, c_int) -> c_int,
    get_analog_value: unsafe extern "C" fn(*mut This, c_int) -> c_int,
    get_analog_delta: unsafe extern "C" fn(*mut This, c_int) -> c_int,
    get_event_count: unsafe extern "C" fn(*mut This) -> c_int,
    get_event_data: unsafe extern "C" fn(*mut This) -> *const InputEvent,
    post_user_event: unsafe extern "C" fn(*mut This, *const InputEvent),
    get_joystick_count: unsafe extern "C" fn(*mut This) -> c_int,
    enable_joystick_input: unsafe extern "C" fn(*mut This, c_int, bool),
    enable_joystick_diagonal_pov: unsafe extern "C" fn(*mut This, c_int, bool),
    sample_devices: unsafe extern "C" fn(*mut This),
    set_rumble: unsafe extern "C" fn(*mut This, f32, f32, c_int),
    stop_rumble: unsafe extern "C" fn(*mut This),
    reset_input_state: unsafe extern "C" fn(*mut This),
    set_primary_user_id: unsafe extern "C" fn(*mut This, c_int),
    button_code_to_string: unsafe extern "C" fn(*mut This, c_int) -> *const c_char,
    analog_code_to_string: unsafe extern "C" fn(*mut This, c_int) -> *const c_char,
    string_to_button_code: unsafe extern "C" fn(*mut This, *const c_char) -> c_int,
    string_to_analog_code: unsafe extern "C" fn(*mut This, *const c_char) -> c_int,
    sleep_until_input: unsafe extern "C" fn(*mut This, c_int),
    virtual_key_to_button_code: unsafe extern "C" fn(*mut This, c_int) -> c_int,
    button_code_to_virtual_key: unsafe extern "C" fn(*mut This, c_int) -> c_int,
    scan_code_to_button_code: unsafe extern "C" fn(*mut This, c_int) -> c_int,
    get_poll_count: unsafe extern "C" fn(*mut This) -> c_int,
    set_cursor_position: unsafe extern "C" fn(*mut This, c_int, c_int),
    get_haptics_interface_address: unsafe extern "C" fn(*mut This) -> *mut c_void,
    set_novint_pure: unsafe extern "C" fn(*mut This, bool),
    get_raw_mouse_accumulators: unsafe extern "C" fn(*mut This, *mut c_int, *mut c_int) -> bool,
    get_touch_accumulators: unsafe extern "C" fn(*mut This, c_int, *mut f32, *mut f32) -> bool,
    set_console_text_mode: unsafe extern "C" fn(*mut This, bool),
    steam_controller_interface: unsafe extern "C" fn(*mut This) -> *mut c_void,
    get_num_steam_controllers_connected: unsafe extern "C" fn(*mut This) -> u32,
    is_steam_controller_active: unsafe extern "C" fn(*mut This) -> bool,
    is_steam_controller_connected: unsafe extern "C" fn(*mut This) -> bool,
    get_steam_controller_index_for_slot: unsafe extern "C" fn(*mut This, c_int) -> c_int,
    get_radial_menu_stick_values: unsafe extern "C" fn(*mut This, c_int, *mut f32, *mut f32) -> bool,
    activate_steam_controller_action_set_for_slot: unsafe extern "C" fn(*mut This, u64, c_int),
    get_action_set_handle_by_enum: unsafe extern "C" fn(*mut This, c_int) -> u64,
    get_action_set_handle_by_name: unsafe extern "C" fn(*mut This, *const c_char) -> u64,
    get_action_origin_by_enum: unsafe extern "C" fn(*mut This, *const c_char, c_int) -> c_int,
    get_action_origin_by_handle: unsafe extern "C" fn(*mut This, *const c_char, u64) -> c_int,
    get_font_character_for_action_origin: unsafe extern "C" fn(*mut This, c_int) -> *const u32,
    get_description_for_action_origin: unsafe extern "C" fn(*mut This, c_int) -> *const u32,
    set_skip_controller_initialization: unsafe extern "C" fn(*mut This, bool),
    start_text_input: unsafe extern "C" fn(*mut This),
}

static TABLE: VTable<Methods> = VTable::new(Methods {
    app_system: AppSystemMethods {
        connect,
        disconnect,
        query_interface,
        init,
        shutdown,
    },
    attach_to_window,
    detach_from_window,
    enable_input,
    enable_message_pump,
    poll_input_state,
    get_poll_tick,
    is_button_down,
    get_button_pressed_tick,
    get_button_released_tick,
    get_analog_value,
    get_analog_delta,
    get_event_count,
    get_event_data,
    post_user_event,
    get_joystick_count,
    enable_joystick_input,
    enable_joystick_diagonal_pov,
    sample_devices,
    set_rumble,
    stop_rumble,
    reset_input_state,
    set_primary_user_id,
    button_code_to_string,
    analog_code_to_string,
    string_to_button_code,
    string_to_analog_code,
    sleep_until_input,
    virtual_key_to_button_code,
    button_code_to_virtual_key,
    scan_code_to_button_code,
    get_poll_count,
    set_cursor_position,
    get_haptics_interface_address,
    set_novint_pure,
    get_raw_mouse_accumulators,
    get_touch_accumulators,
    set_console_text_mode,
    steam_controller_interface,
    get_num_steam_controllers_connected,
    is_steam_controller_active,
    is_steam_controller_connected,
    get_steam_controller_index_for_slot,
    get_radial_menu_stick_values,
    activate_steam_controller_action_set_for_slot,
    get_action_set_handle_by_enum,
    get_action_set_handle_by_name,
    get_action_origin_by_enum,
    get_action_origin_by_handle,
    get_font_character_for_action_origin,
    get_description_for_action_origin,
    set_skip_controller_initialization,
    start_text_input,
});

static INPUT_SYSTEM: This = Object::new(&TABLE);

/// The one game controller the module tracks, as `JoystickInfo_t` does.
#[derive(Debug, Clone, Copy)]
struct Joystick {
    controller: *mut c_void,
    haptic: *mut c_void,
    device_id: i32,
    rumble_enabled: bool,
    current_rumble: f32,
    diagonal_pov_enabled: bool,
}

// SAFETY: the handles are only touched from the thread that polls; `Send` is
// needed because they live in the module's state behind a mutex.
unsafe impl Send for Joystick {}

impl Joystick {
    const fn none() -> Self {
        Self {
            controller: std::ptr::null_mut(),
            haptic: std::ptr::null_mut(),
            device_id: -1,
            rumble_enabled: false,
            current_rumble: 0.0,
            diagonal_pov_enabled: false,
        }
    }
}

struct Module {
    core: InputCore,
    launcher: Option<LauncherMgr>,
    joystick: Joystick,
    watches_registered: bool,
}

static STATE: Mutex<Module> = Mutex::new(Module {
    core: InputCore::new(),
    launcher: None,
    joystick: Joystick::none(),
    watches_registered: false,
});

fn state() -> MutexGuard<'static, Module> {
    // A panic elsewhere leaves the state as it last was, which is still valid.
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Milliseconds since the machine booted, which is what `Plat_MSTime` answers
/// and all the tick arithmetic is relative to.
fn now_ms() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u32)
}

// ---------------------------------------------------------------- IAppSystem

unsafe extern "C" fn connect(_: *mut This, factory: Option<CreateInterfaceFn>) -> bool {
    guard(false, || {
        let Some(factory) = factory else {
            return false;
        };
        // The factory is called with nothing locked: an app-system group
        // resolves a name by asking every system it holds, this one included,
        // so a lock held here would be entered again from inside the call.
        // SAFETY: the group gives a factory that takes a name and a return
        // code, and a null return code is allowed.
        let found = unsafe {
            factory(
                launcher::INTERFACE_VERSION.as_ptr(),
                std::ptr::null_mut::<c_int>(),
            )
        };
        state().launcher = LauncherMgr::new(found);
        true
    })
}

unsafe extern "C" fn disconnect(_: *mut This) {
    guard((), || {
        state().launcher = None;
    });
}

unsafe extern "C" fn query_interface(_: *mut This, name: *const c_char) -> *mut c_void {
    guard(std::ptr::null_mut(), || {
        // SAFETY: the caller passes null or a NUL-terminated name.
        unsafe {
            create_interface(
                &[(INTERFACE_VERSION, INPUT_SYSTEM.as_interface())],
                name,
                std::ptr::null_mut(),
            )
        }
    })
}

unsafe extern "C" fn init(_: *mut This) -> InitReturnVal {
    guard(InitReturnVal::Failed, || {
        {
            let mut module = state();
            module.core.startup_tick = now_ms();
            // `USE_SDL` builds report raw mouse input as supported because the
            // launcher accumulates it; there is no separate raw-input device.
            module.core.raw_input_supported = true;
        }

        // The Steam controller block of `CInputSystem::Init` is not
        // reproduced: `CSteamAPIContext::Init` returns false at its first
        // line against the linked `steam_api` stub, so nothing after it runs.

        if !state().core.console_text_mode {
            initialize_touch();
            initialize_joysticks();
        }

        InitReturnVal::Ok
    })
}

unsafe extern "C" fn shutdown(_: *mut This) {
    guard((), || {
        shutdown_joysticks();
        shutdown_touch();
    });
}

// --------------------------------------------------------------- the window

unsafe extern "C" fn attach_to_window(_: *mut This, window: *mut c_void) {
    guard((), || {
        let mut module = state();
        if module.core.window_attached {
            tier0::warning("CInputSystem::AttachToWindow: Cannot attach to two windows at once!\n");
            return;
        }
        module.core.window_attached = !window.is_null();
        // A new window starts with no input held, and says so silently.
        module.core.clear_input_state();
    });
}

unsafe extern "C" fn detach_from_window(_: *mut This) {
    guard((), || {
        let mut module = state();
        if !module.core.window_attached {
            return;
        }
        module.core.reset_input_state();
        module.core.window_attached = false;
    });
}

unsafe extern "C" fn enable_input(_: *mut This, enable: bool) {
    guard((), || state().core.enabled = enable);
}

unsafe extern "C" fn enable_message_pump(_: *mut This, enable: bool) {
    guard((), || state().core.pump_enabled = enable);
}

// ---------------------------------------------------------------- the frame

unsafe extern "C" fn poll_input_state(_: *mut This) {
    guard((), || {
        let (launcher, pump) = {
            let mut module = state();
            module.core.begin_poll();
            // `SampleDevices` is where the sample tick moves; the joystick and
            // Steam polling it also does are empty here, for the reasons the
            // ledger records.
            module.core.begin_sample(now_ms());
            module.core.adopt_sample_tick();
            (module.launcher, module.core.pump_enabled)
        };

        let Some(launcher) = launcher else {
            state().core.end_poll();
            return;
        };

        // Nothing is locked across the pump. It runs SDL's event loop, which
        // calls this module's watches back on this same thread, and they take
        // the lock themselves. `m_bIsPolling` stays set across it, so what
        // they post lands in the current state, as it does in the C++.
        if pump {
            launcher.pump_windows_message_loop();
        }

        let mut events = [CocoaEvent::default(); EVENT_BATCH];
        loop {
            let count = launcher.get_events(&mut events);
            if count == 0 {
                break;
            }
            let mut module = state();
            for event in &events[..count] {
                apply_cocoa_event(&mut module.core, event);
            }
        }

        state().core.end_poll();
    });
}

/// One `CCocoaEvent` turned into state changes and events, as the body of
/// `PollInputState_Platform`'s switch does.
fn apply_cocoa_event(core: &mut InputCore, event: &CocoaEvent) {
    let tick = core.last_sample_tick();
    match event.kind {
        launcher::KEY_DOWN => {
            let code = cocoa_virtual_key_to_button_code(event.virtual_key);
            if code != codes::BUTTON_CODE_NONE {
                // Space arrives twice, once as a key and once as text, and the
                // second carries no code. Dropping the codeless one keeps
                // vgui's press/release pairing intact.
                core.post_button_pressed(state::IE_BUTTON_PRESSED, tick, code, code);
            }
            core.post_user_event(InputEvent {
                kind: state::IE_KEY_CODE_TYPED,
                tick: core.poll_tick(),
                data: code,
                ..InputEvent::default()
            });

            if event.modifier_mask & launcher::COMMAND_KEY_MASK == 0
                && event.virtual_key >= 0
                && event.unicode_key > 0
            {
                core.post_user_event(InputEvent {
                    kind: state::IE_KEY_TYPED,
                    tick: core.poll_tick(),
                    data: event.unicode_key,
                    ..InputEvent::default()
                });
            }
        }
        launcher::KEY_UP => {
            let code = cocoa_virtual_key_to_button_code(event.virtual_key);
            if code != codes::BUTTON_CODE_NONE {
                core.post_button_released(state::IE_BUTTON_RELEASED, tick, code, code);
            }
        }
        launcher::MOUSE_BUTTON_DOWN => {
            let double_click = if event.mouse_click_count > 1 {
                match event.mouse_button {
                    launcher::BUTTON_RIGHT => codes::MOUSE_RIGHT,
                    launcher::BUTTON_MIDDLE => codes::MOUSE_MIDDLE,
                    launcher::BUTTON_4 => codes::MOUSE_4,
                    launcher::BUTTON_5 => codes::MOUSE_5,
                    launcher::BUTTON_LEFT => codes::MOUSE_LEFT,
                    // The C++ switch labels its default case with left too.
                    _ => codes::MOUSE_LEFT,
                }
            } else {
                codes::BUTTON_CODE_INVALID
            };
            core.update_mouse_button_state(event.mouse_button_flags, double_click);
        }
        launcher::MOUSE_BUTTON_UP => {
            core.update_mouse_button_state(event.mouse_button_flags, codes::BUTTON_CODE_INVALID);
        }
        launcher::MOUSE_MOVE => {
            // The position is packed as two shorts and read back as shorts.
            let x = i32::from(event.mouse_pos[0] as i16);
            let y = i32::from(event.mouse_pos[1] as i16);
            core.update_mouse_position_state(x, y);
            core.post_user_event(InputEvent {
                kind: state::IE_LOCATE_MOUSE_CLICK,
                tick: core.poll_tick(),
                data: x,
                data2: y,
                ..InputEvent::default()
            });
        }
        launcher::MOUSE_SCROLL => {
            core.mouse_wheel(i32::from(event.mouse_pos[1] as i16));
        }
        launcher::APP_ACTIVATE => {
            let activated = event.modifier_mask != 0;
            core.post_user_event(InputEvent {
                kind: state::IE_APP_ACTIVATED,
                data: i32::from(activated),
                ..InputEvent::default()
            });
            if !activated {
                // Losing focus with alt held would otherwise leave vgui
                // believing alt is still down when focus comes back.
                core.reset_input_state();
            }
        }
        launcher::APP_QUIT => {
            core.post_event(state::IE_QUIT, tick, 0, 0, 0);
        }
        // `CocoaEvent_Deleted` and anything unrecognised are ignored.
        _ => {}
    }
}

unsafe extern "C" fn sample_devices(_: *mut This) {
    guard((), || {
        // Only the tick: the joystick arrives through SDL's watches rather
        // than being sampled, and the Steam controller path is inert.
        state().core.begin_sample(now_ms());
    });
}

unsafe extern "C" fn get_poll_tick(_: *mut This) -> c_int {
    guard(0, || state().core.poll_tick())
}

unsafe extern "C" fn get_poll_count(_: *mut This) -> c_int {
    guard(0, || state().core.poll_count())
}

unsafe extern "C" fn sleep_until_input(_: *mut This, max_sleep_ms: c_int) {
    guard((), || {
        let launcher = state().launcher;
        if let Some(launcher) = launcher {
            launcher.wait_until_user_input(max_sleep_ms);
        }
    });
}

// ------------------------------------------------------------ reading state

unsafe extern "C" fn is_button_down(_: *mut This, code: c_int) -> bool {
    guard(false, || state().core.is_button_down(code))
}

unsafe extern "C" fn get_button_pressed_tick(_: *mut This, code: c_int) -> c_int {
    guard(0, || state().core.button_pressed_tick(code))
}

unsafe extern "C" fn get_button_released_tick(_: *mut This, code: c_int) -> c_int {
    guard(0, || state().core.button_released_tick(code))
}

unsafe extern "C" fn get_analog_value(_: *mut This, code: c_int) -> c_int {
    guard(0, || state().core.analog_value(code))
}

unsafe extern "C" fn get_analog_delta(_: *mut This, code: c_int) -> c_int {
    guard(0, || state().core.analog_delta(code))
}

unsafe extern "C" fn get_event_count(_: *mut This) -> c_int {
    guard(0, || {
        c_int::try_from(state().core.current().events().len()).unwrap_or(c_int::MAX)
    })
}

unsafe extern "C" fn get_event_data(_: *mut This) -> *const InputEvent {
    guard(std::ptr::null(), || {
        // The pointer outlives the lock on purpose: this is the interface's
        // contract, and the double buffer is what makes it safe. Only the
        // next `PollInputState`, or a `ClearInputState` from
        // `AttachToWindow`, touches the current state's events; everything a
        // caller can do between frames writes to the queued one. The C++ has
        // exactly the same window, for the same reason.
        state().core.current().events().as_ptr()
    })
}

unsafe extern "C" fn post_user_event(_: *mut This, event: *const InputEvent) {
    guard((), || {
        if event.is_null() {
            return;
        }
        // SAFETY: the parameter is a C++ reference, so non-null and pointing
        // at a live `InputEvent_t`, which is five ints.
        let event = unsafe { *event };
        state().core.post_user_event(event);
    });
}

unsafe extern "C" fn reset_input_state(_: *mut This) {
    guard((), || state().core.reset_input_state());
}

unsafe extern "C" fn set_primary_user_id(_: *mut This, user_id: c_int) {
    guard((), || state().core.set_primary_user_id(user_id));
}

unsafe extern "C" fn set_cursor_position(_: *mut This, x: c_int, y: c_int) {
    guard((), || {
        let (launcher, attached) = {
            let module = state();
            (module.launcher, module.core.window_attached)
        };
        if !attached {
            return;
        }
        if let Some(launcher) = launcher {
            launcher.set_cursor_position(x, y);
        }
        state().core.set_cursor_position_state(x, y);
    });
}

unsafe extern "C" fn get_raw_mouse_accumulators(
    _: *mut This,
    x: *mut c_int,
    y: *mut c_int,
) -> bool {
    guard(false, || {
        let launcher = state().launcher;
        let Some(launcher) = launcher else {
            return false;
        };
        let (dx, dy) = launcher.mouse_delta();
        // SAFETY: both are C++ references, so non-null and writable.
        unsafe {
            if !x.is_null() {
                *x = dx;
            }
            if !y.is_null() {
                *y = dy;
            }
        }
        true
    })
}

unsafe extern "C" fn get_touch_accumulators(
    _: *mut This,
    finger: c_int,
    dx: *mut f32,
    dy: *mut f32,
) -> bool {
    guard(false, || {
        let (x, y) = state().core.take_touch_accumulator(finger);
        // SAFETY: both are C++ references, so non-null and writable.
        unsafe {
            if !dx.is_null() {
                *dx = x;
            }
            if !dy.is_null() {
                *dy = y;
            }
        }
        // The C++ answers true whatever the finger, including one it ignored.
        true
    })
}

// -------------------------------------------------------------- conversions

unsafe extern "C" fn button_code_to_string(_: *mut This, code: c_int) -> *const c_char {
    guard(c"".as_ptr(), || {
        state().core.button_code_to_string(code).as_ptr()
    })
}

unsafe extern "C" fn analog_code_to_string(_: *mut This, code: c_int) -> *const c_char {
    guard(c"".as_ptr(), || {
        state().core.analog_code_to_string(code).as_ptr()
    })
}

unsafe extern "C" fn string_to_button_code(_: *mut This, name: *const c_char) -> c_int {
    guard(codes::BUTTON_CODE_INVALID, || {
        let Some(name) = borrow(name) else {
            return codes::BUTTON_CODE_INVALID;
        };
        state().core.string_to_button_code(name)
    })
}

unsafe extern "C" fn string_to_analog_code(_: *mut This, name: *const c_char) -> c_int {
    guard(codes::ANALOG_CODE_INVALID, || {
        let Some(name) = borrow(name) else {
            return codes::ANALOG_CODE_INVALID;
        };
        codes::string_to_analog_code(name)
    })
}

unsafe extern "C" fn virtual_key_to_button_code(_: *mut This, key: c_int) -> c_int {
    guard(codes::KEY_NONE, || codes::virtual_key_to_button_code(key))
}

unsafe extern "C" fn button_code_to_virtual_key(_: *mut This, code: c_int) -> c_int {
    guard(0, || codes::button_code_to_virtual_key(code))
}

unsafe extern "C" fn scan_code_to_button_code(_: *mut This, lparam: c_int) -> c_int {
    guard(codes::KEY_NONE, || codes::scan_code_to_button_code(lparam))
}

/// A `const char *` parameter as a string, or nothing for a null.
fn borrow<'a>(name: *const c_char) -> Option<&'a CStr> {
    // SAFETY: the engine passes null or a NUL-terminated string that outlives
    // the call, which is all the borrow is used for.
    (!name.is_null()).then(|| unsafe { CStr::from_ptr(name) })
}

// ---------------------------------------------------------------- joysticks

unsafe extern "C" fn get_joystick_count(_: *mut This) -> c_int {
    guard(0, || state().core.joystick_count)
}

unsafe extern "C" fn enable_joystick_input(_: *mut This, joystick: c_int, enable: bool) {
    guard((), || {
        state().core.enable_joystick_input(joystick, enable);
    });
}

unsafe extern "C" fn enable_joystick_diagonal_pov(_: *mut This, joystick: c_int, enable: bool) {
    guard((), || {
        if joystick == 0 {
            state().joystick.diagonal_pov_enabled = enable;
        }
    });
}

unsafe extern "C" fn set_rumble(_: *mut This, left: f32, right: f32, _user_id: c_int) {
    guard((), || set_rumble_impl((left + right) / 2.0));
}

unsafe extern "C" fn stop_rumble(_: *mut This) {
    guard((), || {
        // The C++ calls SetRumble(0, 0, i) once per user; off Windows the user
        // is ignored, so the effect is one stop.
        set_rumble_impl(0.0);
    });
}

fn set_rumble_impl(strength: f32) {
    let Some(sdl) = sdl::sdl() else {
        return;
    };
    let mut module = state();
    let joystick = module.joystick;
    if joystick.device_id < 0 || joystick.haptic.is_null() {
        return;
    }

    // Below a hundredth, stop rather than play: SDL treats a zero strength as
    // an error rather than as silence.
    if strength < 0.01 {
        if joystick.rumble_enabled {
            // SAFETY: the handle came from `open_rumble` and is still open.
            unsafe { sdl.rumble_stop(joystick.haptic) };
            module.joystick.rumble_enabled = false;
            module.joystick.current_rumble = 0.0;
        }
        return;
    }

    if joystick.rumble_enabled && (joystick.current_rumble - strength).abs() < 0.01 {
        return;
    }
    module.joystick.rumble_enabled = true;
    module.joystick.current_rumble = strength;
    drop(module);

    // SAFETY: the handle came from `open_rumble` and is still open.
    if !unsafe { sdl.rumble_play(joystick.haptic, strength) } {
        tier0::warning(&format!(
            "Couldn't play rumble (strength {strength:.1}): {}\n",
            sdl.error()
        ));
    }
}

fn initialize_joysticks() {
    let Some(sdl) = sdl::sdl() else {
        return;
    };
    if state().core.joystick_initialized {
        shutdown_joysticks();
    }

    if !sdl.init_subsystem(sdl::INIT_GAMECONTROLLER | sdl::INIT_HAPTIC) {
        tier0::warning(&format!(
            "Joystick init failed -- SDL_Init(SDL_INIT_GAMECONTROLLER|SDL_INIT_HAPTIC) failed: {}.\n",
            sdl.error()
        ));
        return;
    }
    state().core.joystick_initialized = true;

    // SAFETY: the userdata is null, so nothing has to outlive the watch; the
    // module's state is a static reached through `state()` instead.
    unsafe { sdl.add_event_watch(joystick_watch, std::ptr::null_mut()) };
    state().watches_registered = true;

    let total = sdl.num_joysticks();
    for index in 0..total {
        if sdl.is_game_controller(index) {
            hotplug_added(index);
        }
    }
}

fn shutdown_joysticks() {
    let Some(sdl) = sdl::sdl() else {
        return;
    };
    if !state().core.joystick_initialized {
        return;
    }
    sdl.del_event_watch(joystick_watch, std::ptr::null_mut());

    let device_id = state().joystick.device_id;
    if device_id >= 0 {
        hotplug_removed(device_id);
    }
    sdl.quit_subsystem(sdl::INIT_GAMECONTROLLER | sdl::INIT_HAPTIC);

    let mut module = state();
    module.core.joystick_initialized = false;
    module.watches_registered = false;
}

fn initialize_touch() {
    let Some(sdl) = sdl::sdl() else {
        return;
    };
    // SAFETY: the userdata is null, as above.
    unsafe { sdl.add_event_watch(touch_watch, std::ptr::null_mut()) };
    state().core.touch_initialized = true;
}

fn shutdown_touch() {
    let Some(sdl) = sdl::sdl() else {
        return;
    };
    if !state().core.touch_initialized {
        return;
    }
    sdl.del_event_watch(touch_watch, std::ptr::null_mut());
    state().core.touch_initialized = false;
}

fn hotplug_added(index: c_int) {
    let Some(sdl) = sdl::sdl() else {
        return;
    };
    if !sdl.is_game_controller(index) {
        tier0::warning(
            "Joystick is not recognized by the game controller system. You can configure the controller in Steam Big Picture mode.\n",
        );
        return;
    }
    let Some(joystick_id) = sdl.joystick_instance_id(index) else {
        tier0::warning(&format!(
            "Could not open joystick {index}: {}\n",
            sdl.error()
        ));
        return;
    };

    {
        let module = state();
        // Only one device is tracked, and the first one found keeps the slot.
        if module.joystick.device_id != -1 {
            if module.joystick.device_id == joystick_id {
                return;
            }
            drop(module);
            hotplug_removed(state().joystick.device_id);
        }
    }

    let Some((controller, device_id)) = sdl.open_controller(index) else {
        tier0::warning(&format!(
            "Failed to open joystick {joystick_id}: {}\n",
            sdl.error()
        ));
        return;
    };
    // SAFETY: the controller was just opened and has not been closed.
    let haptic = unsafe { sdl.open_rumble(controller) };
    if haptic.is_none() {
        tier0::warning(&format!(
            "Unable to initialize rumble for joystick #{joystick_id}: {}\n",
            sdl.error()
        ));
    }

    let mut module = state();
    module.joystick = Joystick {
        controller,
        haptic: haptic.unwrap_or(std::ptr::null_mut()),
        device_id,
        ..Joystick::none()
    };
    module.core.enable_joystick_input(0, true);
    module.core.joystick_count = 1;
    module.core.x_controller = true;
}

fn hotplug_removed(joystick_id: i32) {
    let Some(sdl) = sdl::sdl() else {
        return;
    };
    let mut module = state();
    if module.joystick.device_id != joystick_id {
        return;
    }
    let joystick = module.joystick;
    module.joystick = Joystick::none();
    module.core.joystick_count = 0;
    module.core.x_controller = false;
    module.core.enable_joystick_input(0, false);
    drop(module);

    if !joystick.controller.is_null() {
        // SAFETY: both handles were opened here and are being given up.
        unsafe { sdl.close_controller(joystick.controller, joystick.haptic) };
    }
}

/// `JoystickSDLWatcher`. SDL calls this from inside whoever pumps the queue,
/// which is the launcher, on the main thread, from inside `PollInputState`.
unsafe extern "C" fn joystick_watch(_: *mut c_void, event: *mut sdl::Event) -> c_int {
    guard(1, || {
        if event.is_null() {
            return 1;
        }
        // SAFETY: SDL passes a live event for the duration of the callback.
        let event = unsafe { &*event };
        match event.kind() {
            sdl::CONTROLLERAXISMOTION => axis_motion(event),
            sdl::CONTROLLERBUTTONDOWN | sdl::CONTROLLERBUTTONUP => {
                button_event(event, event.kind() == sdl::CONTROLLERBUTTONDOWN);
            }
            sdl::CONTROLLERDEVICEADDED => hotplug_added(event.which()),
            sdl::CONTROLLERDEVICEREMOVED => hotplug_removed(event.which()),
            _ => {}
        }
        1
    })
}

fn button_event(event: &sdl::Event, pressed: bool) {
    let mut module = state();
    if module.joystick.device_id != event.which() {
        return;
    }
    let code = state::controller_button_to_button_code(c_int::from(event.axis_or_button()));
    let tick = module.core.last_sample_tick();
    if pressed {
        module
            .core
            .post_button_pressed(state::IE_BUTTON_PRESSED, tick, code, code);
    } else {
        module
            .core
            .post_button_released(state::IE_BUTTON_RELEASED, tick, code, code);
    }
}

fn axis_motion(event: &sdl::Event) {
    let mut module = state();
    if module.joystick.device_id != event.which() {
        return;
    }
    let Some(axis) = state::GameControllerAxis::from_sdl(c_int::from(event.axis_or_button()))
    else {
        return;
    };
    let Some((code, button)) = state::controller_axis(axis) else {
        return;
    };
    // The thresholds are the `joy_axisbutton_threshold` and `joy_axis_deadzone`
    // convars' defaults. Reading the live values would mean linking `vstdlib`,
    // which would put C++ back in this module; the defaults are what a player
    // who has not changed them gets, which is everyone by default.
    const AXIS_BUTTON_THRESHOLD: i32 = (0.3 * 32767.0) as i32;
    const AXIS_DEAD_ZONE: i32 = (0.2 * 32767.0) as i32;
    module.core.joystick_axis_motion(
        code,
        i32::from(event.axis_value()),
        button,
        AXIS_BUTTON_THRESHOLD,
        AXIS_DEAD_ZONE,
    );
}

/// `TouchSDLWatcher`.
unsafe extern "C" fn touch_watch(_: *mut c_void, event: *mut sdl::Event) -> c_int {
    guard(1, || {
        if event.is_null() {
            return 1;
        }
        // SAFETY: SDL passes a live event for the duration of the callback.
        let event = unsafe { &*event };
        let kind = match event.kind() {
            sdl::FINGERDOWN => state::IE_FINGER_DOWN,
            sdl::FINGERUP => state::IE_FINGER_UP,
            sdl::FINGERMOTION => state::IE_FINGER_MOTION,
            _ => return 1,
        };
        let (x, y, dx, dy) = event.finger_motion();
        let finger = event.finger_id() as i32;
        state().core.finger_event(kind, finger, x, y, dx, dy);
        1
    })
}

unsafe extern "C" fn set_console_text_mode(_: *mut This, console_text_mode: bool) {
    guard((), || {
        let initialized = {
            let mut module = state();
            module.core.console_text_mode = console_text_mode;
            module.core.joystick_initialized
        };
        // Called after `Init` on a dedicated server, which wants no joystick.
        if console_text_mode && initialized {
            shutdown_joysticks();
        }
    });
}

// ------------------------------------------------------ the inert Steam half

unsafe extern "C" fn steam_controller_interface(_: *mut This) -> *mut c_void {
    // Null, because `CSteamAPIContext::Init` fails at its first line against
    // the `steam_api` stub this tree links.
    std::ptr::null_mut()
}

unsafe extern "C" fn get_num_steam_controllers_connected(_: *mut This) -> u32 {
    0
}

unsafe extern "C" fn is_steam_controller_active(_: *mut This) -> bool {
    false
}

unsafe extern "C" fn is_steam_controller_connected(_: *mut This) -> bool {
    false
}

unsafe extern "C" fn get_steam_controller_index_for_slot(_: *mut This, _slot: c_int) -> c_int {
    // No pad is ever marked active, so the C++ loop finds nothing.
    -1
}

unsafe extern "C" fn get_radial_menu_stick_values(
    _: *mut This,
    _slot: c_int,
    x: *mut f32,
    y: *mut f32,
) -> bool {
    guard(false, || {
        // SAFETY: both are C++ references, so non-null and writable.
        unsafe {
            if !x.is_null() {
                *x = 0.0;
            }
            if !y.is_null() {
                *y = 0.0;
            }
        }
        // True even with nothing connected, which is what the C++ returns.
        true
    })
}

unsafe extern "C" fn activate_steam_controller_action_set_for_slot(
    _: *mut This,
    _slot: u64,
    _action_set: c_int,
) {
    // The C++ only records a change when the interface is live, and the
    // debounce flags it would set are read by nothing else here.
}

unsafe extern "C" fn get_action_set_handle_by_enum(_: *mut This, _action_set: c_int) -> u64 {
    // Handles are only ever filled in from Steam, so all of them are zero.
    0
}

unsafe extern "C" fn get_action_set_handle_by_name(_: *mut This, name: *const c_char) -> u64 {
    guard(0, || {
        // The lookup answers zero either way; it is kept so the name still has
        // to be one of the four, ready for a build with Steam behind it.
        borrow(name).and_then(steam::action_set_index).map_or(0, |_| 0)
    })
}

unsafe extern "C" fn get_action_origin_by_enum(
    _: *mut This,
    _action: *const c_char,
    _action_set: c_int,
) -> c_int {
    steam::ACTION_ORIGIN_NONE
}

unsafe extern "C" fn get_action_origin_by_handle(
    _: *mut This,
    _action: *const c_char,
    _handle: u64,
) -> c_int {
    steam::ACTION_ORIGIN_NONE
}

unsafe extern "C" fn get_font_character_for_action_origin(
    _: *mut This,
    origin: c_int,
) -> *const u32 {
    guard(std::ptr::null(), || {
        steam::origin_icon_font(origin).as_ptr()
    })
}

unsafe extern "C" fn get_description_for_action_origin(_: *mut This, origin: c_int) -> *const u32 {
    guard(std::ptr::null(), || {
        steam::origin_description(origin).as_ptr()
    })
}

unsafe extern "C" fn set_skip_controller_initialization(_: *mut This, skip: bool) {
    guard((), || {
        state().core.skip_controller_initialization = skip;
    });
}

// ------------------------------------------------------------------ the rest

unsafe extern "C" fn get_haptics_interface_address(_: *mut This) -> *mut c_void {
    // Novint haptics are Windows-only.
    std::ptr::null_mut()
}

unsafe extern "C" fn set_novint_pure(_: *mut This, _pure: bool) {}

unsafe extern "C" fn start_text_input(_: *mut This) {
    guard((), || {
        if let Some(sdl) = sdl::sdl() {
            sdl.start_text_input();
        }
    });
}

/// `CreateInterface`, which is how the engine finds this module.
///
/// # Safety
///
/// `name` must be null or a NUL-terminated string, and `return_code` null or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn CreateInterface(
    name: *const c_char,
    return_code: *mut c_int,
) -> *mut c_void {
    // SAFETY: the contract is the caller's, and is the one `create_interface`
    // documents.
    unsafe {
        create_interface(
            &[(INTERFACE_VERSION, INPUT_SYSTEM.as_interface())],
            name,
            return_code,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_has_the_fifty_seven_slots_the_interface_declares() {
        // `IInputSystem`'s vtable is 59 entries: offset_to_top, RTTI and 57
        // functions. `Methods` is only the functions.
        assert_eq!(
            size_of::<Methods>() / size_of::<*const c_void>(),
            57,
            "a slot was added or dropped"
        );
    }

    #[test]
    fn create_interface_answers_its_own_name_and_nothing_else() {
        let mut code = -1;
        // SAFETY: both arguments are valid for the call.
        let found = unsafe { CreateInterface(INTERFACE_VERSION.as_ptr(), &raw mut code) };
        assert!(!found.is_null());
        assert_eq!(code, source_cppabi::IFACE_OK);

        // SAFETY: as above.
        let missing = unsafe { CreateInterface(c"InputSystemVersion002".as_ptr(), &raw mut code) };
        assert!(missing.is_null());
        assert_eq!(code, source_cppabi::IFACE_FAILED);
    }

    #[test]
    fn the_object_handed_out_points_at_the_table() {
        let object = INPUT_SYSTEM.as_interface();
        // SAFETY: the object was built over this table.
        let stored = unsafe { *object.cast::<*const Methods>() };
        assert_eq!(stored, TABLE.address());
    }

    #[test]
    fn the_steam_slots_answer_what_a_null_interface_makes_the_cpp_answer() {
        let this = INPUT_SYSTEM.as_interface().cast::<This>();
        // SAFETY: every one of these is a slot of the live static object.
        unsafe {
            assert!(steam_controller_interface(this).is_null());
            assert_eq!(get_num_steam_controllers_connected(this), 0);
            assert!(!is_steam_controller_active(this));
            assert!(!is_steam_controller_connected(this));
            assert_eq!(get_steam_controller_index_for_slot(this, 0), -1);
            assert_eq!(get_action_set_handle_by_enum(this, 0), 0);
            assert_eq!(
                get_action_set_handle_by_name(this, c"MenuControls".as_ptr()),
                0
            );
            assert_eq!(
                get_action_origin_by_enum(this, c"menu_select".as_ptr(), 0),
                steam::ACTION_ORIGIN_NONE
            );

            let mut x = 9.0f32;
            let mut y = 9.0f32;
            assert!(get_radial_menu_stick_values(
                this,
                0,
                &raw mut x,
                &raw mut y
            ));
            assert_eq!((x, y), (0.0, 0.0));
        }
    }

    #[test]
    fn the_origin_tables_answer_through_the_slots() {
        let this = INPUT_SYSTEM.as_interface().cast::<This>();
        // SAFETY: both are slots of the live static object.
        unsafe {
            let icon = get_font_character_for_action_origin(this, 1);
            assert!(!icon.is_null());
            assert_eq!(*icon, u32::from(b'A'));

            let description = get_description_for_action_origin(this, 5);
            assert!(!description.is_null());
            assert_eq!(*description, u32::from(b'L'));
            assert_eq!(*description.add(1), u32::from(b'B'));
            assert_eq!(*description.add(2), 0);
        }
    }

    #[test]
    fn conversions_answer_through_the_slots() {
        let this = INPUT_SYSTEM.as_interface().cast::<This>();
        // SAFETY: every one of these is a slot of the live static object.
        unsafe {
            let name = button_code_to_string(this, codes::KEY_A);
            assert_eq!(CStr::from_ptr(name), c"a");
            assert_eq!(string_to_button_code(this, c"a".as_ptr()), codes::KEY_A);
            assert_eq!(
                string_to_button_code(this, std::ptr::null()),
                codes::BUTTON_CODE_INVALID
            );
            assert_eq!(
                CStr::from_ptr(analog_code_to_string(this, codes::MOUSE_X)),
                c"MOUSE_X"
            );
            assert_eq!(
                string_to_analog_code(this, c"MOUSE_WHEEL".as_ptr()),
                codes::MOUSE_WHEEL
            );
            assert_eq!(
                virtual_key_to_button_code(this, i32::from(b'A')),
                codes::KEY_A
            );
            assert_eq!(
                button_code_to_virtual_key(this, codes::KEY_A),
                i32::from(b'A')
            );
        }
    }

    #[test]
    fn a_user_event_reaches_the_engine_on_the_next_poll() {
        let this = INPUT_SYSTEM.as_interface().cast::<This>();
        // SAFETY: every one of these is a slot of the live static object.
        unsafe {
            // No launcher is connected in a test, so the poll does the state
            // work and skips the pump.
            let event = InputEvent {
                kind: state::IE_QUIT,
                ..InputEvent::default()
            };
            post_user_event(this, &raw const event);
            poll_input_state(this);

            assert_eq!(get_event_count(this), 1);
            let data = get_event_data(this);
            assert!(!data.is_null());
            assert_eq!((*data).kind, state::IE_QUIT);

            poll_input_state(this);
            assert_eq!(get_event_count(this), 0, "delivered once");
        }
    }
}
