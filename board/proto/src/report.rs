//! The 13-byte input report, byte for byte as declared in `descriptor::PAD`.

use crate::pad::{AXIS_MIN, Button, Hat, PadState};

pub const REPORT_LEN: usize = 13;

/// Button bits no button uses (2, 5, 8, 9, 15) must stay zero.
const RESERVED_BUTTONS: u16 = !Button::USED_BITS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReportError {
    #[error("reserved button bits set: {0:#06x}")]
    ReservedButtons(u16),
    #[error("hat value {0} out of range 0..=8")]
    BadHat(u8),
    #[error("D-pad padding bits set")]
    HatPadding,
    #[error("axis value -32768 is outside the declared range")]
    AxisOutOfRange,
}

pub fn encode(state: &PadState) -> [u8; REPORT_LEN] {
    let mut out = [0u8; REPORT_LEN];
    out[0..2].copy_from_slice(&(state.buttons & !RESERVED_BUTTONS).to_le_bytes());
    out[2] = state.hat as u8;
    put_axis(&mut out[3..5], state.left_x);
    put_axis(&mut out[5..7], state.left_y);
    put_axis(&mut out[7..9], state.right_x);
    put_axis(&mut out[9..11], state.right_y);
    out[11] = state.left_trigger;
    out[12] = state.right_trigger;
    out
}

/// Strict: rejects anything `encode` would never produce, so bad phone
/// frames are counted instead of silently fixed.
pub fn decode(bytes: &[u8; REPORT_LEN]) -> Result<PadState, ReportError> {
    let buttons = u16::from_le_bytes([bytes[0], bytes[1]]);
    if buttons & RESERVED_BUTTONS != 0 {
        return Err(ReportError::ReservedButtons(buttons & RESERVED_BUTTONS));
    }
    if bytes[2] & 0xF0 != 0 {
        return Err(ReportError::HatPadding);
    }
    let hat = Hat::from_value(bytes[2]).ok_or(ReportError::BadHat(bytes[2]))?;
    Ok(PadState {
        buttons,
        hat,
        left_x: get_axis(&bytes[3..5])?,
        left_y: get_axis(&bytes[5..7])?,
        right_x: get_axis(&bytes[7..9])?,
        right_y: get_axis(&bytes[9..11])?,
        left_trigger: bytes[11],
        right_trigger: bytes[12],
    })
}

/// The declared logical minimum is -32767; clamp so i16::MIN never goes out.
fn put_axis(dst: &mut [u8], value: i16) {
    dst.copy_from_slice(&value.max(AXIS_MIN).to_le_bytes());
}

fn get_axis(src: &[u8]) -> Result<i16, ReportError> {
    let value = i16::from_le_bytes([src[0], src[1]]);
    if value < AXIS_MIN {
        return Err(ReportError::AxisOutOfRange);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pad::{AXIS_MAX, Button};

    #[test]
    fn neutral_report() {
        assert_eq!(encode(&PadState::NEUTRAL), [0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_pressed() {
        let s = PadState::NEUTRAL.with_button(Button::A, true);
        assert_eq!(encode(&s), [1, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn every_field_lands_little_endian_in_place() {
        let s = PadState {
            buttons: Button::Menu.bit() | Button::R3.bit(),
            hat: Hat::DownLeft,
            left_x: AXIS_MAX,
            left_y: AXIS_MIN,
            right_x: 0x1234,
            right_y: -2,
            left_trigger: 255,
            right_trigger: 7,
        };
        assert_eq!(
            encode(&s),
            [0x00, 0x48, 0x05, 0xFF, 0x7F, 0x01, 0x80, 0x34, 0x12, 0xFE, 0xFF, 0xFF, 0x07]
        );
    }

    #[test]
    fn i16_min_is_clamped() {
        let s = PadState { left_x: i16::MIN, ..PadState::NEUTRAL };
        assert_eq!(&encode(&s)[3..5], &[0x01, 0x80]);
    }

    #[test]
    fn reserved_bits_are_never_sent() {
        let s = PadState { buttons: 0xFFFF, ..PadState::NEUTRAL };
        assert_eq!(&encode(&s)[0..2], &[0xDB, 0x7C]);
    }

    #[test]
    fn decode_round_trips() {
        let s = PadState {
            buttons: Button::X.bit() | Button::LB.bit(),
            hat: Hat::Right,
            left_x: -100,
            left_y: 32767,
            right_x: -32767,
            right_y: 1,
            left_trigger: 0,
            right_trigger: 255,
        };
        assert_eq!(decode(&encode(&s)), Ok(s));
    }

    #[test]
    fn decode_rejects_invalid_values() {
        let mut r = encode(&PadState::NEUTRAL);
        r[1] = 0x80;
        assert_eq!(decode(&r), Err(ReportError::ReservedButtons(0x8000)));

        let mut r = encode(&PadState::NEUTRAL);
        r[2] = 9;
        assert_eq!(decode(&r), Err(ReportError::BadHat(9)));

        let mut r = encode(&PadState::NEUTRAL);
        r[2] = 0x18;
        assert_eq!(decode(&r), Err(ReportError::HatPadding));

        let mut r = encode(&PadState::NEUTRAL);
        r[9..11].copy_from_slice(&i16::MIN.to_le_bytes());
        assert_eq!(decode(&r), Err(ReportError::AxisOutOfRange));
    }
}
