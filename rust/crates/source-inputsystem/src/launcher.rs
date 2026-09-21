//! `ILauncherMgr`, the interface this module gets its keyboard and mouse from.
//!
//! On this platform the input module does not read SDL for keyboard, mouse,
//! focus or quit. The launcher owns the window and the event pump and hands
//! those out as `CCocoaEvent`s, so `SDLMgrInterface001` is the real source of
//! everything a player does with the keyboard and mouse, and SDL only carries
//! game controllers and touch.
//!
//! The slot numbers are the ones `clang -Xclang -fdump-vtable-layouts` prints
//! for `ILauncherMgr` as this tree compiles it. That qualifier matters:
//! `DX_TO_GL_ABSTRACTION` is defined here and adds eight slots in the middle
//! of the class, so the slots after it are not where the header alone
//! suggests.

use source_cppabi::slot;
use std::ffi::{c_int, c_void};

/// `SDLMGR_INTERFACE_VERSION`.
pub const INTERFACE_VERSION: &std::ffi::CStr = c"SDLMgrInterface001";

/// `CocoaEvent_KeyDown`.
pub const KEY_DOWN: i32 = 0;
/// `CocoaEvent_KeyUp`.
pub const KEY_UP: i32 = 1;
/// `CocoaEvent_MouseButtonDown`.
pub const MOUSE_BUTTON_DOWN: i32 = 2;
/// `CocoaEvent_MouseMove`.
pub const MOUSE_MOVE: i32 = 3;
/// `CocoaEvent_MouseButtonUp`.
pub const MOUSE_BUTTON_UP: i32 = 4;
/// `CocoaEvent_AppActivate`.
pub const APP_ACTIVATE: i32 = 5;
/// `CocoaEvent_MouseScroll`.
pub const MOUSE_SCROLL: i32 = 6;
/// `CocoaEvent_AppQuit`.
pub const APP_QUIT: i32 = 7;
// `CocoaEvent_Deleted` is 8: an event that has already been handled. It is not
// named here because it falls into the same ignored arm as an event type this
// module does not recognise.

/// `eCommandKey`, as a bit in `m_ModifierKeyMask`.
pub const COMMAND_KEY_MASK: u32 = 1 << 4;

/// `COCOABUTTON_LEFT`.
pub const BUTTON_LEFT: i32 = 1 << 0;
/// `COCOABUTTON_RIGHT`.
pub const BUTTON_RIGHT: i32 = 1 << 1;
/// `COCOABUTTON_MIDDLE`.
pub const BUTTON_MIDDLE: i32 = 1 << 2;
/// `COCOABUTTON_4`.
pub const BUTTON_4: i32 = 1 << 3;
/// `COCOABUTTON_5`.
pub const BUTTON_5: i32 = 1 << 4;

/// How many events one `GetEvents` call asks for, matching the C++ array.
pub const EVENT_BATCH: usize = 32;

/// `CCocoaEvent`: nine four-byte fields with no padding between them.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CocoaEvent {
    /// `m_EventType`.
    pub kind: i32,
    /// `m_VirtualKeyCode`, an SDL scancode when positive and a button code
    /// already negated when not.
    pub virtual_key: i32,
    /// `m_UnicodeKey`, a `wchar_t`, which is four bytes here.
    pub unicode_key: i32,
    /// `m_UnicodeKeyUnmodified`.
    pub unicode_key_unmodified: i32,
    /// `m_ModifierKeyMask`.
    pub modifier_mask: u32,
    /// `m_MousePos`, which carries the scroll amount for a scroll event.
    pub mouse_pos: [i32; 2],
    /// `m_MouseButtonFlags`, the current state of all mouse buttons.
    pub mouse_button_flags: i32,
    /// `m_nMouseClickCount`.
    pub mouse_click_count: u32,
    /// `m_MouseButton`, which of `COCOABUTTON_*` this event is for.
    pub mouse_button: i32,
}

/// The launcher, as the vtable slots this module calls.
#[derive(Debug, Clone, Copy)]
pub struct LauncherMgr {
    object: *mut c_void,
}

// SAFETY: the pointer is only ever used from the thread that polls, which is
// the engine's main thread; `Send` is needed because it is stored in the
// module's state, which lives behind a mutex.
unsafe impl Send for LauncherMgr {}

impl LauncherMgr {
    /// Adopts the object a factory answered with, or nothing for a null.
    #[must_use]
    pub fn new(object: *mut c_void) -> Option<Self> {
        (!object.is_null()).then_some(Self { object })
    }

    /// `GetEvents`, slot 8: fills `events` and answers how many it wrote.
    ///
    /// The C++ declares a defaulted third argument, which is part of the
    /// signature the compiler emitted, so it is passed explicitly.
    pub fn get_events(&self, events: &mut [CocoaEvent; EVENT_BATCH]) -> usize {
        // SAFETY: slot 8 of a live `ILauncherMgr` has this signature, and the
        // buffer is `EVENT_BATCH` long, which is the count passed with it.
        let count = unsafe {
            let get_events: unsafe extern "C" fn(
                *mut c_void,
                *mut CocoaEvent,
                c_int,
                bool,
            ) -> c_int = slot(self.object, 8);
            get_events(
                self.object,
                events.as_mut_ptr(),
                EVENT_BATCH as c_int,
                false,
            )
        };
        count.clamp(0, EVENT_BATCH as c_int) as usize
    }

    /// `SetCursorPosition`, slot 9.
    pub fn set_cursor_position(&self, x: c_int, y: c_int) {
        // SAFETY: slot 9 of a live `ILauncherMgr` has this signature.
        unsafe {
            let set: unsafe extern "C" fn(*mut c_void, c_int, c_int) = slot(self.object, 9);
            set(self.object, x, y);
        }
    }

    /// `PumpWindowsMessageLoop`, slot 14.
    ///
    /// This is the call that runs SDL's pump, so SDL invokes this module's
    /// event watches from inside it. Nothing may be locked across it.
    pub fn pump_windows_message_loop(&self) {
        // SAFETY: slot 14 of a live `ILauncherMgr` has this signature.
        unsafe {
            let pump: unsafe extern "C" fn(*mut c_void) = slot(self.object, 14);
            pump(self.object);
        }
    }

    /// `GetMouseDelta`, slot 17, which reads and clears the accumulators.
    pub fn mouse_delta(&self) -> (c_int, c_int) {
        let mut x = 0;
        let mut y = 0;
        // SAFETY: slot 17 of a live `ILauncherMgr` has this signature; both
        // out-parameters are references to live locals, and the defaulted
        // third argument is part of the emitted signature.
        unsafe {
            let get: unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int, bool) =
                slot(self.object, 17);
            get(self.object, &raw mut x, &raw mut y, false);
        }
        (x, y)
    }

    /// `WaitUntilUserInput`, slot 30.
    ///
    /// Slot 30, not 22: `DX_TO_GL_ABSTRACTION` puts eight context slots ahead
    /// of it. Reading the header without that define would call `GetDisplayDB`.
    pub fn wait_until_user_input(&self, max_sleep_ms: c_int) {
        // SAFETY: slot 30 of a live `ILauncherMgr` has this signature.
        unsafe {
            let wait: unsafe extern "C" fn(*mut c_void, c_int) = slot(self.object, 30);
            wait(self.object, max_sleep_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cocoa_event_is_the_nine_four_byte_fields_the_class_declares() {
        assert_eq!(size_of::<CocoaEvent>(), 40);
        assert_eq!(align_of::<CocoaEvent>(), 4);
        assert_eq!(std::mem::offset_of!(CocoaEvent, kind), 0);
        assert_eq!(std::mem::offset_of!(CocoaEvent, virtual_key), 4);
        assert_eq!(std::mem::offset_of!(CocoaEvent, unicode_key), 8);
        assert_eq!(std::mem::offset_of!(CocoaEvent, modifier_mask), 16);
        assert_eq!(std::mem::offset_of!(CocoaEvent, mouse_pos), 20);
        assert_eq!(std::mem::offset_of!(CocoaEvent, mouse_button_flags), 28);
        assert_eq!(std::mem::offset_of!(CocoaEvent, mouse_click_count), 32);
        assert_eq!(std::mem::offset_of!(CocoaEvent, mouse_button), 36);
    }

    #[test]
    fn a_null_factory_answer_is_no_launcher_rather_than_a_null_call() {
        assert!(LauncherMgr::new(std::ptr::null_mut()).is_none());
    }
}
