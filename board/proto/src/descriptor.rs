//! HID report descriptor for one pad. `stonesd --write-descriptor` writes it
//! to the file `scripts/setup.sh` feeds to configfs, so this array is the only copy.

#[rustfmt::skip]
pub const PAD: &[u8] = &[
    0x05, 0x01,             // Usage Page (Generic Desktop)
    0x09, 0x05,             // Usage (Game Pad)
    0xA1, 0x01,             // Collection (Application)

    // Bytes 0-1: 16 buttons, bit 0 = A
    0x05, 0x09,             //   Usage Page (Button)
    0x19, 0x01,             //   Usage Minimum (1)
    0x29, 0x10,             //   Usage Maximum (16)
    0x15, 0x00,             //   Logical Minimum (0)
    0x25, 0x01,             //   Logical Maximum (1)
    0x75, 0x01,             //   Report Size (1)
    0x95, 0x10,             //   Report Count (16)
    0x81, 0x02,             //   Input (Data, Var, Abs)

    // Byte 2 low nibble: hat, 8 = null state (released)
    0x05, 0x01,             //   Usage Page (Generic Desktop)
    0x09, 0x39,             //   Usage (Hat Switch)
    0x15, 0x00,             //   Logical Minimum (0)
    0x25, 0x07,             //   Logical Maximum (7)
    0x35, 0x00,             //   Physical Minimum (0)
    0x46, 0x3B, 0x01,       //   Physical Maximum (315)
    0x65, 0x14,             //   Unit (Degrees)
    0x75, 0x04,             //   Report Size (4)
    0x95, 0x01,             //   Report Count (1)
    0x81, 0x42,             //   Input (Data, Var, Abs, Null State)

    // Byte 2 high nibble: padding
    0x65, 0x00,             //   Unit (None)
    0x45, 0x00,             //   Physical Maximum (0): physical = logical from here on
    0x75, 0x04,             //   Report Size (4)
    0x95, 0x01,             //   Report Count (1)
    0x81, 0x03,             //   Input (Const, Var, Abs)

    // Bytes 3-10: X, Y, Rx, Ry as int16
    0x09, 0x30,             //   Usage (X)
    0x09, 0x31,             //   Usage (Y)
    0x09, 0x33,             //   Usage (Rx)
    0x09, 0x34,             //   Usage (Ry)
    0x16, 0x01, 0x80,       //   Logical Minimum (-32767)
    0x26, 0xFF, 0x7F,       //   Logical Maximum (32767)
    0x75, 0x10,             //   Report Size (16)
    0x95, 0x04,             //   Report Count (4)
    0x81, 0x02,             //   Input (Data, Var, Abs)

    // Bytes 11-12: triggers Z and Rz
    0x09, 0x32,             //   Usage (Z)
    0x09, 0x35,             //   Usage (Rz)
    0x15, 0x00,             //   Logical Minimum (0)
    0x26, 0xFF, 0x00,       //   Logical Maximum (255)
    0x75, 0x08,             //   Report Size (8)
    0x95, 0x02,             //   Report Count (2)
    0x81, 0x02,             //   Input (Data, Var, Abs)

    0xC0,                   // End Collection
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::REPORT_LEN;

    /// Minimal short-item walker: (tag and type with size bits cleared, data).
    fn items(desc: &[u8]) -> Vec<(u8, u32)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < desc.len() {
            let prefix = desc[i];
            let len = match prefix & 0x03 {
                3 => 4,
                n => n as usize,
            };
            let data = desc[i + 1..i + 1 + len]
                .iter()
                .rev()
                .fold(0u32, |acc, b| (acc << 8) | *b as u32);
            out.push((prefix & 0xFC, data));
            i += 1 + len;
        }
        out
    }

    #[test]
    fn declares_exactly_one_13_byte_input_report() {
        let (mut size, mut count, mut bits, mut depth) = (0, 0, 0, 0i32);
        for (tag, data) in items(PAD) {
            match tag {
                0x74 => size = data,
                0x94 => count = data,
                0x80 => bits += size * count,
                0xA0 => depth += 1,
                0xC0 => depth -= 1,
                _ => {}
            }
        }
        assert_eq!(bits, REPORT_LEN as u32 * 8);
        assert_eq!(depth, 0, "collections must balance");
    }

    #[test]
    fn has_no_report_id() {
        assert!(items(PAD).iter().all(|(tag, _)| *tag != 0x84));
    }

    #[test]
    fn is_a_generic_desktop_game_pad() {
        assert_eq!(&PAD[..6], &[0x05, 0x01, 0x09, 0x05, 0xA1, 0x01]);
    }
}
