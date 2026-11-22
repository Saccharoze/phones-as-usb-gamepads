//! UDP frames between phones and the board. Every frame starts with the
//! 4-byte header `S S <version> <type>`; integers are little-endian.

use crate::pad::PadState;
use crate::report::{self, REPORT_LEN, ReportError};

pub const MAGIC: [u8; 2] = *b"SS";
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 4;
pub const DEFAULT_PORT: u16 = 40400;
pub const SERVICE_TYPE: &str = "_sendingstones._udp.local.";
/// Wire value meaning "no PIN given".
pub const NO_PIN: u16 = 0xFFFF;
pub const MAX_PIN: u16 = 9999;

mod kind {
    pub const HELLO: u8 = 0x01;
    pub const WELCOME: u8 = 0x02;
    pub const ERROR: u8 = 0x03;
    pub const STATE: u8 = 0x10;
    pub const HEARTBEAT: u8 = 0x11;
    pub const PING: u8 = 0x20;
    pub const PONG: u8 = 0x21;
    pub const STATS: u8 = 0x22;
    pub const RELEASE: u8 = 0x30;
    pub const STATUS_REQ: u8 = 0x40;
    pub const STATUS: u8 = 0x41;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    SlotTaken = 1,
    BadPin = 2,
    NoSuchSlot = 3,
    BadFrame = 4,
    UnknownToken = 5,
}

impl ErrorCode {
    fn from_u8(v: u8) -> Option<ErrorCode> {
        Some(match v {
            1 => ErrorCode::SlotTaken,
            2 => ErrorCode::BadPin,
            3 => ErrorCode::NoSuchSlot,
            4 => ErrorCode::BadFrame,
            5 => ErrorCode::UnknownToken,
            _ => return None,
        })
    }
}

/// PING and PONG carry the same payload; the board echoes it unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ping {
    pub token: u64,
    pub id: u32,
    pub phone_time_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// `token` 0 asks for a new session; `pin` None is sent as 0xFFFF.
    Hello { slot: u8, token: u64, pin: Option<u16> },
    Welcome { slot: u8, token: u64 },
    Error(ErrorCode),
    State { token: u64, seq: u16, state: PadState },
    Heartbeat { token: u64, seq: u16 },
    Ping(Ping),
    Pong(Ping),
    Stats { token: u64, rtt_p50_ms: u16, rtt_p95_ms: u16 },
    Release { token: u64 },
    StatusReq,
    Status(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("frame shorter than the header")]
    TooShort,
    #[error("bad magic")]
    BadMagic,
    #[error("unsupported protocol version {0}")]
    BadVersion(u8),
    #[error("unknown frame type {0:#04x}")]
    UnknownType(u8),
    #[error("type {kind:#04x}: expected {expected} bytes, got {got}")]
    BadLength { kind: u8, expected: usize, got: usize },
    #[error("PIN {0} is not 4 digits")]
    BadPin(u16),
    #[error("unknown error code {0}")]
    BadErrorCode(u8),
    #[error("bad pad state: {0}")]
    BadState(#[from] ReportError),
    #[error("STATUS is not UTF-8")]
    BadStatusText,
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new(self.kind());
        match self {
            Frame::Hello { slot, token, pin } => {
                w.u8(*slot).u64(*token).u16(pin.unwrap_or(NO_PIN));
            }
            Frame::Welcome { slot, token } => {
                w.u8(*slot).u64(*token);
            }
            Frame::Error(code) => {
                w.u8(*code as u8);
            }
            Frame::State { token, seq, state } => {
                w.u64(*token).u16(*seq).bytes(&report::encode(state));
            }
            Frame::Heartbeat { token, seq } => {
                w.u64(*token).u16(*seq);
            }
            Frame::Ping(p) | Frame::Pong(p) => {
                w.u64(p.token).u32(p.id).u32(p.phone_time_ms);
            }
            Frame::Stats { token, rtt_p50_ms, rtt_p95_ms } => {
                w.u64(*token).u16(*rtt_p50_ms).u16(*rtt_p95_ms);
            }
            Frame::Release { token } => {
                w.u64(*token);
            }
            Frame::StatusReq => {}
            Frame::Status(json) => {
                w.bytes(json.as_bytes());
            }
        }
        w.finish()
    }

    pub fn decode(buf: &[u8]) -> Result<Frame, FrameError> {
        if buf.len() < HEADER_LEN {
            return Err(FrameError::TooShort);
        }
        if buf[0..2] != MAGIC {
            return Err(FrameError::BadMagic);
        }
        if buf[2] != VERSION {
            return Err(FrameError::BadVersion(buf[2]));
        }
        let kind = buf[3];
        let payload = &buf[HEADER_LEN..];
        if kind == kind::STATUS {
            let text = std::str::from_utf8(payload).map_err(|_| FrameError::BadStatusText)?;
            return Ok(Frame::Status(text.to_owned()));
        }
        let expected = payload_len(kind).ok_or(FrameError::UnknownType(kind))?;
        if payload.len() != expected {
            return Err(FrameError::BadLength { kind, expected, got: payload.len() });
        }
        decode_fixed(kind, &mut Reader(payload))
    }

    pub fn kind(&self) -> u8 {
        match self {
            Frame::Hello { .. } => kind::HELLO,
            Frame::Welcome { .. } => kind::WELCOME,
            Frame::Error(_) => kind::ERROR,
            Frame::State { .. } => kind::STATE,
            Frame::Heartbeat { .. } => kind::HEARTBEAT,
            Frame::Ping(_) => kind::PING,
            Frame::Pong(_) => kind::PONG,
            Frame::Stats { .. } => kind::STATS,
            Frame::Release { .. } => kind::RELEASE,
            Frame::StatusReq => kind::STATUS_REQ,
            Frame::Status(_) => kind::STATUS,
        }
    }
}

fn payload_len(kind: u8) -> Option<usize> {
    Some(match kind {
        kind::HELLO => 1 + 8 + 2,
        kind::WELCOME => 1 + 8,
        kind::ERROR => 1,
        kind::STATE => 8 + 2 + REPORT_LEN,
        kind::HEARTBEAT => 8 + 2,
        kind::PING | kind::PONG => 8 + 4 + 4,
        kind::STATS => 8 + 2 + 2,
        kind::RELEASE => 8,
        kind::STATUS_REQ => 0,
        _ => return None,
    })
}

fn decode_fixed(kind: u8, r: &mut Reader) -> Result<Frame, FrameError> {
    Ok(match kind {
        kind::HELLO => {
            let (slot, token, pin) = (r.u8(), r.u64(), r.u16());
            Frame::Hello { slot, token, pin: decode_pin(pin)? }
        }
        kind::WELCOME => Frame::Welcome { slot: r.u8(), token: r.u64() },
        kind::ERROR => {
            let code = r.u8();
            Frame::Error(ErrorCode::from_u8(code).ok_or(FrameError::BadErrorCode(code))?)
        }
        kind::STATE => {
            let (token, seq) = (r.u64(), r.u16());
            let state = report::decode(&r.array())?;
            Frame::State { token, seq, state }
        }
        kind::HEARTBEAT => Frame::Heartbeat { token: r.u64(), seq: r.u16() },
        kind::PING => Frame::Ping(read_ping(r)),
        kind::PONG => Frame::Pong(read_ping(r)),
        kind::STATS => Frame::Stats { token: r.u64(), rtt_p50_ms: r.u16(), rtt_p95_ms: r.u16() },
        kind::RELEASE => Frame::Release { token: r.u64() },
        kind::STATUS_REQ => Frame::StatusReq,
        other => return Err(FrameError::UnknownType(other)),
    })
}

fn decode_pin(raw: u16) -> Result<Option<u16>, FrameError> {
    match raw {
        NO_PIN => Ok(None),
        pin if pin <= MAX_PIN => Ok(Some(pin)),
        bad => Err(FrameError::BadPin(bad)),
    }
}

fn read_ping(r: &mut Reader) -> Ping {
    Ping { token: r.u64(), id: r.u32(), phone_time_ms: r.u32() }
}

/// True when `seq` is newer than `last`, treating u16 as a ring: anything up
/// to half the ring ahead counts as newer, so the counter can wrap freely.
pub fn seq_is_newer(seq: u16, last: u16) -> bool {
    let ahead = seq.wrapping_sub(last);
    ahead != 0 && ahead < 0x8000
}

struct Writer(Vec<u8>);

impl Writer {
    fn new(kind: u8) -> Writer {
        Writer(vec![MAGIC[0], MAGIC[1], VERSION, kind])
    }
    fn u8(&mut self, v: u8) -> &mut Writer {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Writer {
        self.bytes(&v.to_le_bytes())
    }
    fn u32(&mut self, v: u32) -> &mut Writer {
        self.bytes(&v.to_le_bytes())
    }
    fn u64(&mut self, v: u64) -> &mut Writer {
        self.bytes(&v.to_le_bytes())
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Writer {
        self.0.extend_from_slice(b);
        self
    }
    fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.0)
    }
}

/// Lengths are checked before reading, so these never run past the end.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn array<const N: usize>(&mut self) -> [u8; N] {
        let (head, rest) = self.0.split_at(N);
        self.0 = rest;
        head.try_into().expect("split_at returned N bytes")
    }
    fn u8(&mut self) -> u8 {
        self.array::<1>()[0]
    }
    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.array())
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.array())
    }
    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.array())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pad::Button;

    fn round_trip(frame: Frame) {
        assert_eq!(Frame::decode(&frame.encode()), Ok(frame));
    }

    #[test]
    fn every_type_round_trips() {
        let p = Ping { token: 7, id: 9, phone_time_ms: 123_456 };
        let state = PadState::NEUTRAL.with_button(Button::Y, true);
        for frame in [
            Frame::Hello { slot: 1, token: 0, pin: None },
            Frame::Hello { slot: 2, token: u64::MAX, pin: Some(0) },
            Frame::Welcome { slot: 2, token: 42 },
            Frame::Error(ErrorCode::UnknownToken),
            Frame::State { token: 1, seq: 65535, state },
            Frame::Heartbeat { token: 1, seq: 3 },
            Frame::Ping(p),
            Frame::Pong(p),
            Frame::Stats { token: 1, rtt_p50_ms: 12, rtt_p95_ms: 40 },
            Frame::Release { token: 5 },
            Frame::StatusReq,
            Frame::Status("{\"pads\":2}".into()),
        ] {
            round_trip(frame);
        }
    }

    #[test]
    fn rejects_bad_headers() {
        assert_eq!(Frame::decode(b"SS\x01"), Err(FrameError::TooShort));
        assert_eq!(Frame::decode(b"XS\x01\x40"), Err(FrameError::BadMagic));
        assert_eq!(Frame::decode(b"SS\x02\x40"), Err(FrameError::BadVersion(2)));
        assert_eq!(Frame::decode(b"SS\x01\x99"), Err(FrameError::UnknownType(0x99)));
    }

    #[test]
    fn rejects_wrong_lengths() {
        let mut bytes = Frame::Release { token: 1 }.encode();
        bytes.push(0);
        assert_eq!(
            Frame::decode(&bytes),
            Err(FrameError::BadLength { kind: 0x30, expected: 8, got: 9 })
        );
        assert!(matches!(Frame::decode(b"SS\x01\x10"), Err(FrameError::BadLength { .. })));
    }

    #[test]
    fn rejects_bad_values() {
        let mut hello = Frame::Hello { slot: 1, token: 0, pin: Some(1) }.encode();
        hello[13..15].copy_from_slice(&10_000u16.to_le_bytes());
        assert_eq!(Frame::decode(&hello), Err(FrameError::BadPin(10_000)));

        let mut state = Frame::State { token: 1, seq: 1, state: PadState::NEUTRAL }.encode();
        state[16] = 9;
        assert!(matches!(Frame::decode(&state), Err(FrameError::BadState(_))));

        assert_eq!(Frame::decode(b"SS\x01\x03\x09"), Err(FrameError::BadErrorCode(9)));
    }

    #[test]
    fn sequence_order_survives_wrap() {
        assert!(seq_is_newer(1, 0));
        assert!(seq_is_newer(0, 65535));
        assert!(seq_is_newer(10, 65530));
        assert!(!seq_is_newer(5, 5));
        assert!(!seq_is_newer(4, 5));
        assert!(!seq_is_newer(65530, 10));
    }
}
