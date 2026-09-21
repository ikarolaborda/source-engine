//! `initKeymap` and `MapCocoaVirtualKeyToButtonCode` from `inputsystem.cpp`.
//!
//! The launcher hands this module a "Cocoa virtual key code" that is really
//! an SDL scancode when positive and a button code already negated when not.
//! Both shapes are decoded here.

use crate::codes::{
    key, key_f, key_pad, KEY_APOSTROPHE, KEY_APP, KEY_BACKQUOTE, KEY_BACKSLASH, KEY_BACKSPACE,
    KEY_CAPSLOCK, KEY_COMMA, KEY_DELETE, KEY_DOWN, KEY_END, KEY_ENTER, KEY_EQUAL, KEY_ESCAPE,
    KEY_HOME, KEY_INSERT, KEY_LALT, KEY_LBRACKET, KEY_LCONTROL, KEY_LEFT, KEY_LSHIFT, KEY_LWIN,
    KEY_MINUS, KEY_NONE, KEY_NUMLOCK, KEY_PAD_DECIMAL, KEY_PAD_DIVIDE, KEY_PAD_MINUS,
    KEY_PAD_MULTIPLY, KEY_PAD_PLUS, KEY_PAGEDOWN, KEY_PAGEUP, KEY_PERIOD, KEY_RALT, KEY_RBRACKET,
    KEY_RCONTROL, KEY_RIGHT, KEY_RSHIFT, KEY_RWIN, KEY_SCROLLLOCK, KEY_SEMICOLON, KEY_SLASH,
    KEY_SPACE, KEY_TAB, KEY_UP,
};

// SDL scancodes, read from the SDL2 headers this tree builds against. Only
// the ones `initKeymap` names are here.
const SCANCODE_A: usize = 4;
const SCANCODE_1: usize = 30;
const SCANCODE_0: usize = 39;
const SCANCODE_RETURN: usize = 40;
const SCANCODE_ESCAPE: usize = 41;
const SCANCODE_BACKSPACE: usize = 42;
const SCANCODE_TAB: usize = 43;
const SCANCODE_SPACE: usize = 44;
const SCANCODE_MINUS: usize = 45;
const SCANCODE_EQUALS: usize = 46;
const SCANCODE_LEFTBRACKET: usize = 47;
const SCANCODE_RIGHTBRACKET: usize = 48;
const SCANCODE_BACKSLASH: usize = 49;
const SCANCODE_SEMICOLON: usize = 51;
const SCANCODE_APOSTROPHE: usize = 52;
const SCANCODE_GRAVE: usize = 53;
const SCANCODE_COMMA: usize = 54;
const SCANCODE_PERIOD: usize = 55;
const SCANCODE_SLASH: usize = 56;
const SCANCODE_CAPSLOCK: usize = 57;
const SCANCODE_F1: usize = 58;
const SCANCODE_SCROLLLOCK: usize = 71;
const SCANCODE_INSERT: usize = 73;
const SCANCODE_HOME: usize = 74;
const SCANCODE_PAGEUP: usize = 75;
const SCANCODE_DELETE: usize = 76;
const SCANCODE_END: usize = 77;
const SCANCODE_PAGEDOWN: usize = 78;
const SCANCODE_RIGHT: usize = 79;
const SCANCODE_LEFT: usize = 80;
const SCANCODE_DOWN: usize = 81;
const SCANCODE_UP: usize = 82;
const SCANCODE_NUMLOCKCLEAR: usize = 83;
const SCANCODE_KP_DIVIDE: usize = 84;
const SCANCODE_KP_MULTIPLY: usize = 85;
const SCANCODE_KP_MINUS: usize = 86;
const SCANCODE_KP_PLUS: usize = 87;
const SCANCODE_KP_ENTER: usize = 88;
const SCANCODE_KP_1: usize = 89;
const SCANCODE_KP_0: usize = 98;
const SCANCODE_KP_PERIOD: usize = 99;
const SCANCODE_APPLICATION: usize = 101;
const SCANCODE_LCTRL: usize = 224;
const SCANCODE_LSHIFT: usize = 225;
const SCANCODE_LALT: usize = 226;
const SCANCODE_LGUI: usize = 227;
const SCANCODE_RCTRL: usize = 228;
const SCANCODE_RSHIFT: usize = 229;
const SCANCODE_RALT: usize = 230;
const SCANCODE_RGUI: usize = 231;

const fn build_scan_to_key() -> [i32; 256] {
    // The C++ table is `SDL_NUM_SCANCODES` long, but the only reader masks the
    // scancode to eight bits first, so entries past 255 are unreachable and
    // 256 is the whole reachable table.
    let mut table = [KEY_NONE; 256];

    let mut i = 0;
    while i < 26 {
        table[SCANCODE_A + i] = key(b'a' + i as u8);
        i += 1;
    }
    // SDL puts 1..9 together and 0 on its own, which is how the keyboard is
    // laid out rather than how the digits are numbered.
    i = 0;
    while i < 9 {
        table[SCANCODE_1 + i] = key(b'1' + i as u8);
        i += 1;
    }
    i = 0;
    while i < 12 {
        table[SCANCODE_F1 + i] = key_f(i as i32 + 1);
        i += 1;
    }
    i = 0;
    while i < 9 {
        table[SCANCODE_KP_1 + i] = key_pad(i as i32 + 1);
        i += 1;
    }

    table[SCANCODE_0] = key(b'0');
    table[SCANCODE_KP_0] = key_pad(0);
    table[SCANCODE_RETURN] = KEY_ENTER;
    table[SCANCODE_ESCAPE] = KEY_ESCAPE;
    table[SCANCODE_BACKSPACE] = KEY_BACKSPACE;
    table[SCANCODE_TAB] = KEY_TAB;
    table[SCANCODE_SPACE] = KEY_SPACE;
    table[SCANCODE_MINUS] = KEY_MINUS;
    table[SCANCODE_EQUALS] = KEY_EQUAL;
    table[SCANCODE_LEFTBRACKET] = KEY_LBRACKET;
    table[SCANCODE_RIGHTBRACKET] = KEY_RBRACKET;
    table[SCANCODE_BACKSLASH] = KEY_BACKSLASH;
    table[SCANCODE_SEMICOLON] = KEY_SEMICOLON;
    table[SCANCODE_APOSTROPHE] = KEY_APOSTROPHE;
    table[SCANCODE_GRAVE] = KEY_BACKQUOTE;
    table[SCANCODE_COMMA] = KEY_COMMA;
    table[SCANCODE_PERIOD] = KEY_PERIOD;
    table[SCANCODE_SLASH] = KEY_SLASH;
    table[SCANCODE_CAPSLOCK] = KEY_CAPSLOCK;
    table[SCANCODE_SCROLLLOCK] = KEY_SCROLLLOCK;
    table[SCANCODE_INSERT] = KEY_INSERT;
    table[SCANCODE_HOME] = KEY_HOME;
    table[SCANCODE_PAGEUP] = KEY_PAGEUP;
    table[SCANCODE_DELETE] = KEY_DELETE;
    table[SCANCODE_END] = KEY_END;
    table[SCANCODE_PAGEDOWN] = KEY_PAGEDOWN;
    table[SCANCODE_RIGHT] = KEY_RIGHT;
    table[SCANCODE_LEFT] = KEY_LEFT;
    table[SCANCODE_DOWN] = KEY_DOWN;
    table[SCANCODE_UP] = KEY_UP;
    table[SCANCODE_NUMLOCKCLEAR] = KEY_NUMLOCK;
    table[SCANCODE_KP_DIVIDE] = KEY_PAD_DIVIDE;
    table[SCANCODE_KP_MULTIPLY] = KEY_PAD_MULTIPLY;
    table[SCANCODE_KP_MINUS] = KEY_PAD_MINUS;
    table[SCANCODE_KP_PLUS] = KEY_PAD_PLUS;
    // The keypad's enter becomes the main one so vgui, which never learned
    // about KEY_PAD_ENTER, still sees a dialog being accepted.
    table[SCANCODE_KP_ENTER] = KEY_ENTER;
    table[SCANCODE_KP_PERIOD] = KEY_PAD_DECIMAL;
    table[SCANCODE_APPLICATION] = KEY_APP;
    table[SCANCODE_LCTRL] = KEY_LCONTROL;
    table[SCANCODE_LSHIFT] = KEY_LSHIFT;
    table[SCANCODE_LALT] = KEY_LALT;
    table[SCANCODE_LGUI] = KEY_LWIN;
    table[SCANCODE_RCTRL] = KEY_RCONTROL;
    table[SCANCODE_RSHIFT] = KEY_RSHIFT;
    table[SCANCODE_RALT] = KEY_RALT;
    table[SCANCODE_RGUI] = KEY_RWIN;

    table
}

static SCAN_TO_KEY: [i32; 256] = build_scan_to_key();

/// `MapCocoaVirtualKeyToButtonCode`.
///
/// A negative code is a button code the launcher already negated, so it is
/// negated back and used as-is. A non-negative one is an SDL scancode, masked
/// to eight bits before the table read exactly as the C++ masks it.
#[must_use]
pub fn cocoa_virtual_key_to_button_code(virtual_key: i32) -> i32 {
    if virtual_key < 0 {
        return -virtual_key;
    }
    SCAN_TO_KEY[(virtual_key & 0xff) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codes::{KEY_0, KEY_9, KEY_A, KEY_F1, KEY_PAD_0, KEY_PAD_9, KEY_Z};

    #[test]
    fn letters_digits_and_function_keys_come_out_in_order() {
        assert_eq!(cocoa_virtual_key_to_button_code(SCANCODE_A as i32), KEY_A);
        assert_eq!(
            cocoa_virtual_key_to_button_code(SCANCODE_A as i32 + 25),
            KEY_Z
        );
        assert_eq!(cocoa_virtual_key_to_button_code(SCANCODE_1 as i32), KEY_0 + 1);
        assert_eq!(cocoa_virtual_key_to_button_code(SCANCODE_1 as i32 + 8), KEY_9);
        assert_eq!(cocoa_virtual_key_to_button_code(SCANCODE_0 as i32), KEY_0);
        assert_eq!(cocoa_virtual_key_to_button_code(SCANCODE_F1 as i32), KEY_F1);
        assert_eq!(
            cocoa_virtual_key_to_button_code(SCANCODE_F1 as i32 + 11),
            KEY_F1 + 11
        );
        assert_eq!(
            cocoa_virtual_key_to_button_code(SCANCODE_KP_1 as i32),
            KEY_PAD_0 + 1
        );
        assert_eq!(
            cocoa_virtual_key_to_button_code(SCANCODE_KP_1 as i32 + 8),
            KEY_PAD_9
        );
        assert_eq!(
            cocoa_virtual_key_to_button_code(SCANCODE_KP_0 as i32),
            KEY_PAD_0
        );
    }

    #[test]
    fn the_keypads_enter_is_deliberately_the_same_key_as_the_main_one() {
        assert_eq!(
            cocoa_virtual_key_to_button_code(SCANCODE_KP_ENTER as i32),
            KEY_ENTER
        );
        assert_eq!(
            cocoa_virtual_key_to_button_code(SCANCODE_RETURN as i32),
            KEY_ENTER
        );
    }

    #[test]
    fn modifiers_keep_their_sides() {
        assert_eq!(
            cocoa_virtual_key_to_button_code(SCANCODE_LSHIFT as i32),
            KEY_LSHIFT
        );
        assert_eq!(
            cocoa_virtual_key_to_button_code(SCANCODE_RSHIFT as i32),
            KEY_RSHIFT
        );
        assert_eq!(
            cocoa_virtual_key_to_button_code(SCANCODE_LGUI as i32),
            KEY_LWIN
        );
        assert_eq!(
            cocoa_virtual_key_to_button_code(SCANCODE_RGUI as i32),
            KEY_RWIN
        );
    }

    #[test]
    fn a_negative_code_is_a_button_code_the_launcher_already_negated() {
        assert_eq!(cocoa_virtual_key_to_button_code(-KEY_A), KEY_A);
        assert_eq!(
            cocoa_virtual_key_to_button_code(-crate::codes::MOUSE_LEFT),
            crate::codes::MOUSE_LEFT
        );
    }

    #[test]
    fn scancodes_past_the_eighth_bit_alias_as_they_do_in_the_cpp() {
        // The mask is the C++'s; a scancode of 256 + A reads A's entry.
        assert_eq!(
            cocoa_virtual_key_to_button_code(256 + SCANCODE_A as i32),
            KEY_A
        );
        assert_eq!(cocoa_virtual_key_to_button_code(0), KEY_NONE);
        assert_eq!(cocoa_virtual_key_to_button_code(1), KEY_NONE);
    }
}
