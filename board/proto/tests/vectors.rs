//! Checks proto against `testdata/vectors.json`, the same file the Android
//! tests read. The vectors come from an independent Python implementation
//! (`testdata/gen_vectors.py`), not from this crate.

use proto::frame::{ErrorCode, Frame, FrameError, Ping};
use proto::{Hat, PadState, report};
use serde_json::Value;

fn vectors() -> Value {
    let text = include_str!("../testdata/vectors.json");
    serde_json::from_str(text).expect("vectors.json is valid JSON")
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex digit"))
        .collect()
}

fn int<T: TryFrom<i64>>(v: &Value, key: &str) -> T {
    let n = v[key].as_i64().unwrap_or_else(|| panic!("{key} is an integer"));
    T::try_from(n).unwrap_or_else(|_| panic!("{key} out of range"))
}

fn token(v: &Value) -> u64 {
    u64::from_str_radix(v["token"].as_str().expect("token is a hex string"), 16).expect("token hex")
}

fn state(v: &Value) -> PadState {
    PadState {
        buttons: int(v, "buttons"),
        hat: Hat::from_value(int(v, "hat")).expect("valid hat"),
        left_x: int(v, "left_x"),
        left_y: int(v, "left_y"),
        right_x: int(v, "right_x"),
        right_y: int(v, "right_y"),
        left_trigger: int(v, "left_trigger"),
        right_trigger: int(v, "right_trigger"),
    }
}

fn error_code(n: u8) -> ErrorCode {
    match n {
        1 => ErrorCode::SlotTaken,
        2 => ErrorCode::BadPin,
        3 => ErrorCode::NoSuchSlot,
        4 => ErrorCode::BadFrame,
        5 => ErrorCode::UnknownToken,
        _ => panic!("unknown error code {n}"),
    }
}

fn ping(v: &Value) -> Ping {
    Ping { token: token(v), id: int(v, "id"), phone_time_ms: int(v, "phone_time_ms") }
}

fn frame(v: &Value) -> Frame {
    match v["type"].as_str().expect("type") {
        "hello" => Frame::Hello { slot: int(v, "slot"), token: token(v), pin: v["pin"].as_u64().map(|p| p as u16) },
        "welcome" => Frame::Welcome { slot: int(v, "slot"), token: token(v) },
        "error" => Frame::Error(error_code(int(v, "code"))),
        "state" => Frame::State { token: token(v), seq: int(v, "seq"), state: state(&v["state"]) },
        "heartbeat" => Frame::Heartbeat { token: token(v), seq: int(v, "seq") },
        "ping" => Frame::Ping(ping(v)),
        "pong" => Frame::Pong(ping(v)),
        "stats" => Frame::Stats { token: token(v), rtt_p50_ms: int(v, "rtt_p50_ms"), rtt_p95_ms: int(v, "rtt_p95_ms") },
        "release" => Frame::Release { token: token(v) },
        "status_req" => Frame::StatusReq,
        "status" => Frame::Status(v["text"].as_str().expect("text").to_owned()),
        other => panic!("unknown frame type {other}"),
    }
}

fn error_name(e: &FrameError) -> &'static str {
    match e {
        FrameError::TooShort => "too_short",
        FrameError::BadMagic => "bad_magic",
        FrameError::BadVersion(_) => "bad_version",
        FrameError::UnknownType(_) => "unknown_type",
        FrameError::BadLength { .. } => "bad_length",
        FrameError::BadPin(_) => "bad_pin",
        FrameError::BadErrorCode(_) => "bad_error_code",
        FrameError::BadState(_) => "bad_state",
        FrameError::BadStatusText => "bad_status_text",
    }
}

#[test]
fn reports_match_vectors() {
    let v = vectors();
    for case in v["reports"].as_array().expect("reports") {
        let name = &case["name"];
        let expected = hex(case["hex"].as_str().expect("hex"));
        let s = state(&case["state"]);
        assert_eq!(report::encode(&s).to_vec(), expected, "encode {name}");
        let bytes: [u8; 13] = expected.try_into().expect("13 bytes");
        assert_eq!(report::decode(&bytes), Ok(s), "decode {name}");
    }
}

#[test]
fn frames_match_vectors() {
    let v = vectors();
    for case in v["frames"].as_array().expect("frames") {
        let name = &case["name"];
        let expected = hex(case["hex"].as_str().expect("hex"));
        let f = frame(&case["frame"]);
        assert_eq!(f.encode(), expected, "encode {name}");
        assert_eq!(Frame::decode(&expected), Ok(f), "decode {name}");
    }
}

#[test]
fn invalid_frames_are_rejected_for_the_right_reason() {
    let v = vectors();
    for case in v["invalid_frames"].as_array().expect("invalid_frames") {
        let name = &case["name"];
        let bytes = hex(case["hex"].as_str().expect("hex"));
        let err = Frame::decode(&bytes).expect_err(&format!("{name} must be rejected"));
        assert_eq!(error_name(&err), case["error"].as_str().expect("error"), "{name}");
    }
}

#[test]
fn every_frame_type_has_a_vector() {
    let v = vectors();
    let types: std::collections::HashSet<_> = v["frames"]
        .as_array()
        .expect("frames")
        .iter()
        .map(|c| c["frame"]["type"].as_str().expect("type").to_owned())
        .collect();
    assert_eq!(types.len(), 11, "one or more of the 11 frame types lack a vector: {types:?}");
}
