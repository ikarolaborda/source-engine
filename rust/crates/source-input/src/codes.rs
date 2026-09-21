//! `ButtonCode_t`, `AnalogCode_t` and the tables `key_translation.cpp` builds.
//!
//! The enums are laid out by arithmetic on `MAX_JOYSTICKS`, `SK_MAX_KEYS` and
//! friends rather than written out, so the constants here are that arithmetic
//! rather than transcribed numbers, and [`NAMES`] is asserted against
//! `BUTTON_CODE_LAST` the way the C++ asserts its own table.
//!
//! Everything here is a table lookup on a code the caller supplies, so every
//! entry point bounds-checks. The C++ does not in a few places — the callers
//! only ever pass codes it produced — and where a caller could reach outside,
//! the divergence is noted on the function.

use std::ffi::CStr;

// The shapes the enums are built from, all from `InputEnums.h` except
// `SK_MAX_KEYS`, which is the count of `sKey_t`.
/// `MAX_JOYSTICKS`.
pub const MAX_JOYSTICKS: i32 = 1;
/// `MAX_JOYSTICK_AXES`, in the non-Linux order.
pub const MAX_JOYSTICK_AXES: i32 = 6;
/// `MAX_STEAM_CONTROLLERS`.
pub const MAX_STEAM_CONTROLLERS: i32 = 8;
/// `SK_MAX_KEYS`: `sKey_t` runs `SK_NULL` through `SK_VBUTTON_F12`.
pub const SK_MAX_KEYS: i32 = 43;
/// `MAX_STEAMPADAXIS`, which aliases `GYRO_AXIS_YAW` rather than being a count.
pub const MAX_STEAMPADAXIS: i32 = 8;

/// `JOYSTICK_MAX_BUTTON_COUNT`.
pub const JOYSTICK_MAX_BUTTON_COUNT: i32 = 32;
/// `JOYSTICK_POV_BUTTON_COUNT`.
pub const JOYSTICK_POV_BUTTON_COUNT: i32 = 4;
/// `JOYSTICK_AXIS_BUTTON_COUNT`.
pub const JOYSTICK_AXIS_BUTTON_COUNT: i32 = MAX_JOYSTICK_AXES * 2;
/// `STEAMCONTROLLER_MAX_BUTTON_COUNT`.
pub const STEAMCONTROLLER_MAX_BUTTON_COUNT: i32 = SK_MAX_KEYS - 1;
/// `STEAMCONTROLLER_AXIS_BUTTON_COUNT`.
pub const STEAMCONTROLLER_AXIS_BUTTON_COUNT: i32 = MAX_STEAMPADAXIS * 2;

/// `BUTTON_CODE_INVALID`.
pub const BUTTON_CODE_INVALID: i32 = -1;
/// `BUTTON_CODE_NONE`, which is also `KEY_FIRST` and `KEY_NONE`.
pub const BUTTON_CODE_NONE: i32 = 0;
/// `KEY_FIRST`.
pub const KEY_FIRST: i32 = 0;
/// `KEY_NONE`.
pub const KEY_NONE: i32 = 0;
/// `KEY_0`.
pub const KEY_0: i32 = 1;
/// `KEY_9`.
pub const KEY_9: i32 = KEY_0 + 9;
/// `KEY_A`.
pub const KEY_A: i32 = 11;
/// `KEY_Z`.
pub const KEY_Z: i32 = KEY_A + 25;
/// `KEY_PAD_0`.
pub const KEY_PAD_0: i32 = 37;
/// `KEY_PAD_1`.
pub const KEY_PAD_1: i32 = 38;
/// `KEY_PAD_2`.
pub const KEY_PAD_2: i32 = 39;
/// `KEY_PAD_3`.
pub const KEY_PAD_3: i32 = 40;
/// `KEY_PAD_4`.
pub const KEY_PAD_4: i32 = 41;
/// `KEY_PAD_5`.
pub const KEY_PAD_5: i32 = 42;
/// `KEY_PAD_6`.
pub const KEY_PAD_6: i32 = 43;
/// `KEY_PAD_7`.
pub const KEY_PAD_7: i32 = 44;
/// `KEY_PAD_8`.
pub const KEY_PAD_8: i32 = 45;
/// `KEY_PAD_9`.
pub const KEY_PAD_9: i32 = 46;
/// `KEY_PAD_DIVIDE`.
pub const KEY_PAD_DIVIDE: i32 = 47;
/// `KEY_PAD_MULTIPLY`.
pub const KEY_PAD_MULTIPLY: i32 = 48;
/// `KEY_PAD_MINUS`.
pub const KEY_PAD_MINUS: i32 = 49;
/// `KEY_PAD_PLUS`.
pub const KEY_PAD_PLUS: i32 = 50;
/// `KEY_PAD_ENTER`.
pub const KEY_PAD_ENTER: i32 = 51;
/// `KEY_PAD_DECIMAL`.
pub const KEY_PAD_DECIMAL: i32 = 52;
/// `KEY_LBRACKET`.
pub const KEY_LBRACKET: i32 = 53;
/// `KEY_RBRACKET`.
pub const KEY_RBRACKET: i32 = 54;
/// `KEY_SEMICOLON`.
pub const KEY_SEMICOLON: i32 = 55;
/// `KEY_APOSTROPHE`.
pub const KEY_APOSTROPHE: i32 = 56;
/// `KEY_BACKQUOTE`.
pub const KEY_BACKQUOTE: i32 = 57;
/// `KEY_COMMA`.
pub const KEY_COMMA: i32 = 58;
/// `KEY_PERIOD`.
pub const KEY_PERIOD: i32 = 59;
/// `KEY_SLASH`.
pub const KEY_SLASH: i32 = 60;
/// `KEY_BACKSLASH`.
pub const KEY_BACKSLASH: i32 = 61;
/// `KEY_MINUS`.
pub const KEY_MINUS: i32 = 62;
/// `KEY_EQUAL`.
pub const KEY_EQUAL: i32 = 63;
/// `KEY_ENTER`.
pub const KEY_ENTER: i32 = 64;
/// `KEY_SPACE`.
pub const KEY_SPACE: i32 = 65;
/// `KEY_BACKSPACE`.
pub const KEY_BACKSPACE: i32 = 66;
/// `KEY_TAB`.
pub const KEY_TAB: i32 = 67;
/// `KEY_CAPSLOCK`.
pub const KEY_CAPSLOCK: i32 = 68;
/// `KEY_NUMLOCK`.
pub const KEY_NUMLOCK: i32 = 69;
/// `KEY_ESCAPE`.
pub const KEY_ESCAPE: i32 = 70;
/// `KEY_SCROLLLOCK`.
pub const KEY_SCROLLLOCK: i32 = 71;
/// `KEY_INSERT`.
pub const KEY_INSERT: i32 = 72;
/// `KEY_DELETE`.
pub const KEY_DELETE: i32 = 73;
/// `KEY_HOME`.
pub const KEY_HOME: i32 = 74;
/// `KEY_END`.
pub const KEY_END: i32 = 75;
/// `KEY_PAGEUP`.
pub const KEY_PAGEUP: i32 = 76;
/// `KEY_PAGEDOWN`.
pub const KEY_PAGEDOWN: i32 = 77;
/// `KEY_BREAK`, which the name table calls `PAUSE`.
pub const KEY_BREAK: i32 = 78;
/// `KEY_LSHIFT`.
pub const KEY_LSHIFT: i32 = 79;
/// `KEY_RSHIFT`.
pub const KEY_RSHIFT: i32 = 80;
/// `KEY_LALT`.
pub const KEY_LALT: i32 = 81;
/// `KEY_RALT`.
pub const KEY_RALT: i32 = 82;
/// `KEY_LCONTROL`.
pub const KEY_LCONTROL: i32 = 83;
/// `KEY_RCONTROL`.
pub const KEY_RCONTROL: i32 = 84;
/// `KEY_LWIN`.
pub const KEY_LWIN: i32 = 85;
/// `KEY_RWIN`.
pub const KEY_RWIN: i32 = 86;
/// `KEY_APP`.
pub const KEY_APP: i32 = 87;
/// `KEY_UP`.
pub const KEY_UP: i32 = 88;
/// `KEY_LEFT`.
pub const KEY_LEFT: i32 = 89;
/// `KEY_DOWN`.
pub const KEY_DOWN: i32 = 90;
/// `KEY_RIGHT`.
pub const KEY_RIGHT: i32 = 91;
/// `KEY_F1`; `KEY_F2` through `KEY_F12` follow it.
pub const KEY_F1: i32 = 92;
/// `KEY_SCROLLLOCKTOGGLE`, the last key code.
pub const KEY_LAST: i32 = 106;

/// The key code for an ASCII letter or digit, which the enum lays out in
/// order: writing the scan-code table as `key(b'q')` keeps it readable and
/// makes a transposed letter a compile error rather than a wrong number.
#[must_use]
pub const fn key(ascii: u8) -> i32 {
    match ascii {
        b'0'..=b'9' => KEY_0 + (ascii - b'0') as i32,
        b'a'..=b'z' => KEY_A + (ascii - b'a') as i32,
        _ => panic!("key() takes an ASCII digit or lowercase letter"),
    }
}

/// The key code for a function key, `1` through `12`.
#[must_use]
pub const fn key_f(number: i32) -> i32 {
    assert!(number >= 1 && number <= 12);
    KEY_F1 + number - 1
}

/// The key code for a keypad digit, `0` through `9`.
#[must_use]
pub const fn key_pad(digit: i32) -> i32 {
    assert!(digit >= 0 && digit <= 9);
    KEY_PAD_0 + digit
}

/// `MOUSE_FIRST`, which is also `MOUSE_LEFT`.
pub const MOUSE_FIRST: i32 = KEY_LAST + 1;
/// `MOUSE_LEFT`.
pub const MOUSE_LEFT: i32 = MOUSE_FIRST;
/// `MOUSE_RIGHT`.
pub const MOUSE_RIGHT: i32 = MOUSE_FIRST + 1;
/// `MOUSE_MIDDLE`.
pub const MOUSE_MIDDLE: i32 = MOUSE_FIRST + 2;
/// `MOUSE_4`.
pub const MOUSE_4: i32 = MOUSE_FIRST + 3;
/// `MOUSE_5`.
pub const MOUSE_5: i32 = MOUSE_FIRST + 4;
/// `MOUSE_WHEEL_UP`.
pub const MOUSE_WHEEL_UP: i32 = MOUSE_FIRST + 5;
/// `MOUSE_WHEEL_DOWN`.
pub const MOUSE_WHEEL_DOWN: i32 = MOUSE_FIRST + 6;
/// `MOUSE_LAST`.
pub const MOUSE_LAST: i32 = MOUSE_WHEEL_DOWN;
/// `MOUSE_COUNT`, the five real buttons plus the two wheel pseudo-buttons.
pub const MOUSE_COUNT: i32 = MOUSE_LAST - MOUSE_FIRST + 1;

/// `JOYSTICK_FIRST`, which is also `JOYSTICK_FIRST_BUTTON`.
pub const JOYSTICK_FIRST: i32 = MOUSE_LAST + 1;
/// `JOYSTICK_FIRST_BUTTON`.
pub const JOYSTICK_FIRST_BUTTON: i32 = JOYSTICK_FIRST;
/// `JOYSTICK_LAST_BUTTON`.
pub const JOYSTICK_LAST_BUTTON: i32 =
    JOYSTICK_FIRST_BUTTON + (MAX_JOYSTICKS - 1) * JOYSTICK_MAX_BUTTON_COUNT
        + JOYSTICK_MAX_BUTTON_COUNT
        - 1;
/// `JOYSTICK_FIRST_POV_BUTTON`.
pub const JOYSTICK_FIRST_POV_BUTTON: i32 = JOYSTICK_LAST_BUTTON + 1;
/// `JOYSTICK_LAST_POV_BUTTON`.
pub const JOYSTICK_LAST_POV_BUTTON: i32 =
    JOYSTICK_FIRST_POV_BUTTON + (MAX_JOYSTICKS - 1) * JOYSTICK_POV_BUTTON_COUNT
        + JOYSTICK_POV_BUTTON_COUNT
        - 1;
/// `JOYSTICK_FIRST_AXIS_BUTTON`.
pub const JOYSTICK_FIRST_AXIS_BUTTON: i32 = JOYSTICK_LAST_POV_BUTTON + 1;
/// `JOYSTICK_LAST_AXIS_BUTTON`.
pub const JOYSTICK_LAST_AXIS_BUTTON: i32 =
    JOYSTICK_FIRST_AXIS_BUTTON + (MAX_JOYSTICKS - 1) * JOYSTICK_AXIS_BUTTON_COUNT
        + JOYSTICK_AXIS_BUTTON_COUNT
        - 1;
/// `JOYSTICK_LAST`.
pub const JOYSTICK_LAST: i32 = JOYSTICK_LAST_AXIS_BUTTON;

/// `NOVINT_FIRST`. The `+ 2` is the header's own: one code is skipped because
/// `+ 1` "seems to cause issues on the first button", and the skipped code is
/// the `FALCON_NULL` entry in the name table.
pub const NOVINT_FIRST: i32 = JOYSTICK_LAST + 2;
/// `NOVINT_LAST`.
pub const NOVINT_LAST: i32 = NOVINT_FIRST + 7;

/// `STEAMCONTROLLER_FIRST`, which is also `STEAMCONTROLLER_FIRST_BUTTON`.
pub const STEAMCONTROLLER_FIRST: i32 = NOVINT_LAST + 1;
/// `STEAMCONTROLLER_FIRST_BUTTON`.
pub const STEAMCONTROLLER_FIRST_BUTTON: i32 = STEAMCONTROLLER_FIRST;
/// `STEAMCONTROLLER_LAST_BUTTON`.
pub const STEAMCONTROLLER_LAST_BUTTON: i32 = STEAMCONTROLLER_FIRST_BUTTON
    + (MAX_STEAM_CONTROLLERS - 1) * STEAMCONTROLLER_MAX_BUTTON_COUNT
    + STEAMCONTROLLER_MAX_BUTTON_COUNT
    - 1;
/// `STEAMCONTROLLER_FIRST_AXIS_BUTTON`.
pub const STEAMCONTROLLER_FIRST_AXIS_BUTTON: i32 = STEAMCONTROLLER_LAST_BUTTON + 1;
/// `STEAMCONTROLLER_LAST_AXIS_BUTTON`.
pub const STEAMCONTROLLER_LAST_AXIS_BUTTON: i32 = STEAMCONTROLLER_FIRST_AXIS_BUTTON
    + (MAX_STEAM_CONTROLLERS - 1) * STEAMCONTROLLER_AXIS_BUTTON_COUNT
    + STEAMCONTROLLER_AXIS_BUTTON_COUNT
    - 1;
/// `STEAMCONTROLLER_LAST`.
pub const STEAMCONTROLLER_LAST: i32 = STEAMCONTROLLER_LAST_AXIS_BUTTON;

/// `BUTTON_CODE_LAST`: one past the last code, and the length of [`NAMES`].
pub const BUTTON_CODE_LAST: i32 = STEAMCONTROLLER_LAST + 1;

/// `KEY_XBUTTON_A`.
pub const KEY_XBUTTON_A: i32 = JOYSTICK_FIRST_BUTTON;
/// `KEY_XBUTTON_B`.
pub const KEY_XBUTTON_B: i32 = JOYSTICK_FIRST_BUTTON + 1;
/// `KEY_XBUTTON_X`.
pub const KEY_XBUTTON_X: i32 = JOYSTICK_FIRST_BUTTON + 2;
/// `KEY_XBUTTON_Y`.
pub const KEY_XBUTTON_Y: i32 = JOYSTICK_FIRST_BUTTON + 3;
/// `KEY_XBUTTON_LEFT_SHOULDER`.
pub const KEY_XBUTTON_LEFT_SHOULDER: i32 = JOYSTICK_FIRST_BUTTON + 4;
/// `KEY_XBUTTON_RIGHT_SHOULDER`.
pub const KEY_XBUTTON_RIGHT_SHOULDER: i32 = JOYSTICK_FIRST_BUTTON + 5;
/// `KEY_XBUTTON_BACK`.
pub const KEY_XBUTTON_BACK: i32 = JOYSTICK_FIRST_BUTTON + 6;
/// `KEY_XBUTTON_START`.
pub const KEY_XBUTTON_START: i32 = JOYSTICK_FIRST_BUTTON + 7;
/// `KEY_XBUTTON_STICK1`.
pub const KEY_XBUTTON_STICK1: i32 = JOYSTICK_FIRST_BUTTON + 8;
/// `KEY_XBUTTON_STICK2`.
pub const KEY_XBUTTON_STICK2: i32 = JOYSTICK_FIRST_BUTTON + 9;

/// `KEY_XBUTTON_UP`.
pub const KEY_XBUTTON_UP: i32 = JOYSTICK_FIRST_POV_BUTTON;
/// `KEY_XBUTTON_RIGHT`.
pub const KEY_XBUTTON_RIGHT: i32 = JOYSTICK_FIRST_POV_BUTTON + 1;
/// `KEY_XBUTTON_DOWN`.
pub const KEY_XBUTTON_DOWN: i32 = JOYSTICK_FIRST_POV_BUTTON + 2;
/// `KEY_XBUTTON_LEFT`.
pub const KEY_XBUTTON_LEFT: i32 = JOYSTICK_FIRST_POV_BUTTON + 3;

/// `KEY_XBUTTON_LTRIGGER`, the Z-axis-positive axis button.
pub const KEY_XBUTTON_LTRIGGER: i32 = JOYSTICK_FIRST_AXIS_BUTTON + 4;
/// `KEY_XBUTTON_RTRIGGER`, the Z-axis-negative axis button.
pub const KEY_XBUTTON_RTRIGGER: i32 = JOYSTICK_FIRST_AXIS_BUTTON + 5;

/// `ANALOG_CODE_INVALID`.
pub const ANALOG_CODE_INVALID: i32 = -1;
/// `MOUSE_X`.
pub const MOUSE_X: i32 = 0;
/// `MOUSE_Y`.
pub const MOUSE_Y: i32 = 1;
/// `MOUSE_XY`, posted when either of the two above changes.
pub const MOUSE_XY: i32 = 2;
/// `MOUSE_WHEEL`.
pub const MOUSE_WHEEL: i32 = 3;
/// `JOYSTICK_FIRST_AXIS`.
pub const JOYSTICK_FIRST_AXIS: i32 = 4;
/// `ANALOG_CODE_LAST`, and the length of [`ANALOG_NAMES`].
pub const ANALOG_CODE_LAST: i32 = JOYSTICK_FIRST_AXIS + MAX_JOYSTICKS * MAX_JOYSTICK_AXES;

/// `JOY_AXIS_X`, in the non-Linux order this platform uses.
pub const JOY_AXIS_X: i32 = 0;
/// `JOY_AXIS_Y`.
pub const JOY_AXIS_Y: i32 = 1;
/// `JOY_AXIS_Z`.
pub const JOY_AXIS_Z: i32 = 2;
/// `JOY_AXIS_R`.
pub const JOY_AXIS_R: i32 = 3;
/// `JOY_AXIS_U`.
pub const JOY_AXIS_U: i32 = 4;

/// `JOYSTICK_AXIS( joystick, axis )`.
#[must_use]
pub const fn joystick_axis(joystick: i32, axis: i32) -> i32 {
    JOYSTICK_FIRST_AXIS + joystick * MAX_JOYSTICK_AXES + axis
}

/// `JOYSTICK_BUTTON( joystick, button )`.
#[must_use]
pub const fn joystick_button(joystick: i32, button: i32) -> i32 {
    JOYSTICK_FIRST_BUTTON + joystick * JOYSTICK_MAX_BUTTON_COUNT + button
}

/// `JOYSTICK_POV_BUTTON( joystick, button )`.
#[must_use]
pub const fn joystick_pov_button(joystick: i32, button: i32) -> i32 {
    JOYSTICK_FIRST_POV_BUTTON + joystick * JOYSTICK_POV_BUTTON_COUNT + button
}

/// `STEAMCONTROLLER_BUTTON( joystick, button )`.
#[must_use]
pub const fn steamcontroller_button(joystick: i32, button: i32) -> i32 {
    STEAMCONTROLLER_FIRST_BUTTON + joystick * STEAMCONTROLLER_MAX_BUTTON_COUNT + button
}

/// `STEAMCONTROLLER_AXIS_BUTTON( joystick, button )`.
#[must_use]
pub const fn steamcontroller_axis_button(joystick: i32, button: i32) -> i32 {
    STEAMCONTROLLER_FIRST_AXIS_BUTTON + joystick * STEAMCONTROLLER_AXIS_BUTTON_COUNT + button
}

/// `IsJoystickButtonCode`, which counts the Novint range as the header does.
#[must_use]
pub const fn is_joystick_button_code(code: i32) -> bool {
    (code >= JOYSTICK_FIRST_BUTTON && code <= JOYSTICK_LAST_BUTTON)
        || (code >= NOVINT_FIRST && code <= NOVINT_LAST)
}

/// `IsJoystickPOVCode`.
#[must_use]
pub const fn is_joystick_pov_code(code: i32) -> bool {
    code >= JOYSTICK_FIRST_POV_BUTTON && code <= JOYSTICK_LAST_POV_BUTTON
}

/// `IsJoystickAxisCode`.
#[must_use]
pub const fn is_joystick_axis_code(code: i32) -> bool {
    code >= JOYSTICK_FIRST_AXIS_BUTTON && code <= JOYSTICK_LAST_AXIS_BUTTON
}

/// `IsSteamControllerButtonCode`.
#[must_use]
pub const fn is_steamcontroller_button_code(code: i32) -> bool {
    code >= STEAMCONTROLLER_FIRST_BUTTON && code <= STEAMCONTROLLER_LAST_BUTTON
}

/// `IsSteamControllerAxisCode`.
#[must_use]
pub const fn is_steamcontroller_axis_code(code: i32) -> bool {
    code >= STEAMCONTROLLER_FIRST_AXIS_BUTTON && code <= STEAMCONTROLLER_LAST_AXIS_BUTTON
}

// The first 114 names, through the two mouse-wheel pseudo-buttons.
const KEYBOARD_AND_MOUSE_NAMES: [&CStr; 114] = [
    c"", c"0", c"1", c"2", c"3", c"4", c"5", c"6", c"7", c"8", c"9", c"a", c"b", c"c", c"d", c"e",
    c"f", c"g", c"h", c"i", c"j", c"k", c"l", c"m", c"n", c"o", c"p", c"q", c"r", c"s", c"t", c"u",
    c"v", c"w", c"x", c"y", c"z", c"KP_INS", c"KP_END", c"KP_DOWNARROW", c"KP_PGDN",
    c"KP_LEFTARROW", c"KP_5", c"KP_RIGHTARROW", c"KP_HOME", c"KP_UPARROW", c"KP_PGUP", c"KP_SLASH",
    c"KP_MULTIPLY", c"KP_MINUS", c"KP_PLUS", c"KP_ENTER", c"KP_DEL", c"[", c"]", c"SEMICOLON",
    c"'", c"`", c",", c".", c"/", c"\\", c"-", c"=", c"ENTER", c"SPACE", c"BACKSPACE", c"TAB",
    c"CAPSLOCK", c"NUMLOCK", c"ESCAPE", c"SCROLLLOCK", c"INS", c"DEL", c"HOME", c"END", c"PGUP",
    c"PGDN", c"PAUSE", c"SHIFT", c"RSHIFT", c"ALT", c"RALT", c"CTRL", c"RCTRL", c"LWIN", c"RWIN",
    c"APP", c"UPARROW", c"LEFTARROW", c"DOWNARROW", c"RIGHTARROW", c"F1", c"F2", c"F3", c"F4",
    c"F5", c"F6", c"F7", c"F8", c"F9", c"F10", c"F11", c"F12", c"CAPSLOCKTOGGLE", c"NUMLOCKTOGGLE",
    c"SCROLLLOCKTOGGLE", c"MOUSE1", c"MOUSE2", c"MOUSE3", c"MOUSE4", c"MOUSE5", c"MWHEELUP",
    c"MWHEELDOWN",
];

// The 32 joystick buttons, 4 POV buttons, 12 axis buttons, the unused code the
// header skips, and the 8 Novint buttons. `_LINUX` is not defined here, so the
// first ten are the `JOY1`..`JOY10` branch rather than `A_BUTTON`..`STICK2`.
const JOYSTICK_AND_NOVINT_NAMES: [&CStr; 57] = [
    c"JOY1", c"JOY2", c"JOY3", c"JOY4", c"JOY5", c"JOY6", c"JOY7", c"JOY8", c"JOY9", c"JOY10",
    c"JOY11", c"JOY12", c"JOY13", c"JOY14", c"JOY15", c"JOY16", c"JOY17", c"JOY18", c"JOY19",
    c"JOY20", c"JOY21", c"JOY22", c"JOY23", c"JOY24", c"JOY25", c"JOY26", c"JOY27", c"JOY28",
    c"JOY29", c"JOY30", c"JOY31", c"JOY32", c"POV_UP", c"POV_RIGHT", c"POV_DOWN", c"POV_LEFT",
    c"X AXIS POS", c"X AXIS NEG", c"Y AXIS POS", c"Y AXIS NEG", c"Z AXIS POS", c"Z AXIS NEG",
    c"R AXIS POS", c"R AXIS NEG", c"U AXIS POS", c"U AXIS NEG", c"V AXIS POS", c"V AXIS NEG",
    c"FALCON_NULL", c"FALCON_1", c"FALCON_2", c"FALCON_3", c"FALCON_4", c"FALCON2_1",
    c"FALCON2_2", c"FALCON2_3", c"FALCON2_4",
];

// `SCONTROLLERBUTTONS_BUTTONS`. Its macro parameter is never used, so all
// eight expansions are the same thirty names.
const SC_BUTTON_NAMES: [&CStr; 30] = [
    c"SC_A", c"SC_B", c"SC_X", c"SC_Y", c"SC_DPAD_UP", c"SC_DPAD_RIGHT", c"SC_DPAD_DOWN",
    c"SC_DPAD_LEFT", c"SC_LEFT_BUMPER", c"SC_RIGHT_BUMPER", c"SC_LEFT_TRIGGER",
    c"SC_RIGHT_TRIGGER", c"SC_LEFT_GRIP", c"SC_RIGHT_GRIP", c"SC_LEFT_PAD_TOUCH",
    c"SC_RIGHT_PAD_TOUCH", c"SC_LEFT_PAD_CLICK", c"SC_RIGHT_PAD_CLICK", c"SC_LPAD_UP",
    c"SC_LPAD_RIGHT", c"SC_LPAD_DOWN", c"SC_LPAD_LEFT", c"SC_RPAD_UP", c"SC_RPAD_RIGHT",
    c"SC_RPAD_DOWN", c"SC_RPAD_LEFT", c"SC_SELECT", c"SC_START", c"SC_STEAM", c"SC_NULL",
];

// `SCONTROLLERBUTTONS_AXIS`.
const SC_AXIS_NAMES: [&CStr; 16] = [
    c"SC_LPAD_AXIS_RIGHT", c"SC_LPAD_AXIS_LEFT", c"SC_LPAD_AXIS_DOWN", c"SC_LPAD_AXIS_UP",
    c"SC_AXIS_L_TRIGGER", c"SC_AXIS_R_TRIGGER", c"SC_RPAD_AXIS_RIGHT", c"SC_RPAD_AXIS_LEFT",
    c"SC_RPAD_AXIS_DOWN", c"SC_RPAD_AXIS_UP", c"SC_GYRO_AXIS_PITCH_POSITIVE",
    c"SC_GYRO_AXIS_PITCH_NEGATIVE", c"SC_GYRO_AXIS_ROLL_POSITIVE", c"SC_GYRO_AXIS_ROLL_NEGATIVE",
    c"SC_GYRO_AXIS_YAW_POSITIVE", c"SC_GYRO_AXIS_YAW_NEGATIVE",
];

// `SCONTROLLERBUTTONS_VBUTTONS`.
const SC_VBUTTON_NAMES: [&CStr; 12] = [
    c"SC_F1", c"SC_F2", c"SC_F3", c"SC_F4", c"SC_F5", c"SC_F6", c"SC_F7", c"SC_F8", c"SC_F9",
    c"SC_F10", c"SC_F11", c"SC_F12",
];

const fn build_names() -> [&'static CStr; BUTTON_CODE_LAST as usize] {
    let mut table = [c""; BUTTON_CODE_LAST as usize];
    let mut at = 0;

    let mut i = 0;
    while i < KEYBOARD_AND_MOUSE_NAMES.len() {
        table[at] = KEYBOARD_AND_MOUSE_NAMES[i];
        at += 1;
        i += 1;
    }

    i = 0;
    while i < JOYSTICK_AND_NOVINT_NAMES.len() {
        table[at] = JOYSTICK_AND_NOVINT_NAMES[i];
        at += 1;
        i += 1;
    }

    // The three Steam Controller groups are laid out group-by-group rather
    // than controller-by-controller, which is not where the code arithmetic
    // puts them. That is the C++ table's own shape and a lookup only ever
    // indexes it, so it is reproduced rather than corrected.
    let mut pad = 0;
    while pad < MAX_STEAM_CONTROLLERS {
        i = 0;
        while i < SC_BUTTON_NAMES.len() {
            table[at] = SC_BUTTON_NAMES[i];
            at += 1;
            i += 1;
        }
        pad += 1;
    }
    pad = 0;
    while pad < MAX_STEAM_CONTROLLERS {
        i = 0;
        while i < SC_AXIS_NAMES.len() {
            table[at] = SC_AXIS_NAMES[i];
            at += 1;
            i += 1;
        }
        pad += 1;
    }
    pad = 0;
    while pad < MAX_STEAM_CONTROLLERS {
        i = 0;
        while i < SC_VBUTTON_NAMES.len() {
            table[at] = SC_VBUTTON_NAMES[i];
            at += 1;
            i += 1;
        }
        pad += 1;
    }

    assert!(at == BUTTON_CODE_LAST as usize);
    table
}

/// `s_pButtonCodeName`, one name per code.
pub static NAMES: [&CStr; BUTTON_CODE_LAST as usize] = build_names();

/// `s_pAnalogCodeName`.
pub static ANALOG_NAMES: [&CStr; ANALOG_CODE_LAST as usize] = [
    c"MOUSE_X", c"MOUSE_Y", c"MOUSE_XY", c"MOUSE_WHEEL", c"X AXIS", c"Y AXIS", c"Z AXIS",
    c"R AXIS", c"U AXIS", c"V AXIS",
];

/// `s_pXControllerButtonCodeNames`: the joystick range renamed for a gamepad,
/// covering `JOYSTICK_FIRST_BUTTON` through `JOYSTICK_LAST_AXIS_BUTTON`.
pub static XCONTROLLER_NAMES: [&CStr; 48] = [
    c"A_BUTTON", c"B_BUTTON", c"X_BUTTON", c"Y_BUTTON", c"L_SHOULDER", c"R_SHOULDER", c"BACK",
    c"START", c"STICK1", c"STICK2", c"JOY11", c"JOY12", c"JOY13", c"JOY14", c"JOY15", c"JOY16",
    c"JOY17", c"JOY18", c"JOY19", c"JOY20", c"JOY21", c"JOY22", c"JOY23", c"JOY24", c"JOY25",
    c"JOY26", c"JOY27", c"JOY28", c"JOY29", c"JOY30", c"JOY31", c"JOY32", c"UP", c"RIGHT",
    c"DOWN", c"LEFT", c"S1_RIGHT", c"S1_LEFT", c"S1_DOWN", c"S1_UP", c"L_TRIGGER", c"R_TRIGGER",
    c"S2_RIGHT", c"S2_LEFT", c"S2_DOWN", c"S2_UP", c"V AXIS POS", c"V AXIS NEG",
];

const fn build_virtual_key_table() -> [i32; 256] {
    // `ButtonCode_InitKeyTranslationTable` memsets the whole table to
    // `KEY_NONE` and, off Windows, fills only the ASCII letters and digits and
    // the eleven OEM codes below. The `VK_*` entries are Windows-only.
    let mut table = [KEY_NONE; 256];

    let mut i = 0;
    while i < 10 {
        table[b'0' as usize + i as usize] = KEY_0 + i;
        i += 1;
    }
    i = 0;
    while i < 26 {
        table[b'A' as usize + i as usize] = KEY_A + i;
        i += 1;
    }

    table[0xdb] = KEY_LBRACKET;
    table[0xdd] = KEY_RBRACKET;
    table[0xba] = KEY_SEMICOLON;
    table[0xde] = KEY_APOSTROPHE;
    table[0xc0] = KEY_BACKQUOTE;
    table[0xbc] = KEY_COMMA;
    table[0xbe] = KEY_PERIOD;
    table[0xbf] = KEY_SLASH;
    table[0xdc] = KEY_BACKSLASH;
    table[0xbd] = KEY_MINUS;
    table[0xbb] = KEY_EQUAL;

    table
}

static VIRTUAL_KEY_TO_BUTTON_CODE: [i32; 256] = build_virtual_key_table();

const fn build_button_code_to_virtual() -> [i32; BUTTON_CODE_LAST as usize] {
    // The reverse table is built by sweeping the forward one, so every code it
    // does not name keeps the zero a file-scope array starts with. Codes the
    // forward table maps more than once would keep the last virtual key; none
    // does off Windows.
    let mut table = [0; BUTTON_CODE_LAST as usize];
    let mut i = 0;
    while i < VIRTUAL_KEY_TO_BUTTON_CODE.len() {
        table[VIRTUAL_KEY_TO_BUTTON_CODE[i] as usize] = i as i32;
        i += 1;
    }
    table[0] = 0;
    table
}

static BUTTON_CODE_TO_VIRTUAL: [i32; BUTTON_CODE_LAST as usize] = build_button_code_to_virtual();

/// `s_pScanToButtonCode_QWERTY`, which on this platform is also
/// `s_pScanToButtonCode`: the layout fixup is Windows-only.
#[rustfmt::skip]
static SCAN_TO_BUTTON_CODE: [i32; 128] = {
    const N: i32 = KEY_NONE;
    [
        //  x0            x1              x2              x3              x4              x5              x6              x7
        //  x8            x9              xA              xB              xC              xD              xE              xF
        N,                KEY_ESCAPE,     key(b'1'),      key(b'2'),      key(b'3'),      key(b'4'),      key(b'5'),      key(b'6'),      // 0x
        key(b'7'),        key(b'8'),      key(b'9'),      key(b'0'),      KEY_MINUS,      KEY_EQUAL,      KEY_BACKSPACE,  KEY_TAB,

        key(b'q'),        key(b'w'),      key(b'e'),      key(b'r'),      key(b't'),      key(b'y'),      key(b'u'),      key(b'i'),      // 1x
        key(b'o'),        key(b'p'),      KEY_LBRACKET,   KEY_RBRACKET,   KEY_ENTER,      KEY_LCONTROL,   key(b'a'),      key(b's'),

        key(b'd'),        key(b'f'),      key(b'g'),      key(b'h'),      key(b'j'),      key(b'k'),      key(b'l'),      KEY_SEMICOLON,  // 2x
        KEY_APOSTROPHE,   KEY_BACKQUOTE,  KEY_LSHIFT,     KEY_BACKSLASH,  key(b'z'),      key(b'x'),      key(b'c'),      key(b'v'),

        key(b'b'),        key(b'n'),      key(b'm'),      KEY_COMMA,      KEY_PERIOD,     KEY_SLASH,      KEY_RSHIFT,     KEY_PAD_MULTIPLY, // 3x
        KEY_LALT,         KEY_SPACE,      KEY_CAPSLOCK,   key_f(1),       key_f(2),       key_f(3),       key_f(4),       key_f(5),

        key_f(6),         key_f(7),       key_f(8),       key_f(9),       key_f(10),      KEY_NUMLOCK,    KEY_SCROLLLOCK, KEY_HOME,       // 4x
        KEY_UP,           KEY_PAGEUP,     KEY_PAD_MINUS,  KEY_LEFT,       key_pad(5),     KEY_RIGHT,      KEY_PAD_PLUS,   KEY_END,

        KEY_DOWN,         KEY_PAGEDOWN,   KEY_INSERT,     KEY_DELETE,     N,              N,              N,              key_f(11),      // 5x
        key_f(12),        KEY_BREAK,      N,              N,              N,              N,              N,              N,

        N, N, N, N, N, N, N, N,                                                                                                          // 6x
        N, N, N, N, N, N, N, N,

        N, N, N, N, N, N, N, N,                                                                                                          // 7x
        N, N, N, N, N, N, N, N,
    ]
};

/// `s_pSKeytoButtonCode`: `sKey_t` to the controller-0 Steam Controller code.
static SKEY_TO_BUTTON_CODE: [i32; SK_MAX_KEYS as usize] = {
    let first = STEAMCONTROLLER_FIRST_BUTTON;
    [
        KEY_NONE,
        first,      // SK_BUTTON_A          -> STEAMCONTROLLER_A
        first + 1,  // SK_BUTTON_B
        first + 2,  // SK_BUTTON_X
        first + 3,  // SK_BUTTON_Y
        first + 4,  // SK_BUTTON_UP         -> STEAMCONTROLLER_DPAD_UP
        first + 5,  // SK_BUTTON_RIGHT
        first + 6,  // SK_BUTTON_DOWN
        first + 7,  // SK_BUTTON_LEFT
        first + 8,  // SK_BUTTON_LEFT_BUMPER
        first + 9,  // SK_BUTTON_RIGHT_BUMPER
        first + 10, // SK_BUTTON_LEFT_TRIGGER
        first + 11, // SK_BUTTON_RIGHT_TRIGGER
        first + 12, // SK_BUTTON_LEFT_GRIP
        first + 13, // SK_BUTTON_RIGHT_GRIP
        first + 14, // SK_BUTTON_LPAD_TOUCH -> STEAMCONTROLLER_LEFT_PAD_FINGERDOWN
        first + 15, // SK_BUTTON_RPAD_TOUCH
        first + 16, // SK_BUTTON_LPAD_CLICK
        first + 17, // SK_BUTTON_RPAD_CLICK
        first + 18, // SK_BUTTON_LPAD_UP
        first + 19, // SK_BUTTON_LPAD_RIGHT
        first + 20, // SK_BUTTON_LPAD_DOWN
        first + 21, // SK_BUTTON_LPAD_LEFT
        first + 22, // SK_BUTTON_RPAD_UP
        first + 23, // SK_BUTTON_RPAD_RIGHT
        first + 24, // SK_BUTTON_RPAD_DOWN
        first + 25, // SK_BUTTON_RPAD_LEFT
        first + 26, // SK_BUTTON_SELECT
        first + 27, // SK_BUTTON_START
        first + 28, // SK_BUTTON_STEAM
        first + 29, // SK_BUTTON_INACTIVE_START
        first + 30, // SK_VBUTTON_F1        -> STEAMCONTROLLER_F1
        first + 31,
        first + 32,
        first + 33,
        first + 34,
        first + 35,
        first + 36,
        first + 37,
        first + 38,
        first + 39,
        first + 40,
        first + 41, // SK_VBUTTON_F12
    ]
};

/// `ButtonCode_ButtonCodeToString`.
///
/// Out-of-range codes answer `""` where the C++ indexes its table anyway;
/// no caller in the engine reaches that, and a table read past the end is not
/// something worth reproducing.
#[must_use]
pub fn button_code_to_string(code: i32, x_controller: bool) -> &'static CStr {
    if x_controller && (JOYSTICK_FIRST_BUTTON..=JOYSTICK_LAST_AXIS_BUTTON).contains(&code) {
        return XCONTROLLER_NAMES[(code - JOYSTICK_FIRST_BUTTON) as usize];
    }
    usize::try_from(code)
        .ok()
        .and_then(|code| NAMES.get(code))
        .copied()
        .unwrap_or(c"")
}

/// `AnalogCode_AnalogCodeToString`, bounds-checked for the same reason.
#[must_use]
pub fn analog_code_to_string(code: i32) -> &'static CStr {
    usize::try_from(code)
        .ok()
        .and_then(|code| ANALOG_NAMES.get(code))
        .copied()
        .unwrap_or(c"")
}

/// `ButtonCode_StringToButtonCode`.
///
/// The `auxN` branch is Valve's back-compatibility for older joystick button
/// names and is tried before the tables, so a binding written as `aux3` still
/// resolves. `atoi` semantics are reproduced, including that it yields 0 for
/// text that is not a number: `"auxwhatever"` is joystick button 0, as it is
/// in the C++.
#[must_use]
pub fn string_to_button_code(name: &CStr, x_controller: bool) -> i32 {
    let bytes = name.to_bytes();
    if bytes.is_empty() {
        return BUTTON_CODE_INVALID;
    }

    if bytes.len() >= 3 && bytes[..3].eq_ignore_ascii_case(b"aux") {
        let index = atoi(&bytes[3..]);
        if index < 29 {
            return joystick_button(0, index);
        }
        if (29..=32).contains(&index) {
            return joystick_pov_button(0, index - 29);
        }
        return BUTTON_CODE_INVALID;
    }

    if let Some(code) = NAMES
        .iter()
        .position(|candidate| candidate.to_bytes().eq_ignore_ascii_case(bytes))
    {
        return code as i32;
    }

    if x_controller {
        if let Some(offset) = XCONTROLLER_NAMES
            .iter()
            .position(|candidate| candidate.to_bytes().eq_ignore_ascii_case(bytes))
        {
            return JOYSTICK_FIRST_BUTTON + offset as i32;
        }
    }

    BUTTON_CODE_INVALID
}

/// `AnalogCode_StringToAnalogCode`.
#[must_use]
pub fn string_to_analog_code(name: &CStr) -> i32 {
    let bytes = name.to_bytes();
    if bytes.is_empty() {
        return ANALOG_CODE_INVALID;
    }
    ANALOG_NAMES
        .iter()
        .position(|candidate| candidate.to_bytes().eq_ignore_ascii_case(bytes))
        .map_or(ANALOG_CODE_INVALID, |code| code as i32)
}

/// C's `atoi` over the bytes after `aux`: optional sign, then digits, stopping
/// at the first byte that is not one, and saturating rather than wrapping.
fn atoi(bytes: &[u8]) -> i32 {
    let mut at = 0;
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    let negative = match bytes.get(at) {
        Some(b'-') => {
            at += 1;
            true
        }
        Some(b'+') => {
            at += 1;
            false
        }
        _ => false,
    };
    let mut value: i32 = 0;
    while let Some(digit) = bytes.get(at).and_then(|byte| (*byte as char).to_digit(10)) {
        value = value
            .saturating_mul(10)
            .saturating_add(i32::try_from(digit).unwrap_or(0));
        at += 1;
    }
    if negative {
        -value
    } else {
        value
    }
}

/// `ButtonCode_VirtualKeyToButtonCode`.
#[must_use]
pub fn virtual_key_to_button_code(key: i32) -> i32 {
    usize::try_from(key)
        .ok()
        .and_then(|key| VIRTUAL_KEY_TO_BUTTON_CODE.get(key))
        .copied()
        .unwrap_or(KEY_NONE)
}

/// `ButtonCode_ButtonCodeToVirtualKey`.
#[must_use]
pub fn button_code_to_virtual_key(code: i32) -> i32 {
    usize::try_from(code)
        .ok()
        .and_then(|code| BUTTON_CODE_TO_VIRTUAL.get(code))
        .copied()
        .unwrap_or(0)
}

/// `ButtonCode_ScanCodeToButtonCode`. `lparam` carries the scan code in bits
/// 16..23 and the extended flag in bit 24, as a Windows keyboard message does;
/// the engine keeps calling it that way off Windows.
#[must_use]
pub fn scan_code_to_button_code(lparam: i32) -> i32 {
    let scan_code = ((lparam >> 16) & 0xFF) as usize;
    if scan_code > 127 {
        return KEY_NONE;
    }
    let result = SCAN_TO_BUTTON_CODE[scan_code];

    if lparam & (1 << 24) == 0 {
        // The keypad shares scan codes with the navigation block; without the
        // extended flag the key really was on the keypad.
        match result {
            KEY_HOME => KEY_PAD_7,
            KEY_UP => KEY_PAD_8,
            KEY_PAGEUP => KEY_PAD_9,
            KEY_LEFT => KEY_PAD_4,
            KEY_RIGHT => KEY_PAD_6,
            KEY_END => KEY_PAD_1,
            KEY_DOWN => KEY_PAD_2,
            KEY_PAGEDOWN => KEY_PAD_3,
            KEY_INSERT => KEY_PAD_0,
            KEY_DELETE => KEY_PAD_DECIMAL,
            other => other,
        }
    } else {
        match result {
            KEY_ENTER => KEY_PAD_ENTER,
            KEY_LALT => KEY_RALT,
            KEY_LCONTROL => KEY_RCONTROL,
            KEY_SLASH => KEY_PAD_DIVIDE,
            // The C++ really does map capslock to the keypad plus here.
            KEY_CAPSLOCK => KEY_PAD_PLUS,
            other => other,
        }
    }
}

/// `ButtonCode_SKeyToButtonCode`: a Steam Controller key on a given pad.
#[must_use]
pub fn skey_to_button_code(port: i32, key: i32) -> i32 {
    let Some(code) = usize::try_from(key)
        .ok()
        .and_then(|key| SKEY_TO_BUTTON_CODE.get(key))
        .copied()
    else {
        return KEY_NONE;
    };

    if is_steamcontroller_button_code(code) {
        return steamcontroller_button(port, code - STEAMCONTROLLER_FIRST_BUTTON);
    }
    if is_steamcontroller_axis_code(code) {
        return steamcontroller_axis_button(port, code - STEAMCONTROLLER_FIRST_AXIS_BUTTON);
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_enum_arithmetic_lands_where_the_headers_say() {
        assert_eq!(KEY_LAST, 106);
        assert_eq!(MOUSE_FIRST, 107);
        assert_eq!(MOUSE_LAST, 113);
        assert_eq!(MOUSE_COUNT, 7);
        assert_eq!(JOYSTICK_FIRST_BUTTON, 114);
        assert_eq!(JOYSTICK_LAST_BUTTON, 145);
        assert_eq!(JOYSTICK_FIRST_POV_BUTTON, 146);
        assert_eq!(JOYSTICK_LAST_POV_BUTTON, 149);
        assert_eq!(JOYSTICK_FIRST_AXIS_BUTTON, 150);
        assert_eq!(JOYSTICK_LAST_AXIS_BUTTON, 161);
        assert_eq!(NOVINT_FIRST, 163);
        assert_eq!(NOVINT_LAST, 170);
        assert_eq!(STEAMCONTROLLER_FIRST_BUTTON, 171);
        assert_eq!(STEAMCONTROLLER_LAST_BUTTON, 506);
        assert_eq!(STEAMCONTROLLER_FIRST_AXIS_BUTTON, 507);
        assert_eq!(STEAMCONTROLLER_LAST_AXIS_BUTTON, 634);
        assert_eq!(BUTTON_CODE_LAST, 635);
        assert_eq!(ANALOG_CODE_LAST, 10);
    }

    #[test]
    fn the_name_tables_are_exactly_as_long_as_the_enums() {
        // The C++ pins these with COMPILE_TIME_ASSERT; the same two facts are
        // the only cheap check that the tables were transcribed whole.
        assert_eq!(NAMES.len(), BUTTON_CODE_LAST as usize);
        assert_eq!(ANALOG_NAMES.len(), ANALOG_CODE_LAST as usize);
        assert_eq!(
            XCONTROLLER_NAMES.len(),
            (JOYSTICK_LAST_AXIS_BUTTON - JOYSTICK_FIRST_BUTTON + 1) as usize
        );
    }

    #[test]
    fn names_land_on_the_codes_they_belong_to() {
        assert_eq!(NAMES[KEY_NONE as usize], c"");
        assert_eq!(NAMES[KEY_A as usize], c"a");
        assert_eq!(NAMES[KEY_Z as usize], c"z");
        assert_eq!(NAMES[KEY_LAST as usize], c"SCROLLLOCKTOGGLE");
        assert_eq!(NAMES[MOUSE_LEFT as usize], c"MOUSE1");
        assert_eq!(NAMES[MOUSE_WHEEL_DOWN as usize], c"MWHEELDOWN");
        assert_eq!(NAMES[JOYSTICK_FIRST_BUTTON as usize], c"JOY1");
        assert_eq!(NAMES[JOYSTICK_LAST_BUTTON as usize], c"JOY32");
        assert_eq!(NAMES[JOYSTICK_FIRST_POV_BUTTON as usize], c"POV_UP");
        assert_eq!(NAMES[JOYSTICK_FIRST_AXIS_BUTTON as usize], c"X AXIS POS");
        assert_eq!(NAMES[JOYSTICK_LAST_AXIS_BUTTON as usize], c"V AXIS NEG");
        assert_eq!(NAMES[(NOVINT_FIRST - 1) as usize], c"FALCON_NULL");
        assert_eq!(NAMES[NOVINT_LAST as usize], c"FALCON2_4");
        assert_eq!(NAMES[STEAMCONTROLLER_FIRST_BUTTON as usize], c"SC_A");
        assert_eq!(NAMES[(BUTTON_CODE_LAST - 1) as usize], c"SC_F12");
    }

    #[test]
    fn strings_round_trip_back_to_their_codes() {
        for code in 0..BUTTON_CODE_LAST {
            let name = button_code_to_string(code, false);
            if name.is_empty() {
                continue;
            }
            // A name may be shared by several codes; the search answers the
            // first, so a round trip is only required to reach a code with the
            // same name, which is what the engine's bindings rely on.
            let back = string_to_button_code(name, false);
            assert_eq!(button_code_to_string(back, false), name, "code {code}");
        }
        for code in 0..ANALOG_CODE_LAST {
            let name = analog_code_to_string(code);
            assert_eq!(string_to_analog_code(name), code);
        }
    }

    #[test]
    fn the_aux_back_compat_branch_matches_atoi() {
        assert_eq!(string_to_button_code(c"aux0", false), JOYSTICK_FIRST_BUTTON);
        assert_eq!(
            string_to_button_code(c"aux28", false),
            joystick_button(0, 28)
        );
        assert_eq!(
            string_to_button_code(c"aux29", false),
            JOYSTICK_FIRST_POV_BUTTON
        );
        assert_eq!(
            string_to_button_code(c"aux32", false),
            joystick_pov_button(0, 3)
        );
        assert_eq!(string_to_button_code(c"aux33", false), BUTTON_CODE_INVALID);
        // atoi of nothing is zero, so these are joystick button 0.
        assert_eq!(string_to_button_code(c"aux", false), JOYSTICK_FIRST_BUTTON);
        assert_eq!(
            string_to_button_code(c"AUXnonsense", false),
            JOYSTICK_FIRST_BUTTON
        );
        assert_eq!(string_to_button_code(c"", false), BUTTON_CODE_INVALID);
    }

    #[test]
    fn the_x_controller_names_shadow_the_joystick_range_both_ways() {
        assert_eq!(
            button_code_to_string(KEY_XBUTTON_A, true),
            c"A_BUTTON",
            "a gamepad renames joystick button 0"
        );
        assert_eq!(button_code_to_string(KEY_XBUTTON_A, false), c"JOY1");
        assert_eq!(string_to_button_code(c"A_BUTTON", true), KEY_XBUTTON_A);
        assert_eq!(
            string_to_button_code(c"A_BUTTON", false),
            BUTTON_CODE_INVALID
        );
        // Outside the joystick range the flag changes nothing.
        assert_eq!(button_code_to_string(KEY_A, true), c"a");
    }

    #[test]
    fn virtual_keys_go_both_ways_and_nothing_else_is_mapped() {
        assert_eq!(virtual_key_to_button_code(i32::from(b'A')), KEY_A);
        assert_eq!(virtual_key_to_button_code(i32::from(b'0')), KEY_0);
        assert_eq!(virtual_key_to_button_code(0xdb), KEY_LBRACKET);
        assert_eq!(virtual_key_to_button_code(-1), KEY_NONE);
        assert_eq!(virtual_key_to_button_code(256), KEY_NONE);
        assert_eq!(virtual_key_to_button_code(0x10), KEY_NONE);

        assert_eq!(button_code_to_virtual_key(KEY_A), i32::from(b'A'));
        assert_eq!(button_code_to_virtual_key(KEY_9), i32::from(b'9'));
        assert_eq!(button_code_to_virtual_key(KEY_EQUAL), 0xbb);
        assert_eq!(button_code_to_virtual_key(KEY_NONE), 0);
        assert_eq!(button_code_to_virtual_key(MOUSE_LEFT), 0);

        for key in 0..256 {
            let code = virtual_key_to_button_code(key);
            if code != KEY_NONE {
                assert_eq!(button_code_to_virtual_key(code), key);
            }
        }
    }

    #[test]
    fn scan_codes_pick_the_keypad_when_the_extended_bit_is_clear() {
        let lparam = |scan: i32, extended: bool| (scan << 16) | (i32::from(extended) << 24);

        // Scan code 0x47 is HOME on the navigation block, keypad 7 without it.
        assert_eq!(scan_code_to_button_code(lparam(0x47, true)), KEY_HOME);
        assert_eq!(scan_code_to_button_code(lparam(0x47, false)), KEY_PAD_7);
        // 0x1c is ENTER; extended makes it the keypad's.
        assert_eq!(scan_code_to_button_code(lparam(0x1c, false)), KEY_ENTER);
        assert_eq!(scan_code_to_button_code(lparam(0x1c, true)), KEY_PAD_ENTER);
        assert_eq!(scan_code_to_button_code(lparam(0x38, true)), KEY_RALT);
        assert_eq!(scan_code_to_button_code(lparam(0x1d, true)), KEY_RCONTROL);
        // Letters are unaffected by the flag.
        assert_eq!(scan_code_to_button_code(lparam(0x1e, false)), KEY_A);
        assert_eq!(scan_code_to_button_code(lparam(0x1e, true)), KEY_A);
        // Only the low byte is a scan code, so nothing can exceed the table.
        assert_eq!(scan_code_to_button_code(0x00FF_0000), KEY_NONE);
    }

    #[test]
    fn the_qwerty_scan_table_names_the_keys_it_should() {
        let at = |scan: usize| button_code_to_string(SCAN_TO_BUTTON_CODE[scan], false);
        assert_eq!(at(0x01), c"ESCAPE");
        assert_eq!(at(0x02), c"1");
        assert_eq!(at(0x0b), c"0");
        assert_eq!(at(0x10), c"q");
        assert_eq!(at(0x1e), c"a");
        assert_eq!(at(0x2c), c"z");
        assert_eq!(at(0x39), c"SPACE");
        assert_eq!(at(0x3b), c"F1");
        assert_eq!(at(0x58), c"F12");
    }

    #[test]
    fn steam_controller_keys_land_on_the_pad_they_came_from() {
        // SK_BUTTON_A on pad 0 and pad 3.
        assert_eq!(skey_to_button_code(0, 1), STEAMCONTROLLER_FIRST_BUTTON);
        assert_eq!(
            skey_to_button_code(3, 1),
            STEAMCONTROLLER_FIRST_BUTTON + 3 * STEAMCONTROLLER_MAX_BUTTON_COUNT
        );
        assert_eq!(skey_to_button_code(0, 0), KEY_NONE);
        assert_eq!(skey_to_button_code(0, SK_MAX_KEYS), KEY_NONE);
        assert_eq!(skey_to_button_code(0, -1), KEY_NONE);
    }
}
