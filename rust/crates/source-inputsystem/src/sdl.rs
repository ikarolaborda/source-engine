//! SDL, reached through the copy the process already loaded.
//!
//! The input module is a guest in somebody else's SDL. The launcher creates
//! the window and owns the event pump; this module only asks for the
//! game-controller and haptic subsystems, hangs two watches on the queue, and
//! turns text input on. Two things follow, and both are why the symbols here
//! are looked up rather than linked:
//!
//! * Binding to the running process's SDL is then structural rather than
//!   something a link line has to be trusted to get right, so the module
//!   cannot end up driving a second SDL beside the launcher's.
//! * There is no build script and no link path to keep in step with whichever
//!   SDL the tree is built against.
//!
//! The layouts below were measured against the SDL2 headers this tree builds
//! with — Homebrew's `sdl2-compat`, which is the SDL2 API over SDL 3 — with
//! `offsetof`, not assumed from the struct declarations.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::OnceLock;

/// `SDL_INIT_GAMECONTROLLER`.
pub const INIT_GAMECONTROLLER: u32 = 0x2000;
/// `SDL_INIT_HAPTIC`.
pub const INIT_HAPTIC: u32 = 0x1000;
/// `SDL_HAPTIC_INFINITY`.
pub const HAPTIC_INFINITY: u32 = u32::MAX;

/// `SDL_CONTROLLERAXISMOTION`.
pub const CONTROLLERAXISMOTION: u32 = 1616;
/// `SDL_CONTROLLERBUTTONDOWN`.
pub const CONTROLLERBUTTONDOWN: u32 = 1617;
/// `SDL_CONTROLLERBUTTONUP`.
pub const CONTROLLERBUTTONUP: u32 = 1618;
/// `SDL_CONTROLLERDEVICEADDED`.
pub const CONTROLLERDEVICEADDED: u32 = 1619;
/// `SDL_CONTROLLERDEVICEREMOVED`.
pub const CONTROLLERDEVICEREMOVED: u32 = 1620;
/// `SDL_FINGERDOWN`.
pub const FINGERDOWN: u32 = 1792;
/// `SDL_FINGERUP`.
pub const FINGERUP: u32 = 1793;
/// `SDL_FINGERMOTION`.
pub const FINGERMOTION: u32 = 1794;

/// `SDL_Event`, as much of it as this module reads.
///
/// The union is 56 bytes and every arm starts with the type. The fields are
/// reached by offset rather than by declaring each arm, because only six
/// fields out of the whole union are ever read and the offsets are what was
/// measured.
#[repr(C, align(8))]
pub struct Event([u8; 56]);

impl Event {
    fn at<T: Copy>(&self, offset: usize) -> T {
        debug_assert!(offset + size_of::<T>() <= self.0.len());
        // SAFETY: `offset` is within the union, which SDL filled, and every
        // field read this way is a plain integer or float at its measured
        // offset, so any bit pattern there is a valid `T`. The read is
        // unaligned because the union's arms are not all aligned for `T`.
        unsafe { self.0.as_ptr().add(offset).cast::<T>().read_unaligned() }
    }

    /// `event.type`.
    #[must_use]
    pub fn kind(&self) -> u32 {
        self.at(0)
    }

    /// `event.cdevice.which` / `event.caxis.which` / `event.cbutton.which`:
    /// the joystick instance the event came from, which all three arms put at
    /// the same offset.
    #[must_use]
    pub fn which(&self) -> i32 {
        self.at(8)
    }

    /// `event.caxis.axis` or `event.cbutton.button`, both single bytes.
    #[must_use]
    pub fn axis_or_button(&self) -> u8 {
        self.at(12)
    }

    /// `event.caxis.value`.
    #[must_use]
    pub fn axis_value(&self) -> i16 {
        self.at(16)
    }

    /// `event.tfinger.fingerId`, an `SDL_FingerID`.
    #[must_use]
    pub fn finger_id(&self) -> i64 {
        self.at(16)
    }

    /// `event.tfinger.x`, `.y`, `.dx`, `.dy`.
    #[must_use]
    pub fn finger_motion(&self) -> (f32, f32, f32, f32) {
        (self.at(24), self.at(28), self.at(32), self.at(36))
    }
}

/// `SDL_EventFilter`.
pub type EventFilter = unsafe extern "C" fn(*mut c_void, *mut Event) -> c_int;

/// The entry points this module uses, resolved once.
pub struct Sdl {
    init_subsystem: unsafe extern "C" fn(u32) -> c_int,
    quit_subsystem: unsafe extern "C" fn(u32),
    get_error: unsafe extern "C" fn() -> *const c_char,
    add_event_watch: unsafe extern "C" fn(EventFilter, *mut c_void),
    del_event_watch: unsafe extern "C" fn(EventFilter, *mut c_void),
    num_joysticks: unsafe extern "C" fn() -> c_int,
    is_game_controller: unsafe extern "C" fn(c_int) -> c_int,
    joystick_open: unsafe extern "C" fn(c_int) -> *mut c_void,
    joystick_close: unsafe extern "C" fn(*mut c_void),
    joystick_instance_id: unsafe extern "C" fn(*mut c_void) -> i32,
    game_controller_open: unsafe extern "C" fn(c_int) -> *mut c_void,
    game_controller_close: unsafe extern "C" fn(*mut c_void),
    game_controller_get_joystick: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    haptic_open_from_joystick: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    haptic_close: unsafe extern "C" fn(*mut c_void),
    haptic_rumble_init: unsafe extern "C" fn(*mut c_void) -> c_int,
    haptic_rumble_play: unsafe extern "C" fn(*mut c_void, f32, u32) -> c_int,
    haptic_rumble_stop: unsafe extern "C" fn(*mut c_void) -> c_int,
    start_text_input: unsafe extern "C" fn(),
}

/// The SDL the process is already using, or nothing outside the engine.
pub fn sdl() -> Option<&'static Sdl> {
    static SDL: OnceLock<Option<Sdl>> = OnceLock::new();
    SDL.get_or_init(|| {
        // SAFETY: each signature below is the one SDL's headers declare for
        // that symbol.
        unsafe {
            Some(Sdl {
                init_subsystem: resolve(b"SDL_InitSubSystem\0")?,
                quit_subsystem: resolve(b"SDL_QuitSubSystem\0")?,
                get_error: resolve(b"SDL_GetError\0")?,
                add_event_watch: resolve(b"SDL_AddEventWatch\0")?,
                del_event_watch: resolve(b"SDL_DelEventWatch\0")?,
                num_joysticks: resolve(b"SDL_NumJoysticks\0")?,
                is_game_controller: resolve(b"SDL_IsGameController\0")?,
                joystick_open: resolve(b"SDL_JoystickOpen\0")?,
                joystick_close: resolve(b"SDL_JoystickClose\0")?,
                joystick_instance_id: resolve(b"SDL_JoystickInstanceID\0")?,
                game_controller_open: resolve(b"SDL_GameControllerOpen\0")?,
                game_controller_close: resolve(b"SDL_GameControllerClose\0")?,
                game_controller_get_joystick: resolve(b"SDL_GameControllerGetJoystick\0")?,
                haptic_open_from_joystick: resolve(b"SDL_HapticOpenFromJoystick\0")?,
                haptic_close: resolve(b"SDL_HapticClose\0")?,
                haptic_rumble_init: resolve(b"SDL_HapticRumbleInit\0")?,
                haptic_rumble_play: resolve(b"SDL_HapticRumblePlay\0")?,
                haptic_rumble_stop: resolve(b"SDL_HapticRumbleStop\0")?,
                start_text_input: resolve(b"SDL_StartTextInput\0")?,
            })
        }
    })
    .as_ref()
}

/// One entry point, typed by the field it is being stored in.
///
/// # Safety
///
/// `F` must be the `extern "C"` signature SDL declares for `symbol`.
unsafe fn resolve<F: Copy>(symbol: &[u8]) -> Option<F> {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    let found = lookup(symbol)?;
    // SAFETY: the sizes match, and the signature is the caller's contract.
    Some(unsafe { std::mem::transmute_copy(&found) })
}

impl Sdl {
    /// `SDL_InitSubSystem`, which is ref-counted: the launcher's own
    /// initialisation keeps the outer reference and this one only adds to it.
    pub fn init_subsystem(&self, flags: u32) -> bool {
        // SAFETY: a resolved SDL entry point called with its own flags.
        unsafe { (self.init_subsystem)(flags) == 0 }
    }

    /// `SDL_QuitSubSystem`, which must be paired with the call above.
    pub fn quit_subsystem(&self, flags: u32) {
        // SAFETY: as above.
        unsafe { (self.quit_subsystem)(flags) }
    }



    /// `SDL_GetError`.
    pub fn error(&self) -> String {
        // SAFETY: SDL owns the string and it is valid until the next SDL call
        // on this thread, which is after it has been copied here.
        unsafe {
            let message = (self.get_error)();
            if message.is_null() {
                String::new()
            } else {
                CStr::from_ptr(message).to_string_lossy().into_owned()
            }
        }
    }

    /// `SDL_AddEventWatch`.
    ///
    /// # Safety
    ///
    /// `userdata` must stay valid until the matching [`Self::del_event_watch`],
    /// because SDL passes it back to `filter` on every event.
    pub unsafe fn add_event_watch(&self, filter: EventFilter, userdata: *mut c_void) {
        // SAFETY: the caller keeps `userdata` alive per the contract.
        unsafe { (self.add_event_watch)(filter, userdata) }
    }

    /// `SDL_DelEventWatch`, which must name the same pair that was added.
    pub fn del_event_watch(&self, filter: EventFilter, userdata: *mut c_void) {
        // SAFETY: removing a watch only unregisters the pair.
        unsafe { (self.del_event_watch)(filter, userdata) }
    }

    /// `SDL_NumJoysticks`.
    pub fn num_joysticks(&self) -> c_int {
        // SAFETY: a resolved entry point with no arguments.
        unsafe { (self.num_joysticks)() }
    }

    /// `SDL_IsGameController`, which does not bounds-check its own argument,
    /// so the caller does.
    pub fn is_game_controller(&self, index: c_int) -> bool {
        if index < 0 || index >= self.num_joysticks() {
            return false;
        }
        // SAFETY: the index was just checked against the device count.
        unsafe { (self.is_game_controller)(index) != 0 }
    }

    /// The instance id SDL gave the joystick at `index`, opening and closing
    /// it just to ask — which is what the C++ does, because the id is stable
    /// and the device index is not.
    pub fn joystick_instance_id(&self, index: c_int) -> Option<i32> {
        // SAFETY: open returns null on failure, and the handle is only used
        // between the open and the close.
        unsafe {
            let joystick = (self.joystick_open)(index);
            if joystick.is_null() {
                return None;
            }
            let id = (self.joystick_instance_id)(joystick);
            (self.joystick_close)(joystick);
            Some(id)
        }
    }


    /// `SDL_GameControllerOpen`, with the handle and its instance id.
    pub fn open_controller(&self, index: c_int) -> Option<(*mut c_void, i32)> {
        // SAFETY: open returns null on failure; the joystick behind an open
        // controller is owned by SDL and valid while the controller is.
        unsafe {
            let controller = (self.game_controller_open)(index);
            if controller.is_null() {
                return None;
            }
            let joystick = (self.game_controller_get_joystick)(controller);
            Some((controller, (self.joystick_instance_id)(joystick)))
        }
    }

    /// `SDL_HapticOpenFromJoystick` followed by `SDL_HapticRumbleInit`, which
    /// is only useful together: a haptic device that cannot rumble is closed.
    ///
    /// # Safety
    ///
    /// `controller` must be a live handle from [`Self::open_controller`].
    pub unsafe fn open_rumble(&self, controller: *mut c_void) -> Option<*mut c_void> {
        // SAFETY: the caller supplies a live controller per the contract.
        unsafe {
            let joystick = (self.game_controller_get_joystick)(controller);
            let haptic = (self.haptic_open_from_joystick)(joystick);
            if haptic.is_null() {
                return None;
            }
            if (self.haptic_rumble_init)(haptic) != 0 {
                (self.haptic_close)(haptic);
                return None;
            }
            Some(haptic)
        }
    }

    /// `SDL_HapticRumblePlay`.
    ///
    /// # Safety
    ///
    /// `haptic` must be a live handle from [`Self::open_rumble`].
    pub unsafe fn rumble_play(&self, haptic: *mut c_void, strength: f32) -> bool {
        // SAFETY: the caller supplies a live handle per the contract.
        unsafe { (self.haptic_rumble_play)(haptic, strength, HAPTIC_INFINITY) == 0 }
    }

    /// `SDL_HapticRumbleStop`.
    ///
    /// # Safety
    ///
    /// `haptic` must be a live handle from [`Self::open_rumble`].
    pub unsafe fn rumble_stop(&self, haptic: *mut c_void) {
        // SAFETY: the caller supplies a live handle per the contract.
        unsafe {
            (self.haptic_rumble_stop)(haptic);
        }
    }

    /// Closes a controller and its haptic device, in that order.
    ///
    /// # Safety
    ///
    /// Both handles must be live and not used again afterwards.
    pub unsafe fn close_controller(&self, controller: *mut c_void, haptic: *mut c_void) {
        // SAFETY: the caller supplies live handles per the contract.
        unsafe {
            if !haptic.is_null() {
                (self.haptic_close)(haptic);
            }
            if !controller.is_null() {
                (self.game_controller_close)(controller);
            }
        }
    }

    /// `SDL_StartTextInput`.
    pub fn start_text_input(&self) {
        // SAFETY: a resolved entry point with no arguments.
        unsafe { (self.start_text_input)() }
    }
}

/// Looks a symbol up across everything the process has loaded.
///
/// `RTLD_DEFAULT` is the right scope here, unlike for `tier0`'s `Error`: an
/// `SDL_`-prefixed name belongs to SDL wherever it was loaded from, and
/// finding the process's own copy is the point.
fn lookup(symbol: &[u8]) -> Option<*mut c_void> {
    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    const RTLD_DEFAULT: *mut c_void = std::ptr::without_provenance_mut(-2isize as usize);

    debug_assert_eq!(symbol.last(), Some(&0));
    // SAFETY: the name is NUL-terminated, and `RTLD_DEFAULT` is the documented
    // pseudo-handle for the process's own search order.
    let found = unsafe { dlsym(RTLD_DEFAULT, symbol.as_ptr().cast::<c_char>()) };
    (!found.is_null()).then_some(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_union_is_the_size_sdl_declares() {
        assert_eq!(size_of::<Event>(), 56);
        assert_eq!(align_of::<Event>(), 8);
    }

    #[test]
    fn fields_are_read_from_the_offsets_that_were_measured() {
        let mut bytes = [0u8; 56];
        bytes[0..4].copy_from_slice(&CONTROLLERAXISMOTION.to_ne_bytes());
        bytes[8..12].copy_from_slice(&7i32.to_ne_bytes());
        bytes[12] = 4;
        bytes[16..18].copy_from_slice(&(-3000i16).to_ne_bytes());
        let event = Event(bytes);
        assert_eq!(event.kind(), CONTROLLERAXISMOTION);
        assert_eq!(event.which(), 7);
        assert_eq!(event.axis_or_button(), 4);
        assert_eq!(event.axis_value(), -3000);
    }

    #[test]
    fn finger_fields_are_read_from_their_own_offsets() {
        let mut bytes = [0u8; 56];
        bytes[0..4].copy_from_slice(&FINGERMOTION.to_ne_bytes());
        bytes[16..24].copy_from_slice(&2i64.to_ne_bytes());
        bytes[24..28].copy_from_slice(&0.25f32.to_ne_bytes());
        bytes[28..32].copy_from_slice(&0.5f32.to_ne_bytes());
        bytes[32..36].copy_from_slice(&(-0.125f32).to_ne_bytes());
        bytes[36..40].copy_from_slice(&0.0625f32.to_ne_bytes());
        let event = Event(bytes);
        assert_eq!(event.kind(), FINGERMOTION);
        assert_eq!(event.finger_id(), 2);
        assert_eq!(event.finger_motion(), (0.25, 0.5, -0.125, 0.0625));
    }

    #[test]
    fn sdl_is_absent_outside_the_engine_rather_than_wrong() {
        // A test binary has no SDL loaded, so the lookup must answer nothing
        // instead of resolving something else.
        assert!(lookup(b"SDL_ThisSymbolDoesNotExist\0").is_none());
    }
}
