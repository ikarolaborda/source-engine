//! The Steam Controller tables from `steamcontroller.cpp`.
//!
//! Everything else in that file is guarded by `SteamControllerInterface()`,
//! which is null in a build that links the `steam_api` stub, so it collapses
//! to the constants the ledger records. These two tables are not: the UI asks
//! what glyph or label an action's physical origin has whether or not a
//! controller is attached, so both are ported whole.
//!
//! The strings are UTF-32 because the interface returns `const wchar_t *` and
//! `wchar_t` is four bytes here. They are `'static` so the pointer the caller
//! keeps stays good.

/// `k_EControllerActionOrigin_None`.
pub const ACTION_ORIGIN_NONE: i32 = 0;

/// `g_GameActionSets`, in the order `GameActionSet_t` numbers them.
pub const ACTION_SET_NAMES: [&std::ffi::CStr; 4] = [
    c"MenuControls",
    c"FPSControls",
    c"InGameHUDControls",
    c"SpectatorControls",
];

/// A NUL-terminated UTF-32 string, which is what `const wchar_t *` is here.
pub type WideString = &'static [u32];

const EMPTY: WideString = &[0];

/// An ASCII literal as the NUL-terminated UTF-32 a `const wchar_t *` points
/// at. Each use gets its own constant, so each has `'static` storage of
/// exactly the right length.
macro_rules! wide {
    ($text:literal) => {{
        const WIDE: [u32; $text.len() + 1] = {
            let bytes = $text.as_bytes();
            let mut out = [0u32; $text.len() + 1];
            let mut i = 0;
            while i < bytes.len() {
                // One byte is one character only for ASCII, and everything in
                // these tables is; anything else would be a transcription slip.
                assert!(bytes[i].is_ascii(), "the origin tables are ASCII");
                out[i] = bytes[i] as u32;
                i += 1;
            }
            out
        };
        &WIDE as WideString
    }};
}

/// `g_MapSteamControllerOriginToIconFont`: the character in the game's Steam
/// Controller icon font that draws a physical control.
pub static ORIGIN_ICON_FONT: [WideString; 39] = [
    EMPTY,  // None
    wide!("A"), wide!("B"), wide!("X"), wide!("Y"),
    wide!("2"), // LeftBumper
    wide!("3"), // RightBumper
    wide!("("), // LeftGrip
    wide!(")"), // RightGrip
    wide!("5"), // Start
    wide!("4"), // Back
    wide!("q"), wide!("w"), wide!("e"), // LeftPad touch, swipe, click
    wide!("a"), wide!("s"), wide!("d"), wide!("f"), // LeftPad dpad N S W E
    wide!("y"), wide!("u"), wide!("i"), // RightPad touch, swipe, click
    wide!("h"), wide!("j"), wide!("k"), wide!("l"), // RightPad dpad N S W E
    wide!("z"), wide!("x"), // LeftTrigger pull, click
    wide!("n"), wide!("m"), // RightTrigger pull, click
    wide!("C"), wide!("V"), // LeftStick move, click
    wide!("7"), wide!("8"), wide!("9"), wide!("0"), // LeftStick dpad N S W E
    wide!("6"), wide!("6"), wide!("6"), wide!("6"), // Gyro move, pitch, yaw, roll
];

/// `g_MapSteamControllerOriginToDescription`: a short label for the same
/// control, for where the icon font cannot be used.
pub static ORIGIN_DESCRIPTION: [WideString; 39] = [
    EMPTY,
    wide!("A"), wide!("B"), wide!("X"), wide!("Y"),
    wide!("LB"), wide!("RB"),
    wide!("LG"), wide!("RG"),
    wide!("START"), wide!("BACK"),
    wide!("LPTOUCH"), wide!("LPSWIPE"), wide!("LPCLICK"),
    wide!("LPUP"), wide!("LPDOWN"), wide!("LPLEFT"), wide!("LPRIGHT"),
    wide!("RPTOUCH"), wide!("RPSWIPE"), wide!("RPCLICK"),
    wide!("RPUP"), wide!("RPDOWN"), wide!("RPLEFT"), wide!("RPRIGHT"),
    // All four trigger origins really do say "LT" in the C++ table.
    wide!("LT"), wide!("LT"), wide!("LT"), wide!("LT"),
    wide!("LS"), wide!("LSCLICK"),
    wide!("LSUP"), wide!("LSDOWN"), wide!("LSLEFT"), wide!("LSRIGHT"),
    wide!("GYRO"), wide!("GYRO"), wide!("GYRO"), wide!("GYRO"),
];

/// `GetSteamControllerFontCharacterForActionOrigin`, which answers the empty
/// string for an origin the table does not cover.
#[must_use]
pub fn origin_icon_font(origin: i32) -> WideString {
    lookup(&ORIGIN_ICON_FONT, origin)
}

/// `GetSteamControllerDescriptionForActionOrigin`, same guard.
#[must_use]
pub fn origin_description(origin: i32) -> WideString {
    lookup(&ORIGIN_DESCRIPTION, origin)
}

/// `GetActionSetHandle( const char * )`. Every handle is zero in a build with
/// no Steam behind it, but the name still has to be one the table knows —
/// the C++ returns zero either way, so the answer is the same and the lookup
/// is kept so a real Steamworks build only has to fill the handles in.
#[must_use]
pub fn action_set_index(name: &std::ffi::CStr) -> Option<usize> {
    ACTION_SET_NAMES
        .iter()
        .position(|candidate| *candidate == name)
}

fn lookup(table: &'static [WideString], origin: i32) -> WideString {
    usize::try_from(origin)
        .ok()
        .and_then(|origin| table.get(origin))
        .copied()
        .unwrap_or(EMPTY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: WideString) -> String {
        value
            .iter()
            .take_while(|character| **character != 0)
            .filter_map(|character| char::from_u32(*character))
            .collect()
    }

    #[test]
    fn both_origin_tables_cover_the_same_origins() {
        assert_eq!(ORIGIN_ICON_FONT.len(), ORIGIN_DESCRIPTION.len());
        assert_eq!(ORIGIN_ICON_FONT.len(), 39);
    }

    #[test]
    fn origins_map_to_the_glyph_and_label_the_cpp_tables_hold() {
        assert_eq!(text(origin_icon_font(ACTION_ORIGIN_NONE)), "");
        assert_eq!(text(origin_icon_font(1)), "A");
        assert_eq!(text(origin_icon_font(5)), "2", "left bumper");
        assert_eq!(text(origin_icon_font(38)), "6", "gyro roll, the last one");

        assert_eq!(text(origin_description(ACTION_ORIGIN_NONE)), "");
        assert_eq!(text(origin_description(5)), "LB");
        assert_eq!(text(origin_description(9)), "START");
        assert_eq!(text(origin_description(30)), "LSCLICK");
        assert_eq!(text(origin_description(38)), "GYRO");
    }

    #[test]
    fn an_origin_off_the_end_answers_the_empty_string_rather_than_reading_past_it() {
        assert_eq!(text(origin_icon_font(39)), "");
        assert_eq!(text(origin_icon_font(-1)), "");
        assert_eq!(text(origin_description(i32::MAX)), "");
        assert_eq!(origin_icon_font(39).last(), Some(&0), "still terminated");
    }

    #[test]
    fn every_entry_is_nul_terminated_so_a_caller_can_read_it_as_a_c_string() {
        for entry in ORIGIN_ICON_FONT.iter().chain(ORIGIN_DESCRIPTION.iter()) {
            assert!(entry.contains(&0), "no terminator");
        }
    }

    #[test]
    fn action_sets_are_named_in_the_order_the_enum_numbers_them() {
        assert_eq!(action_set_index(c"MenuControls"), Some(0));
        assert_eq!(action_set_index(c"SpectatorControls"), Some(3));
        assert_eq!(action_set_index(c"NoSuchSet"), None);
    }
}
