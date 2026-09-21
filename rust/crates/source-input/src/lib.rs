//! The input system's behaviour, with no FFI in it.
//!
//! [`codes`], [`keymap`], [`state`] and [`steam`] are what `inputsystem`'s
//! five translation units do once the SDL, launcher and Steam calls are taken
//! out: the button and analog code space and its tables, the SDL scancode
//! keymap, the double-buffered input state, and the Steam Controller origin
//! tables. `source-inputsystem` wraps them in the module's vtable and supplies
//! those calls.
//!
//! [`State`] below predates that and belongs to the transitional ABI that
//! `source-abi` still goes through.

pub mod codes;
pub mod keymap;
pub mod state;
pub mod steam;

/// Stateful normalization for SDL input passed through the transitional ABI.
pub const MOD_CAPS_LOCK: u32 = 1 << 0;
pub const MOD_RIGHT_SHIFT: u32 = 1 << 1;
pub const MOD_LEFT_SHIFT: u32 = 1 << 2;
pub const MOD_RIGHT_CONTROL: u32 = 1 << 3;
pub const MOD_LEFT_CONTROL: u32 = 1 << 4;
pub const MOD_RIGHT_ALT: u32 = 1 << 5;
pub const MOD_LEFT_ALT: u32 = 1 << 6;
pub const MOD_RIGHT_GUI: u32 = 1 << 7;
pub const MOD_LEFT_GUI: u32 = 1 << 8;
pub const VALID_MODIFIERS: u32 = (1 << 9) - 1;
pub const VALID_MOUSE_BUTTONS: u32 = (1 << 5) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidModifier(u32),
    InvalidMouseButton(u32),
}

#[derive(Debug, Clone, Default)]
pub struct State {
    modifiers: u32,
    mouse_buttons: u32,
    mouse_delta_x: i32,
    mouse_delta_y: i32,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update_modifier(&mut self, modifier: u32, pressed: bool) -> Result<u32, Error> {
        if modifier != 0 && (!modifier.is_power_of_two() || modifier & !VALID_MODIFIERS != 0) {
            return Err(Error::InvalidModifier(modifier));
        }
        if pressed {
            self.modifiers |= modifier;
        } else {
            self.modifiers &= !modifier;
        }
        Ok(self.modifier_mask())
    }

    pub fn modifier_mask(&self) -> u32 {
        let mut mask = 0;
        if self.modifiers & MOD_CAPS_LOCK != 0 {
            mask |= 1 << 0;
        }
        if self.modifiers & (MOD_RIGHT_SHIFT | MOD_LEFT_SHIFT) != 0 {
            mask |= 1 << 1;
        }
        if self.modifiers & (MOD_RIGHT_CONTROL | MOD_LEFT_CONTROL) != 0 {
            mask |= 1 << 2;
        }
        if self.modifiers & (MOD_RIGHT_ALT | MOD_LEFT_ALT) != 0 {
            mask |= 1 << 3;
        }
        if self.modifiers & (MOD_RIGHT_GUI | MOD_LEFT_GUI) != 0 {
            mask |= 1 << 4;
        }
        mask
    }

    pub fn update_mouse_button(&mut self, button: u32, pressed: bool) -> Result<u32, Error> {
        if !button.is_power_of_two() || button & !VALID_MOUSE_BUTTONS != 0 {
            return Err(Error::InvalidMouseButton(button));
        }
        if pressed {
            self.mouse_buttons |= button;
        } else {
            self.mouse_buttons &= !button;
        }
        Ok(self.mouse_buttons)
    }

    pub fn mouse_buttons(&self) -> u32 {
        self.mouse_buttons
    }

    pub fn add_mouse_delta(&mut self, x: i32, y: i32) {
        self.mouse_delta_x = self.mouse_delta_x.saturating_add(x);
        self.mouse_delta_y = self.mouse_delta_y.saturating_add(y);
    }

    pub fn take_mouse_delta(&mut self) -> (i32, i32) {
        let delta = (self.mouse_delta_x, self.mouse_delta_y);
        self.mouse_delta_x = 0;
        self.mouse_delta_y = 0;
        delta
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_left_and_right_modifiers_to_source_masks() {
        let mut state = State::new();
        assert_eq!(state.update_modifier(MOD_LEFT_SHIFT, true).unwrap(), 1 << 1);
        assert_eq!(
            state.update_modifier(MOD_RIGHT_SHIFT, true).unwrap(),
            1 << 1
        );
        assert_eq!(
            state.update_modifier(MOD_LEFT_SHIFT, false).unwrap(),
            1 << 1
        );
        assert_eq!(state.update_modifier(MOD_RIGHT_SHIFT, false).unwrap(), 0);
        assert!(state.update_modifier(VALID_MODIFIERS + 1, true).is_err());
    }

    #[test]
    fn tracks_buttons_and_consumes_saturating_mouse_deltas() {
        let mut state = State::new();
        assert_eq!(state.update_mouse_button(1 << 2, true).unwrap(), 1 << 2);
        assert_eq!(state.update_mouse_button(1 << 0, true).unwrap(), 5);
        assert_eq!(state.update_mouse_button(1 << 2, false).unwrap(), 1);
        assert!(state.update_mouse_button(0, true).is_err());

        state.add_mouse_delta(i32::MAX, -10);
        state.add_mouse_delta(50, 3);
        assert_eq!(state.take_mouse_delta(), (i32::MAX, -7));
        assert_eq!(state.take_mouse_delta(), (0, 0));
    }
}
