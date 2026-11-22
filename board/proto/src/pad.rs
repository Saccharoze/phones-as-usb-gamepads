/// Buttons and their report bit. Bit n is HID button n + 1, which a Linux PC reports as key code
/// 0x130 + n, so each button sits on the bit whose code carries its name: BTN_A, BTN_B, BTN_X,
/// BTN_Y, BTN_TL, BTN_TR, BTN_SELECT, BTN_START, BTN_MODE, BTN_THUMBL, BTN_THUMBR. Bits 2, 5, 8,
/// 9 and 15 (BTN_C, BTN_Z, BTN_TL2, BTN_TR2, 0x13f) are reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    A = 0,
    B = 1,
    X = 3,
    Y = 4,
    LB = 6,
    RB = 7,
    View = 10,
    Menu = 11,
    Guide = 12,
    L3 = 13,
    R3 = 14,
}

impl Button {
    pub const ALL: [Button; 11] = [
        Button::A,
        Button::B,
        Button::X,
        Button::Y,
        Button::LB,
        Button::RB,
        Button::View,
        Button::Menu,
        Button::Guide,
        Button::L3,
        Button::R3,
    ];

    /// Every bit a button uses; the others must stay zero.
    pub const USED_BITS: u16 = {
        let mut bits = 0;
        let mut i = 0;
        while i < Button::ALL.len() {
            bits |= Button::ALL[i].bit();
            i += 1;
        }
        bits
    };

    pub const fn bit(self) -> u16 {
        1 << self as u16
    }

    pub fn name(self) -> &'static str {
        match self {
            Button::A => "A",
            Button::B => "B",
            Button::X => "X",
            Button::Y => "Y",
            Button::LB => "LB",
            Button::RB => "RB",
            Button::View => "View",
            Button::Menu => "Menu",
            Button::Guide => "Guide",
            Button::L3 => "L3",
            Button::R3 => "R3",
        }
    }

    pub fn from_name(name: &str) -> Option<Button> {
        Button::ALL
            .into_iter()
            .find(|b| b.name().eq_ignore_ascii_case(name))
    }
}

/// D-pad as a HID hat switch: 0 = up, clockwise in 45° steps, 8 = released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hat {
    Up = 0,
    UpRight = 1,
    Right = 2,
    DownRight = 3,
    Down = 4,
    DownLeft = 5,
    Left = 6,
    UpLeft = 7,
    #[default]
    Neutral = 8,
}

impl Hat {
    pub fn from_value(value: u8) -> Option<Hat> {
        Some(match value {
            0 => Hat::Up,
            1 => Hat::UpRight,
            2 => Hat::Right,
            3 => Hat::DownRight,
            4 => Hat::Down,
            5 => Hat::DownLeft,
            6 => Hat::Left,
            7 => Hat::UpLeft,
            8 => Hat::Neutral,
            _ => return None,
        })
    }
}

/// Smallest axis value; -32768 is excluded so the range is symmetric around 0.
pub const AXIS_MIN: i16 = -32767;
pub const AXIS_MAX: i16 = 32767;

/// One controller's full state. Stick Y is down-positive (HID convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PadState {
    pub buttons: u16,
    pub hat: Hat,
    pub left_x: i16,
    pub left_y: i16,
    pub right_x: i16,
    pub right_y: i16,
    pub left_trigger: u8,
    pub right_trigger: u8,
}

impl PadState {
    pub const NEUTRAL: PadState = PadState {
        buttons: 0,
        hat: Hat::Neutral,
        left_x: 0,
        left_y: 0,
        right_x: 0,
        right_y: 0,
        left_trigger: 0,
        right_trigger: 0,
    };

    pub fn is_pressed(&self, button: Button) -> bool {
        self.buttons & button.bit() != 0
    }

    pub fn with_button(mut self, button: Button, pressed: bool) -> PadState {
        if pressed {
            self.buttons |= button.bit();
        } else {
            self.buttons &= !button.bit();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_bits_follow_linux_key_codes() {
        assert_eq!(Button::A.bit(), 0x0001);
        assert_eq!(Button::X.bit(), 0x0008);
        assert_eq!(Button::Menu.bit(), 0x0800);
        assert_eq!(Button::R3.bit(), 0x4000);
        assert_eq!(Button::USED_BITS, 0x7CDB);
    }

    #[test]
    fn button_names_round_trip_case_insensitively() {
        for b in Button::ALL {
            assert_eq!(Button::from_name(&b.name().to_lowercase()), Some(b));
        }
        assert_eq!(Button::from_name("Z"), None);
    }

    #[test]
    fn with_button_sets_and_clears() {
        let s = PadState::NEUTRAL.with_button(Button::B, true);
        assert!(s.is_pressed(Button::B));
        assert_eq!(s.with_button(Button::B, false), PadState::NEUTRAL);
    }

    #[test]
    fn default_is_neutral() {
        assert_eq!(PadState::default(), PadState::NEUTRAL);
    }
}
