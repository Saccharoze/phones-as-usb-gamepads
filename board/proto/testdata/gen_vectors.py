"""Independent oracle for proto/testdata/vectors.json: written from the tables in docs/02-how-it-works.md, not from the Rust code."""
import json, struct, sys

def report(s):
    return struct.pack("<HBhhhhBB", s["buttons"], s["hat"], s["left_x"], s["left_y"],
                       s["right_x"], s["right_y"], s["left_trigger"], s["right_trigger"])

def state(buttons=0, hat=8, lx=0, ly=0, rx=0, ry=0, lt=0, rt=0):
    return dict(buttons=buttons, hat=hat, left_x=lx, left_y=ly, right_x=rx, right_y=ry,
                left_trigger=lt, right_trigger=rt)

def hdr(t): return b"SS\x01" + bytes([t])
def tok(t): return format(t, "016x")

reports = [
    ("neutral", state()),
    ("a_pressed", state(buttons=1)),
    ("all_v1_buttons", state(buttons=0x7CDB)),
    ("guide", state(buttons=0x1000)),
    ("dpad_up", state(hat=0)),
    ("dpad_up_left", state(hat=7)),
    ("left_stick_full_up_right", state(lx=32767, ly=-32767)),
    ("right_stick_full_down_left", state(rx=-32767, ry=32767)),
    ("triggers_full", state(lt=255, rt=255)),
    ("mixed", state(buttons=0x4800, hat=5, lx=32767, ly=-32767, rx=0x1234, ry=-2, lt=255, rt=7)),
]

A_RB = state(buttons=0x0081, hat=2, lx=-1000, ly=2000, rt=255)
T = 0x0123456789ABCDEF

def f(name, frame, raw): return dict(name=name, frame=frame, hex=raw.hex())

frames = [
    f("hello_new_no_pin", dict(type="hello", slot=1, token=tok(0), pin=None),
      hdr(0x01) + struct.pack("<BQH", 1, 0, 0xFFFF)),
    f("hello_resume_with_pin", dict(type="hello", slot=2, token=tok(T), pin=1234),
      hdr(0x01) + struct.pack("<BQH", 2, T, 1234)),
    f("hello_pin_zero", dict(type="hello", slot=1, token=tok(0), pin=0),
      hdr(0x01) + struct.pack("<BQH", 1, 0, 0)),
    f("welcome", dict(type="welcome", slot=2, token=tok(T)), hdr(0x02) + struct.pack("<BQ", 2, T)),
] + [
    f(f"error_{n}", dict(type="error", code=c), hdr(0x03) + bytes([c]))
    for n, c in [("slot_taken", 1), ("bad_pin", 2), ("no_such_slot", 3), ("bad_frame", 4), ("unknown_token", 5)]
] + [
    f("state_neutral", dict(type="state", token=tok(T), seq=0, state=state()),
      hdr(0x10) + struct.pack("<QH", T, 0) + report(state())),
    f("state_a_rb_dpad_right", dict(type="state", token=tok(T), seq=65535, state=A_RB),
      hdr(0x10) + struct.pack("<QH", T, 65535) + report(A_RB)),
    f("heartbeat", dict(type="heartbeat", token=tok(T), seq=513), hdr(0x11) + struct.pack("<QH", T, 513)),
    f("ping", dict(type="ping", token=tok(T), id=7, phone_time_ms=4000000000),
      hdr(0x20) + struct.pack("<QII", T, 7, 4000000000)),
    f("pong", dict(type="pong", token=tok(T), id=7, phone_time_ms=4000000000),
      hdr(0x21) + struct.pack("<QII", T, 7, 4000000000)),
    f("stats", dict(type="stats", token=tok(T), rtt_p50_ms=12, rtt_p95_ms=48),
      hdr(0x22) + struct.pack("<QHH", T, 12, 48)),
    f("release", dict(type="release", token=tok(T)), hdr(0x30) + struct.pack("<Q", T)),
    f("status_req", dict(type="status_req"), hdr(0x40)),
    f("status", dict(type="status", text='{"pads":2}'), hdr(0x41) + b'{"pads":2}'),
]

def bad(name, raw, error): return dict(name=name, hex=raw.hex(), error=error)
good_state = hdr(0x10) + struct.pack("<QH", T, 1) + report(state())
invalid = [
    bad("too_short", b"SS\x01", "too_short"),
    bad("bad_magic", b"SX\x01\x40", "bad_magic"),
    bad("bad_version", b"SS\x02\x40", "bad_version"),
    bad("unknown_type", b"SS\x01\x99", "unknown_type"),
    bad("state_too_short", good_state[:-1], "bad_length"),
    bad("release_too_long", hdr(0x30) + struct.pack("<Q", T) + b"\x00", "bad_length"),
    bad("hello_pin_10000", hdr(0x01) + struct.pack("<BQH", 1, 0, 10000), "bad_pin"),
    bad("error_code_0", hdr(0x03) + b"\x00", "bad_error_code"),
    bad("state_hat_9", good_state[:16] + b"\x09" + good_state[17:], "bad_state"),
    bad("state_reserved_button", good_state[:15] + b"\x80" + good_state[16:], "bad_state"),
    bad("state_axis_min", good_state[:17] + struct.pack("<h", -32768) + good_state[19:], "bad_state"),
    bad("status_not_utf8", hdr(0x41) + b"\xff\xfe", "bad_status_text"),
]

out = dict(
    about="Golden vectors for the Sending Stones protocol. Tokens are 16 hex digits (u64 does not fit a JSON double). Hex is lowercase, no separators. Checked by proto (Rust), the Android tests (Kotlin) and the browser pad (board/daemon/web/proto.test.mjs).",
    version=1,
    reports=[dict(name=n, state=s, hex=report(s).hex()) for n, s in reports],
    frames=frames,
    invalid_frames=invalid,
)
json.dump(out, open(sys.argv[1], "w"), indent=2)
print(len(reports), "reports,", len(frames), "frames,", len(invalid), "invalid")
