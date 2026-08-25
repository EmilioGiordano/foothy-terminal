//! win32-input-mode: ConPTY extension that carries a full Win32 key event
//! instead of a VT byte, so combinations the VT stream cannot express
//! (Ctrl+Backspace, Ctrl+Shift+C) reach the shell intact.
//!
//! Spec: microsoft/terminal doc/specs/#4999.

use crate::Mods;

pub const ENABLE: &[u8] = b"\x1b[?9001h";
pub const DISABLE: &[u8] = b"\x1b[?9001l";

pub mod control_key_state {
    pub const RIGHT_ALT: u32 = 0x0001;
    pub const LEFT_ALT: u32 = 0x0002;
    pub const RIGHT_CTRL: u32 = 0x0004;
    pub const LEFT_CTRL: u32 = 0x0008;
    pub const SHIFT: u32 = 0x0010;
    pub const ENHANCED_KEY: u32 = 0x0100;
}

pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0D;
    pub const ESCAPE: u16 = 0x1B;
    pub const SPACE: u16 = 0x20;
    pub const PRIOR: u16 = 0x21;
    pub const NEXT: u16 = 0x22;
    pub const END: u16 = 0x23;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const INSERT: u16 = 0x2D;
    pub const DELETE: u16 = 0x2E;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub virtual_key: u16,
    pub scan_code: u16,
    pub unicode: u16,
    pub down: bool,
    pub control_key_state: u32,
    pub repeat: u16,
}

impl KeyEvent {
    pub fn down(virtual_key: u16, unicode: u16, mods: Mods) -> Self {
        Self {
            virtual_key,
            scan_code: 0,
            unicode,
            down: true,
            control_key_state: mods.to_control_key_state(),
            repeat: 1,
        }
    }

    pub fn up(self) -> Self {
        Self {
            down: false,
            ..self
        }
    }
}

impl Mods {
    pub fn to_control_key_state(self) -> u32 {
        use control_key_state as c;
        let mut state = 0;
        if self.ctrl {
            state |= c::LEFT_CTRL;
        }
        if self.alt {
            state |= c::LEFT_ALT;
        }
        if self.shift {
            state |= c::SHIFT;
        }
        state
    }
}

/// `ESC [ Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`
pub fn encode(ev: &KeyEvent) -> Vec<u8> {
    format!(
        "\x1b[{};{};{};{};{};{}_",
        ev.virtual_key,
        ev.scan_code,
        ev.unicode,
        u8::from(ev.down),
        ev.control_key_state,
        ev.repeat
    )
    .into_bytes()
}

/// A full press: key down followed by key up.
pub fn press(ev: &KeyEvent) -> Vec<u8> {
    let mut out = encode(ev);
    out.extend_from_slice(&encode(&ev.up()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_letter() {
        let ev = KeyEvent::down(b'A' as u16, 'a' as u16, Mods::NONE);
        assert_eq!(encode(&ev), b"\x1b[65;0;97;1;0;1_");
    }

    #[test]
    fn ctrl_backspace_carries_the_modifier() {
        let ev = KeyEvent::down(vk::BACK, 0, Mods::CTRL);
        // Cs = 8 = LEFT_CTRL_PRESSED, which the VT stream cannot express at all.
        assert_eq!(encode(&ev), b"\x1b[8;0;0;1;8;1_");
    }

    #[test]
    fn ctrl_backspace_differs_from_ctrl_h() {
        let bs = KeyEvent::down(vk::BACK, 0, Mods::CTRL);
        let h = KeyEvent::down(b'H' as u16, 0, Mods::CTRL);
        assert_ne!(encode(&bs), encode(&h));
    }

    #[test]
    fn ctrl_shift_c_is_distinct_from_ctrl_c() {
        let mods = Mods {
            ctrl: true,
            shift: true,
            alt: false,
        };
        let ctrl_shift_c = KeyEvent::down(b'C' as u16, 0, mods);
        let ctrl_c = KeyEvent::down(b'C' as u16, 0, Mods::CTRL);
        assert_ne!(encode(&ctrl_shift_c), encode(&ctrl_c));
    }

    #[test]
    fn key_up_flips_only_kd() {
        let down = KeyEvent::down(vk::RETURN, 13, Mods::NONE);
        assert_eq!(encode(&down.up()), b"\x1b[13;0;13;0;0;1_");
    }

    #[test]
    fn press_emits_down_then_up() {
        let ev = KeyEvent::down(vk::SPACE, 32, Mods::NONE);
        let mut expected = encode(&ev);
        expected.extend_from_slice(&encode(&ev.up()));
        assert_eq!(press(&ev), expected);
    }

    #[test]
    fn control_key_state_combines() {
        use control_key_state as c;
        let mods = Mods {
            ctrl: true,
            alt: true,
            shift: true,
        };
        assert_eq!(
            mods.to_control_key_state(),
            c::LEFT_CTRL | c::LEFT_ALT | c::SHIFT
        );
    }
}
