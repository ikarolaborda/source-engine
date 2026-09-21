//! What `CInputSystem` remembers between calls, with no FFI in it.
//!
//! The engine asks this module two kinds of question: what is the input doing
//! right now, and what happened since the last frame. Both are answered from
//! two [`Slot`]s. Everything that *writes* goes to the slot
//! `self.states[self.polling]` — `QUEUED` between frames, `CURRENT` during a
//! poll — and everything the engine *reads* comes from `CURRENT`. That is what
//! makes the pointer `GetEventData` hands out safe to keep for a frame: a
//! caller posting its own event between frames appends to `QUEUED` and cannot
//! disturb the buffer the engine is still reading.

use crate::codes::{
    analog_code_to_string, button_code_to_string, is_joystick_button_code, joystick_axis,
    ANALOG_CODE_LAST, BUTTON_CODE_INVALID, BUTTON_CODE_LAST, JOY_AXIS_X, JOY_AXIS_Y,
    KEY_XBUTTON_LTRIGGER, KEY_XBUTTON_RTRIGGER, MOUSE_FIRST, MOUSE_WHEEL, MOUSE_WHEEL_DOWN,
    MOUSE_WHEEL_UP, MOUSE_X, MOUSE_XY, MOUSE_Y,
};

/// `TOUCH_FINGER_MAX_COUNT`.
pub const TOUCH_FINGER_MAX_COUNT: usize = 10;
/// `XUSER_MAX_COUNT` from `posix_stubs.h`.
pub const XUSER_MAX_COUNT: usize = 2;
/// `XK_MAX_KEYS` from `posix_stubs.h`.
pub const XK_MAX_KEYS: usize = 5;
/// `MOUSE_BUTTON_COUNT`.
pub const MOUSE_BUTTON_COUNT: i32 = 5;

/// `INPUT_STATE_QUEUED`: written between polls, read by nothing.
pub const QUEUED: usize = 0;
/// `INPUT_STATE_CURRENT`: what the engine reads, rewritten by each poll.
pub const CURRENT: usize = 1;

/// `IE_ButtonPressed`.
pub const IE_BUTTON_PRESSED: i32 = 0;
/// `IE_ButtonReleased`.
pub const IE_BUTTON_RELEASED: i32 = 1;
/// `IE_ButtonDoubleClicked`.
pub const IE_BUTTON_DOUBLE_CLICKED: i32 = 2;
/// `IE_AnalogValueChanged`.
pub const IE_ANALOG_VALUE_CHANGED: i32 = 3;
/// `IE_FingerDown`.
pub const IE_FINGER_DOWN: i32 = 4;
/// `IE_FingerUp`.
pub const IE_FINGER_UP: i32 = 5;
/// `IE_FingerMotion`.
pub const IE_FINGER_MOTION: i32 = 6;
/// `IE_Quit`, the first system event.
pub const IE_QUIT: i32 = 100;
/// `IE_FirstVguiEvent`. vgui's own numbering starts here; the input system
/// posts three of them by offset, as the C++ does, because their names live
/// in the vgui headers rather than these.
pub const IE_FIRST_VGUI_EVENT: i32 = 1000;
/// `IE_LocateMouseClick`.
pub const IE_LOCATE_MOUSE_CLICK: i32 = IE_FIRST_VGUI_EVENT + 1;
/// `IE_KeyTyped`.
pub const IE_KEY_TYPED: i32 = IE_FIRST_VGUI_EVENT + 3;
/// `IE_KeyCodeTyped`.
pub const IE_KEY_CODE_TYPED: i32 = IE_FIRST_VGUI_EVENT + 4;
/// `IE_FirstAppEvent`.
pub const IE_FIRST_APP_EVENT: i32 = 2000;
/// `IE_AppActivated`, defined in the engine's `sys_mainwind.cpp`.
pub const IE_APP_ACTIVATED: i32 = IE_FIRST_APP_EVENT + 2;

/// `InputEvent_t`: five ints, which is all that crosses the boundary.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputEvent {
    /// `m_nType`, an `InputEventType_t`.
    pub kind: i32,
    /// `m_nTick`.
    pub tick: i32,
    /// `m_nData`, whose meaning depends on `kind`.
    pub data: i32,
    /// `m_nData2`.
    pub data2: i32,
    /// `m_nData3`.
    pub data3: i32,
}

/// `CInputSystem::appKey_t`: how long a synthesized key has been held.
#[derive(Debug, Clone, Copy, Default)]
pub struct AppKey {
    /// `repeats`.
    pub repeats: i32,
    /// `sample`.
    pub sample: i32,
}

/// `CInputSystem::InputState_t`.
#[derive(Debug)]
pub struct Slot {
    down: [bool; BUTTON_CODE_LAST as usize],
    pressed_tick: [i32; BUTTON_CODE_LAST as usize],
    released_tick: [i32; BUTTON_CODE_LAST as usize],
    analog_value: [i32; ANALOG_CODE_LAST as usize],
    analog_delta: [i32; ANALOG_CODE_LAST as usize],
    events: Vec<InputEvent>,
    dirty: bool,
}

impl Slot {
    const fn new() -> Self {
        Self {
            down: [false; BUTTON_CODE_LAST as usize],
            pressed_tick: [0; BUTTON_CODE_LAST as usize],
            released_tick: [0; BUTTON_CODE_LAST as usize],
            analog_value: [0; ANALOG_CODE_LAST as usize],
            analog_delta: [0; ANALOG_CODE_LAST as usize],
            events: Vec::new(),
            dirty: false,
        }
    }

    /// The events posted into this slot, in the order they arrived.
    #[must_use]
    pub fn events(&self) -> &[InputEvent] {
        &self.events
    }
}

impl Default for Slot {
    fn default() -> Self {
        Self::new()
    }
}

/// `CInputSystem`'s state, without the parts that talk to SDL or the launcher.
#[derive(Debug)]
pub struct InputCore {
    states: [Slot; 2],
    polling: bool,
    /// `m_bEnabled`.
    pub enabled: bool,
    /// `m_bPumpEnabled`.
    pub pump_enabled: bool,
    /// `m_StartupTimeTick`.
    pub startup_tick: u32,
    last_poll_tick: i32,
    last_sample_tick: i32,
    poll_count: i32,
    /// `m_nJoystickCount`.
    pub joystick_count: i32,
    /// `m_JoysticksEnabled`, a flag per joystick.
    pub joysticks_enabled: u16,
    /// `m_bXController`: a gamepad is attached, which renames the joystick
    /// range in [`button_code_to_string`].
    pub x_controller: bool,
    /// `m_PrimaryUserId`.
    pub primary_user_id: i32,
    /// `m_hAttachedHWnd`, which off Windows is only ever tested for null.
    pub window_attached: bool,
    /// `m_bConsoleTextMode`.
    pub console_text_mode: bool,
    /// `m_bSkipControllerInitialization`.
    pub skip_controller_initialization: bool,
    /// `m_bJoystickInitialized`.
    pub joystick_initialized: bool,
    /// `m_bTouchInitialized`.
    pub touch_initialized: bool,
    /// `m_bRawInputSupported`, which `USE_SDL` builds set at init.
    pub raw_input_supported: bool,
    touch_accum: [(f32, f32); TOUCH_FINGER_MAX_COUNT],
    app_x_keys: [[AppKey; XK_MAX_KEYS]; XUSER_MAX_COUNT],
    raw_mouse_accum: (i32, i32),
}

/// `INVALID_USER_ID`.
pub const INVALID_USER_ID: i32 = -1;

impl InputCore {
    /// A freshly constructed `CInputSystem`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            states: [Slot::new(), Slot::new()],
            polling: false,
            enabled: true,
            pump_enabled: true,
            startup_tick: 0,
            last_poll_tick: 0,
            last_sample_tick: 0,
            poll_count: 0,
            joystick_count: 0,
            joysticks_enabled: 0,
            x_controller: false,
            primary_user_id: INVALID_USER_ID,
            window_attached: false,
            console_text_mode: false,
            skip_controller_initialization: false,
            joystick_initialized: false,
            touch_initialized: false,
            raw_input_supported: false,
            touch_accum: [(0.0, 0.0); TOUCH_FINGER_MAX_COUNT],
            app_x_keys: [[AppKey {
                repeats: 0,
                sample: 0,
            }; XK_MAX_KEYS]; XUSER_MAX_COUNT],
            raw_mouse_accum: (0, 0),
        }
    }

    /// The slot writes go to: `QUEUED` between polls, `CURRENT` during one.
    fn writing(&mut self) -> &mut Slot {
        &mut self.states[usize::from(self.polling)]
    }

    /// The slot the engine reads.
    #[must_use]
    pub fn current(&self) -> &Slot {
        &self.states[CURRENT]
    }

    /// `m_nLastSampleTick`.
    #[must_use]
    pub const fn last_sample_tick(&self) -> i32 {
        self.last_sample_tick
    }

    /// `GetPollTick`.
    #[must_use]
    pub const fn poll_tick(&self) -> i32 {
        self.last_poll_tick
    }

    /// `GetPollCount`.
    #[must_use]
    pub const fn poll_count(&self) -> i32 {
        self.poll_count
    }

    /// `ComputeSampleTick`: milliseconds since startup, folded the way the
    /// C++ folds them so a wrap of the platform's 32-bit millisecond counter
    /// still produces a rising tick.
    #[must_use]
    pub const fn sample_tick_from(&self, now: u32) -> i32 {
        if now >= self.startup_tick {
            (now - self.startup_tick) as i32
        } else {
            let delta = u32::MAX - self.startup_tick;
            now.wrapping_add(delta).wrapping_add(1) as i32
        }
    }

    /// `SampleDevices`, less the device polling the cdylib does.
    pub const fn begin_sample(&mut self, now: u32) {
        self.last_sample_tick = self.sample_tick_from(now);
    }

    /// The opening of `PollInputState`: take the queued state as current,
    /// events and all, and start directing writes at it.
    pub fn begin_poll(&mut self) {
        self.polling = true;
        self.poll_count = self.poll_count.wrapping_add(1);
        self.copy_state(CURRENT, QUEUED, true);
    }

    /// `PollInputState`'s tick update, which happens after sampling because
    /// sampling is what moves `m_nLastSampleTick`.
    pub const fn adopt_sample_tick(&mut self) {
        self.last_poll_tick = self.last_sample_tick;
    }

    /// The close of `PollInputState`: leave the queued state agreeing with
    /// the current one, but without its events — they have been delivered.
    pub fn end_poll(&mut self) {
        self.copy_state(QUEUED, CURRENT, false);
        self.polling = false;
    }

    /// `CopyInputState`. A source that has not changed since the last copy
    /// leaves the destination's button and analog state alone; its events are
    /// dropped either way.
    fn copy_state(&mut self, dst: usize, src: usize, copy_events: bool) {
        debug_assert_ne!(dst, src);
        let (first, second) = self.states.split_at_mut(1);
        let (dst_slot, src_slot) = if dst == QUEUED {
            (&mut first[0], &second[0])
        } else {
            (&mut second[0], &first[0])
        };

        dst_slot.events.clear();
        dst_slot.dirty = false;
        if !src_slot.dirty {
            return;
        }

        dst_slot.down = src_slot.down;
        dst_slot.pressed_tick = src_slot.pressed_tick;
        dst_slot.released_tick = src_slot.released_tick;
        dst_slot.analog_value = src_slot.analog_value;
        dst_slot.analog_delta = src_slot.analog_delta;
        if copy_events {
            dst_slot.events.extend_from_slice(&src_slot.events);
        }
    }

    /// `PostUserEvent`.
    pub fn post_user_event(&mut self, event: InputEvent) {
        let slot = self.writing();
        slot.events.push(event);
        slot.dirty = true;
    }

    /// `PostEvent`.
    pub fn post_event(&mut self, kind: i32, tick: i32, data: i32, data2: i32, data3: i32) {
        self.post_user_event(InputEvent {
            kind,
            tick,
            data,
            data2,
            data3,
        });
    }

    /// `PostButtonPressedEvent`. A button already down produces nothing,
    /// which is what suppresses key repeats.
    pub fn post_button_pressed(&mut self, kind: i32, tick: i32, scan: i32, virtual_code: i32) {
        let Ok(index) = usize::try_from(scan) else {
            return;
        };
        let slot = self.writing();
        if index >= slot.down.len() || slot.down[index] {
            return;
        }
        slot.down[index] = true;
        slot.pressed_tick[index] = tick;
        self.post_event(kind, tick, scan, virtual_code, 0);
    }

    /// `PostButtonReleasedEvent`. A button already up produces nothing, which
    /// is what keeps an alt-tab from double-releasing.
    pub fn post_button_released(&mut self, kind: i32, tick: i32, scan: i32, virtual_code: i32) {
        let Ok(index) = usize::try_from(scan) else {
            return;
        };
        let slot = self.writing();
        if index >= slot.down.len() || !slot.down[index] {
            return;
        }
        slot.down[index] = false;
        slot.released_tick[index] = tick;
        self.post_event(kind, tick, scan, virtual_code, 0);
    }

    /// `IsButtonDown`.
    #[must_use]
    pub fn is_button_down(&self, code: i32) -> bool {
        usize::try_from(code)
            .ok()
            .and_then(|code| self.current().down.get(code))
            .copied()
            .unwrap_or(false)
    }

    /// `GetButtonPressedTick`.
    #[must_use]
    pub fn button_pressed_tick(&self, code: i32) -> i32 {
        usize::try_from(code)
            .ok()
            .and_then(|code| self.current().pressed_tick.get(code))
            .copied()
            .unwrap_or(0)
    }

    /// `GetButtonReleasedTick`.
    #[must_use]
    pub fn button_released_tick(&self, code: i32) -> i32 {
        usize::try_from(code)
            .ok()
            .and_then(|code| self.current().released_tick.get(code))
            .copied()
            .unwrap_or(0)
    }

    /// `GetAnalogValue`.
    #[must_use]
    pub fn analog_value(&self, code: i32) -> i32 {
        usize::try_from(code)
            .ok()
            .and_then(|code| self.current().analog_value.get(code))
            .copied()
            .unwrap_or(0)
    }

    /// `GetAnalogDelta`.
    #[must_use]
    pub fn analog_delta(&self, code: i32) -> i32 {
        usize::try_from(code)
            .ok()
            .and_then(|code| self.current().analog_delta.get(code))
            .copied()
            .unwrap_or(0)
    }

    /// `ReleaseAllButtons`, which posts a release for every button still down.
    pub fn release_all_buttons(&mut self) {
        let tick = self.last_sample_tick;
        for code in 0..BUTTON_CODE_LAST {
            self.post_button_released(IE_BUTTON_RELEASED, tick, code, code);
        }
    }

    /// `ZeroAnalogState` over the whole range.
    pub fn zero_analog_state(&mut self) {
        let slot = self.writing();
        slot.analog_value = [0; ANALOG_CODE_LAST as usize];
        slot.analog_delta = [0; ANALOG_CODE_LAST as usize];
    }

    /// `ResetInputState`: tell the engine every button came up, then forget
    /// the analog and raw-mouse state.
    pub fn reset_input_state(&mut self) {
        self.release_all_buttons();
        self.zero_analog_state();
        self.app_x_keys = [[AppKey::default(); XK_MAX_KEYS]; XUSER_MAX_COUNT];
        self.raw_mouse_accum = (0, 0);
    }

    /// `ClearInputState`: drop everything without telling anyone, which is
    /// what a new window wants.
    pub fn clear_input_state(&mut self) {
        for slot in &mut self.states {
            *slot = Slot::new();
        }
        self.app_x_keys = [[AppKey::default(); XK_MAX_KEYS]; XUSER_MAX_COUNT];
    }

    /// `UpdateMouseButtonState`: the mask holds one bit per mouse button, and
    /// every button not in it is released.
    pub fn update_mouse_button_state(&mut self, mask: i32, double_click_code: i32) {
        let tick = self.last_sample_tick;
        for button in 0..MOUSE_BUTTON_COUNT {
            let code = MOUSE_FIRST + button;
            if mask & (1 << button) == 0 {
                self.post_button_released(IE_BUTTON_RELEASED, tick, code, code);
                continue;
            }
            let kind = if code == double_click_code {
                IE_BUTTON_DOUBLE_CLICKED
            } else {
                IE_BUTTON_PRESSED
            };
            self.post_button_pressed(kind, tick, code, code);
        }
    }

    /// `UpdateMousePositionState`: the cursor moved to `x`,`y`, so the delta
    /// is against wherever it was.
    pub fn update_mouse_position_state(&mut self, x: i32, y: i32) {
        let tick = self.last_sample_tick;
        let slot = self.writing();
        let old_x = slot.analog_value[MOUSE_X as usize];
        let old_y = slot.analog_value[MOUSE_Y as usize];
        slot.analog_value[MOUSE_X as usize] = x;
        slot.analog_value[MOUSE_Y as usize] = y;
        slot.analog_delta[MOUSE_X as usize] = x - old_x;
        slot.analog_delta[MOUSE_Y as usize] = y - old_y;

        if x != old_x {
            self.post_event(IE_ANALOG_VALUE_CHANGED, tick, MOUSE_X, x, x - old_x);
        }
        if y != old_y {
            self.post_event(IE_ANALOG_VALUE_CHANGED, tick, MOUSE_Y, y, y - old_y);
        }
        if x != old_x || y != old_y {
            self.post_event(IE_ANALOG_VALUE_CHANGED, tick, MOUSE_XY, x, y);
        }
    }

    /// The state half of `SetCursorPosition`: the cursor was *put* somewhere,
    /// so the delta is zero however far it moved.
    pub fn set_cursor_position_state(&mut self, x: i32, y: i32) {
        let tick = self.last_sample_tick;
        let slot = self.writing();
        let changed_x = slot.analog_value[MOUSE_X as usize] != x;
        let changed_y = slot.analog_value[MOUSE_Y as usize] != y;
        slot.analog_value[MOUSE_X as usize] = x;
        slot.analog_value[MOUSE_Y as usize] = y;
        slot.analog_delta[MOUSE_X as usize] = 0;
        slot.analog_delta[MOUSE_Y as usize] = 0;

        if changed_x {
            self.post_event(IE_ANALOG_VALUE_CHANGED, tick, MOUSE_X, x, 0);
        }
        if changed_y {
            self.post_event(IE_ANALOG_VALUE_CHANGED, tick, MOUSE_Y, y, 0);
        }
        if changed_x || changed_y {
            self.post_event(IE_ANALOG_VALUE_CHANGED, tick, MOUSE_XY, x, y);
        }
    }

    /// The wheel, which is a pair of pseudo-buttons and an analog axis at once.
    pub fn mouse_wheel(&mut self, clicks: i32) {
        let tick = self.last_sample_tick;
        let code = if clicks > 0 {
            MOUSE_WHEEL_UP
        } else {
            MOUSE_WHEEL_DOWN
        };

        let slot = self.writing();
        // The wheel's press and release are recorded directly rather than
        // through post_button_*, because it is never held: both ticks move and
        // the down state never changes.
        slot.pressed_tick[code as usize] = tick;
        slot.released_tick[code as usize] = tick;
        slot.analog_delta[MOUSE_WHEEL as usize] = clicks;
        slot.analog_value[MOUSE_WHEEL as usize] += clicks;
        let value = slot.analog_value[MOUSE_WHEEL as usize];

        self.post_event(IE_BUTTON_PRESSED, tick, code, code, 0);
        self.post_event(IE_BUTTON_RELEASED, tick, code, code, 0);
        self.post_event(IE_ANALOG_VALUE_CHANGED, tick, MOUSE_WHEEL, value, clicks);
    }

    /// `JoystickAxisMotion`'s state half: record an axis and, for the two
    /// triggers, the button they double as. `press_threshold` and
    /// `dead_zone` come from the `joy_axisbutton_threshold` and
    /// `joy_axis_deadzone` convars.
    pub fn joystick_axis_motion(
        &mut self,
        code: i32,
        mut value: i32,
        button_code: i32,
        press_threshold: i32,
        dead_zone: i32,
    ) {
        let tick = self.last_sample_tick;

        if button_code != crate::codes::BUTTON_CODE_NONE {
            let key_index = (button_code - KEY_XBUTTON_LTRIGGER) as usize;
            if let Some(key) = self.app_x_keys[0].get_mut(key_index) {
                if value > press_threshold {
                    let first = key.repeats < 1;
                    key.repeats += 1;
                    if first {
                        self.post_button_pressed(IE_BUTTON_PRESSED, tick, button_code, button_code);
                    }
                } else {
                    key.repeats = 0;
                    self.post_button_released(IE_BUTTON_RELEASED, tick, button_code, button_code);
                }
            }
        }

        if value.abs() < dead_zone {
            value = 0;
        }

        let Ok(index) = usize::try_from(code) else {
            return;
        };
        let slot = self.writing();
        if index >= slot.analog_value.len() {
            return;
        }
        let delta = value - slot.analog_value[index];
        slot.analog_delta[index] = delta;
        slot.analog_value[index] = value;
        if delta != 0 {
            self.post_event(IE_ANALOG_VALUE_CHANGED, tick, code, value, 0);
        }
    }

    /// `FingerEvent`. Fingers past the tenth are dropped, as in the C++.
    /// `x` and `y` are normalized floats reinterpreted as ints, because the
    /// event carries ints and the engine reinterprets them back.
    pub fn finger_event(&mut self, kind: i32, finger: i32, x: f32, y: f32, dx: f32, dy: f32) {
        let Ok(index) = usize::try_from(finger) else {
            return;
        };
        let Some(accum) = self.touch_accum.get_mut(index) else {
            return;
        };
        if kind == IE_FINGER_UP {
            *accum = (0.0, 0.0);
        } else {
            accum.0 += dx;
            accum.1 += dy;
        }
        let tick = self.last_sample_tick;
        self.post_event(
            kind,
            tick,
            finger,
            x.to_bits() as i32,
            y.to_bits() as i32,
        );
    }

    /// `GetTouchAccumulators`: read and clear one finger's travel.
    pub fn take_touch_accumulator(&mut self, finger: i32) -> (f32, f32) {
        let Ok(index) = usize::try_from(finger) else {
            return (0.0, 0.0);
        };
        let Some(accum) = self.touch_accum.get_mut(index) else {
            return (0.0, 0.0);
        };
        std::mem::replace(accum, (0.0, 0.0))
    }

    /// `EnableJoystickInput`.
    pub const fn enable_joystick_input(&mut self, joystick: i32, enable: bool) {
        if joystick < 0 || joystick >= 16 {
            return;
        }
        let flag = 1u16 << joystick;
        if enable {
            self.joysticks_enabled |= flag;
        } else {
            self.joysticks_enabled &= !flag;
        }
    }

    /// `SetPrimaryUserId`, which rejects anything outside the user range.
    pub const fn set_primary_user_id(&mut self, user_id: i32) {
        self.primary_user_id = if user_id < 0 || user_id >= XUSER_MAX_COUNT as i32 {
            INVALID_USER_ID
        } else {
            user_id
        };
    }

    /// `m_mouseRawAccum*`, which only the non-SDL path fills; kept so the
    /// reset path has the same effect it has in the C++.
    pub const fn take_raw_mouse_accumulators(&mut self) -> (i32, i32) {
        let taken = self.raw_mouse_accum;
        self.raw_mouse_accum = (0, 0);
        taken
    }

    /// `ButtonCodeToString` with this module's gamepad flag applied.
    #[must_use]
    pub fn button_code_to_string(&self, code: i32) -> &'static std::ffi::CStr {
        button_code_to_string(code, self.x_controller)
    }

    /// `AnalogCodeToString`.
    #[must_use]
    pub fn analog_code_to_string(&self, code: i32) -> &'static std::ffi::CStr {
        analog_code_to_string(code)
    }

    /// `StringToButtonCode` with this module's gamepad flag applied.
    #[must_use]
    pub fn string_to_button_code(&self, name: &std::ffi::CStr) -> i32 {
        crate::codes::string_to_button_code(name, self.x_controller)
    }
}

impl Default for InputCore {
    fn default() -> Self {
        Self::new()
    }
}

/// The axis a gamepad stick reports, mapped to the analog code and, for the
/// triggers, the button they also press. Mirrors
/// `ControllerAxisToAnalogCode` and the switch beside it.
#[must_use]
pub const fn controller_axis(axis: GameControllerAxis) -> Option<(i32, i32)> {
    let none = crate::codes::BUTTON_CODE_NONE;
    match axis {
        GameControllerAxis::LeftX => Some((joystick_axis(0, JOY_AXIS_X), none)),
        GameControllerAxis::LeftY => Some((joystick_axis(0, JOY_AXIS_Y), none)),
        GameControllerAxis::RightX => Some((joystick_axis(0, crate::codes::JOY_AXIS_U), none)),
        GameControllerAxis::RightY => Some((joystick_axis(0, crate::codes::JOY_AXIS_R), none)),
        GameControllerAxis::TriggerLeft => Some((
            joystick_axis(0, crate::codes::JOY_AXIS_Z),
            KEY_XBUTTON_LTRIGGER,
        )),
        GameControllerAxis::TriggerRight => Some((
            joystick_axis(0, crate::codes::JOY_AXIS_Z),
            KEY_XBUTTON_RTRIGGER,
        )),
    }
}

/// `SDL_GameControllerAxis`, named rather than numbered so the mapping above
/// reads as the C++ switch does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameControllerAxis {
    /// `SDL_CONTROLLER_AXIS_LEFTX`.
    LeftX,
    /// `SDL_CONTROLLER_AXIS_LEFTY`.
    LeftY,
    /// `SDL_CONTROLLER_AXIS_RIGHTX`.
    RightX,
    /// `SDL_CONTROLLER_AXIS_RIGHTY`.
    RightY,
    /// `SDL_CONTROLLER_AXIS_TRIGGERLEFT`.
    TriggerLeft,
    /// `SDL_CONTROLLER_AXIS_TRIGGERRIGHT`.
    TriggerRight,
}

impl GameControllerAxis {
    /// The axis SDL numbers `axis`, or nothing for one this module ignores.
    #[must_use]
    pub const fn from_sdl(axis: i32) -> Option<Self> {
        match axis {
            0 => Some(Self::LeftX),
            1 => Some(Self::LeftY),
            2 => Some(Self::RightX),
            3 => Some(Self::RightY),
            4 => Some(Self::TriggerLeft),
            5 => Some(Self::TriggerRight),
            _ => None,
        }
    }
}

/// `ControllerButtonToButtonCode`: `SDL_GameControllerButton` to a code.
/// Returns `BUTTON_CODE_NONE` for buttons the engine has no name for.
///
/// The guide button answers `KEY_XBUTTON_BACK`, as the C++ does — its own
/// comment asks what else it should do.
#[must_use]
pub const fn controller_button_to_button_code(button: i32) -> i32 {
    use crate::codes::{
        joystick_button, BUTTON_CODE_NONE, KEY_XBUTTON_BACK, KEY_XBUTTON_DOWN, KEY_XBUTTON_LEFT,
        KEY_XBUTTON_LEFT_SHOULDER, KEY_XBUTTON_RIGHT, KEY_XBUTTON_RIGHT_SHOULDER,
        KEY_XBUTTON_START, KEY_XBUTTON_STICK1, KEY_XBUTTON_STICK2, KEY_XBUTTON_UP,
    };
    match button {
        // A, B, X, Y are joystick buttons 0..3 by their SDL numbering.
        0..=3 => joystick_button(0, button),
        4 => KEY_XBUTTON_BACK,       // SDL_CONTROLLER_BUTTON_BACK
        5 => KEY_XBUTTON_BACK,       // SDL_CONTROLLER_BUTTON_GUIDE
        6 => KEY_XBUTTON_START,      // SDL_CONTROLLER_BUTTON_START
        7 => KEY_XBUTTON_STICK1,     // SDL_CONTROLLER_BUTTON_LEFTSTICK
        8 => KEY_XBUTTON_STICK2,     // SDL_CONTROLLER_BUTTON_RIGHTSTICK
        9 => KEY_XBUTTON_LEFT_SHOULDER,
        10 => KEY_XBUTTON_RIGHT_SHOULDER,
        11 => KEY_XBUTTON_UP,        // SDL_CONTROLLER_BUTTON_DPAD_UP
        12 => KEY_XBUTTON_DOWN,
        13 => KEY_XBUTTON_LEFT,
        14 => KEY_XBUTTON_RIGHT,
        _ => BUTTON_CODE_NONE,
    }
}

/// Whether a code names a joystick button, for the callers that ask.
#[must_use]
pub const fn is_joystick_button(code: i32) -> bool {
    is_joystick_button_code(code)
}

/// A code that is not a real button, for callers that need one.
#[must_use]
pub const fn invalid_button_code() -> i32 {
    BUTTON_CODE_INVALID
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codes::{key, KEY_A, MOUSE_LEFT, MOUSE_RIGHT};

    fn polled(core: &mut InputCore, body: impl FnOnce(&mut InputCore)) {
        core.begin_poll();
        core.begin_sample(0);
        body(core);
        core.adopt_sample_tick();
        core.end_poll();
    }

    #[test]
    fn a_press_is_reported_once_and_stays_down_until_released() {
        let mut core = InputCore::new();
        polled(&mut core, |core| {
            core.post_button_pressed(IE_BUTTON_PRESSED, 5, KEY_A, KEY_A);
            core.post_button_pressed(IE_BUTTON_PRESSED, 6, KEY_A, KEY_A);
        });

        assert!(core.is_button_down(KEY_A));
        assert_eq!(core.button_pressed_tick(KEY_A), 5, "the repeat is suppressed");
        assert_eq!(core.current().events().len(), 1);
        assert_eq!(core.current().events()[0].kind, IE_BUTTON_PRESSED);

        polled(&mut core, |core| {
            core.post_button_released(IE_BUTTON_RELEASED, 9, KEY_A, KEY_A);
            core.post_button_released(IE_BUTTON_RELEASED, 10, KEY_A, KEY_A);
        });
        assert!(!core.is_button_down(KEY_A));
        assert_eq!(core.button_released_tick(KEY_A), 9);
        assert_eq!(core.current().events().len(), 1);
    }

    #[test]
    fn events_posted_between_polls_arrive_on_the_next_one_and_only_once() {
        let mut core = InputCore::new();

        // A caller posting its own event between frames writes to QUEUED.
        core.post_user_event(InputEvent {
            kind: IE_QUIT,
            ..InputEvent::default()
        });
        assert_eq!(
            core.current().events().len(),
            0,
            "nothing the engine reads has changed yet"
        );

        polled(&mut core, |_| {});
        assert_eq!(core.current().events().len(), 1);
        assert_eq!(core.current().events()[0].kind, IE_QUIT);

        // The second poll must not deliver it again: end_poll copies CURRENT
        // back to QUEUED without its events.
        polled(&mut core, |_| {});
        assert_eq!(core.current().events().len(), 0);
    }

    #[test]
    fn button_state_survives_a_poll_that_posted_nothing() {
        let mut core = InputCore::new();
        polled(&mut core, |core| {
            core.post_button_pressed(IE_BUTTON_PRESSED, 1, key(b'b'), key(b'b'));
        });
        assert!(core.is_button_down(key(b'b')));

        polled(&mut core, |_| {});
        assert!(
            core.is_button_down(key(b'b')),
            "a quiet frame must not drop the key the player is holding"
        );
        assert_eq!(core.current().events().len(), 0);
    }

    #[test]
    fn the_mouse_mask_presses_what_is_set_and_releases_what_is_not() {
        let mut core = InputCore::new();
        polled(&mut core, |core| {
            core.update_mouse_button_state(0b1, BUTTON_CODE_INVALID);
        });
        assert!(core.is_button_down(MOUSE_LEFT));
        assert!(!core.is_button_down(MOUSE_RIGHT));

        polled(&mut core, |core| {
            core.update_mouse_button_state(0b10, BUTTON_CODE_INVALID);
        });
        assert!(!core.is_button_down(MOUSE_LEFT));
        assert!(core.is_button_down(MOUSE_RIGHT));

        polled(&mut core, |core| {
            core.update_mouse_button_state(0b1, MOUSE_LEFT);
        });
        let kinds: Vec<i32> = core.current().events().iter().map(|e| e.kind).collect();
        assert!(
            kinds.contains(&IE_BUTTON_DOUBLE_CLICKED),
            "the double-click code turns the press into a double click"
        );
    }

    #[test]
    fn moving_the_mouse_posts_x_y_and_xy_but_putting_it_leaves_no_delta() {
        let mut core = InputCore::new();
        polled(&mut core, |core| {
            core.update_mouse_position_state(10, 20);
        });
        assert_eq!(core.analog_value(MOUSE_X), 10);
        assert_eq!(core.analog_value(MOUSE_Y), 20);
        assert_eq!(core.analog_delta(MOUSE_X), 10);
        let codes: Vec<i32> = core
            .current()
            .events()
            .iter()
            .filter(|e| e.kind == IE_ANALOG_VALUE_CHANGED)
            .map(|e| e.data)
            .collect();
        assert_eq!(codes, vec![MOUSE_X, MOUSE_Y, MOUSE_XY]);

        polled(&mut core, |core| {
            core.set_cursor_position_state(99, 20);
        });
        assert_eq!(core.analog_value(MOUSE_X), 99);
        assert_eq!(core.analog_delta(MOUSE_X), 0, "a warp is not a movement");
        let codes: Vec<i32> = core
            .current()
            .events()
            .iter()
            .filter(|e| e.kind == IE_ANALOG_VALUE_CHANGED)
            .map(|e| e.data)
            .collect();
        assert_eq!(codes, vec![MOUSE_X, MOUSE_XY], "y did not change");
    }

    #[test]
    fn the_wheel_accumulates_and_presses_a_direction() {
        let mut core = InputCore::new();
        polled(&mut core, |core| {
            core.mouse_wheel(2);
            core.mouse_wheel(-1);
        });
        assert_eq!(core.analog_value(MOUSE_WHEEL), 1);
        assert_eq!(core.analog_delta(MOUSE_WHEEL), -1);
        assert_eq!(core.button_pressed_tick(MOUSE_WHEEL_UP), 0);
        assert!(
            !core.is_button_down(MOUSE_WHEEL_UP),
            "the wheel is never held down"
        );
    }

    #[test]
    fn resetting_releases_every_held_button_so_the_engine_hears_about_it() {
        let mut core = InputCore::new();
        polled(&mut core, |core| {
            core.post_button_pressed(IE_BUTTON_PRESSED, 1, KEY_A, KEY_A);
            core.post_button_pressed(IE_BUTTON_PRESSED, 1, MOUSE_LEFT, MOUSE_LEFT);
        });

        polled(&mut core, InputCore::reset_input_state);
        assert!(!core.is_button_down(KEY_A));
        assert!(!core.is_button_down(MOUSE_LEFT));
        let released: Vec<i32> = core
            .current()
            .events()
            .iter()
            .filter(|e| e.kind == IE_BUTTON_RELEASED)
            .map(|e| e.data)
            .collect();
        assert_eq!(released, vec![KEY_A, MOUSE_LEFT]);
    }

    #[test]
    fn clearing_drops_state_without_telling_anyone() {
        let mut core = InputCore::new();
        polled(&mut core, |core| {
            core.post_button_pressed(IE_BUTTON_PRESSED, 1, KEY_A, KEY_A);
        });
        core.clear_input_state();
        assert!(!core.is_button_down(KEY_A));
        assert_eq!(core.current().events().len(), 0);
    }

    #[test]
    fn the_sample_tick_keeps_rising_across_the_millisecond_counters_wrap() {
        let mut core = InputCore::new();
        core.startup_tick = u32::MAX - 100;
        assert_eq!(core.sample_tick_from(u32::MAX - 100), 0);
        assert_eq!(core.sample_tick_from(u32::MAX), 100);
        assert_eq!(core.sample_tick_from(0), 101, "the counter wrapped");
        assert_eq!(core.sample_tick_from(5), 106);
    }

    #[test]
    fn a_trigger_axis_presses_its_button_once_and_deadzones_the_value() {
        let mut core = InputCore::new();
        let code = joystick_axis(0, crate::codes::JOY_AXIS_Z);
        polled(&mut core, |core| {
            core.joystick_axis_motion(code, 30_000, KEY_XBUTTON_RTRIGGER, 9_830, 6_553);
            core.joystick_axis_motion(code, 30_000, KEY_XBUTTON_RTRIGGER, 9_830, 6_553);
        });
        assert!(core.is_button_down(KEY_XBUTTON_RTRIGGER));
        assert_eq!(core.analog_value(code), 30_000);
        assert_eq!(
            core.current()
                .events()
                .iter()
                .filter(|e| e.kind == IE_BUTTON_PRESSED)
                .count(),
            1,
            "holding the trigger is one press, not one per sample"
        );

        polled(&mut core, |core| {
            core.joystick_axis_motion(code, 500, KEY_XBUTTON_RTRIGGER, 9_830, 6_553);
        });
        assert!(!core.is_button_down(KEY_XBUTTON_RTRIGGER));
        assert_eq!(core.analog_value(code), 0, "inside the dead zone");
    }

    #[test]
    fn touch_travel_accumulates_until_it_is_read() {
        let mut core = InputCore::new();
        polled(&mut core, |core| {
            core.finger_event(IE_FINGER_MOTION, 0, 0.5, 0.5, 0.25, -0.125);
            core.finger_event(IE_FINGER_MOTION, 0, 0.75, 0.375, 0.25, -0.125);
        });
        assert_eq!(core.take_touch_accumulator(0), (0.5, -0.25));
        assert_eq!(core.take_touch_accumulator(0), (0.0, 0.0));
        // Out of range fingers are dropped rather than panicking.
        assert_eq!(core.take_touch_accumulator(999), (0.0, 0.0));
        core.finger_event(IE_FINGER_MOTION, 999, 0.0, 0.0, 1.0, 1.0);
    }

    #[test]
    fn a_finger_lifting_zeroes_its_travel() {
        let mut core = InputCore::new();
        core.finger_event(IE_FINGER_MOTION, 1, 0.0, 0.0, 0.5, 0.5);
        core.finger_event(IE_FINGER_UP, 1, 0.0, 0.0, 0.5, 0.5);
        assert_eq!(core.take_touch_accumulator(1), (0.0, 0.0));
    }

    #[test]
    fn the_primary_user_id_rejects_anything_outside_the_user_range() {
        let mut core = InputCore::new();
        core.set_primary_user_id(1);
        assert_eq!(core.primary_user_id, 1);
        core.set_primary_user_id(XUSER_MAX_COUNT as i32);
        assert_eq!(core.primary_user_id, INVALID_USER_ID);
        core.set_primary_user_id(-5);
        assert_eq!(core.primary_user_id, INVALID_USER_ID);
    }

    #[test]
    fn out_of_range_codes_answer_rather_than_panic() {
        let mut core = InputCore::new();
        assert!(!core.is_button_down(-1));
        assert!(!core.is_button_down(BUTTON_CODE_LAST));
        assert_eq!(core.button_pressed_tick(BUTTON_CODE_LAST), 0);
        assert_eq!(core.analog_value(ANALOG_CODE_LAST), 0);
        assert_eq!(core.analog_delta(-1), 0);
        core.post_button_pressed(IE_BUTTON_PRESSED, 0, BUTTON_CODE_LAST, 0);
        core.post_button_released(IE_BUTTON_RELEASED, 0, -1, 0);
    }
}
