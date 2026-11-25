// The protocol for the browser pad: the same frames as the Rust crate
// board/proto and the app's Kotlin code, all three checked against
// board/proto/testdata/vectors.json. Field names follow that file; tokens
// are BigInt, since a u64 doesn't fit a JS number.

export const VERSION = 1;
export const NO_PIN = 0xffff;
export const MAX_PIN = 9999;
export const REPORT_LEN = 13;
export const AXIS_MAX = 32767;
export const HAT_NEUTRAL = 8;

/** Button to report bit (docs/02-how-it-works.md, "The HID report"). */
export const BUTTON_BIT = { A: 0, B: 1, X: 3, Y: 4, LB: 6, RB: 7, View: 10, Menu: 11, Guide: 12, L3: 13, R3: 14 };
const USED_BITS = Object.values(BUTTON_BIT).reduce((bits, bit) => bits | (1 << bit), 0);

export const ERROR_NAMES = { 1: 'slot taken', 2: 'wrong PIN', 3: 'no such slot', 4: 'bad frame', 5: 'unknown token' };

const TYPES = {
  hello: 0x01, welcome: 0x02, error: 0x03, state: 0x10, heartbeat: 0x11,
  ping: 0x20, pong: 0x21, stats: 0x22, release: 0x30, status_req: 0x40, status: 0x41,
};
const NAMES = Object.fromEntries(Object.entries(TYPES).map(([name, type]) => [type, name]));
/** Payload length after the 4-byte header. STATUS has no fixed length. */
const PAYLOAD_LEN = {
  hello: 11, welcome: 9, error: 1, state: 10 + REPORT_LEN, heartbeat: 10,
  ping: 16, pong: 16, stats: 12, release: 8, status_req: 0,
};

export class ProtoError extends Error {
  constructor(kind, message) {
    super(message);
    this.kind = kind;
  }
}

export function neutral() {
  return { buttons: 0, hat: HAT_NEUTRAL, left_x: 0, left_y: 0, right_x: 0, right_y: 0, left_trigger: 0, right_trigger: 0 };
}

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

function writeReport(view, at, s) {
  view.setUint16(at, s.buttons & USED_BITS, true);
  view.setUint8(at + 2, clamp(s.hat, 0, HAT_NEUTRAL));
  view.setInt16(at + 3, clamp(s.left_x, -AXIS_MAX, AXIS_MAX), true);
  view.setInt16(at + 5, clamp(s.left_y, -AXIS_MAX, AXIS_MAX), true);
  view.setInt16(at + 7, clamp(s.right_x, -AXIS_MAX, AXIS_MAX), true);
  view.setInt16(at + 9, clamp(s.right_y, -AXIS_MAX, AXIS_MAX), true);
  view.setUint8(at + 11, clamp(s.left_trigger, 0, 255));
  view.setUint8(at + 12, clamp(s.right_trigger, 0, 255));
}

/** Strict, like the board: rejects anything writeReport never produces. */
function readReport(view, at) {
  const buttons = view.getUint16(at, true);
  if (buttons & ~USED_BITS & 0xffff) throw new ProtoError('bad_state', 'reserved button bits');
  const hat = view.getUint8(at + 2);
  if (hat > HAT_NEUTRAL) throw new ProtoError('bad_state', `hat ${hat}`);
  const axis = (offset) => {
    const v = view.getInt16(at + offset, true);
    if (v < -AXIS_MAX) throw new ProtoError('bad_state', 'axis -32768');
    return v;
  };
  return {
    buttons, hat,
    left_x: axis(3), left_y: axis(5), right_x: axis(7), right_y: axis(9),
    left_trigger: view.getUint8(at + 11), right_trigger: view.getUint8(at + 12),
  };
}

export function encodeReport(state) {
  const bytes = new Uint8Array(REPORT_LEN);
  writeReport(new DataView(bytes.buffer), 0, state);
  return bytes;
}

export function decodeReport(bytes) {
  return readReport(new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength), 0);
}

/** A frame object (`{ type: 'state', token, seq, state }`...) to its bytes. */
export function encode(frame) {
  const type = TYPES[frame.type];
  if (type === undefined) throw new ProtoError('unknown_type', `type ${frame.type}`);
  const text = frame.type === 'status' ? new TextEncoder().encode(frame.text) : null;
  const bytes = new Uint8Array(4 + (text ? text.length : PAYLOAD_LEN[frame.type]));
  const v = new DataView(bytes.buffer);
  bytes.set([0x53, 0x53, VERSION, type]);
  switch (frame.type) {
    case 'hello':
      v.setUint8(4, frame.slot);
      v.setBigUint64(5, frame.token, true);
      v.setUint16(13, frame.pin ?? NO_PIN, true);
      break;
    case 'welcome':
      v.setUint8(4, frame.slot);
      v.setBigUint64(5, frame.token, true);
      break;
    case 'error':
      v.setUint8(4, frame.code);
      break;
    case 'state':
      v.setBigUint64(4, frame.token, true);
      v.setUint16(12, frame.seq, true);
      writeReport(v, 14, frame.state);
      break;
    case 'heartbeat':
      v.setBigUint64(4, frame.token, true);
      v.setUint16(12, frame.seq, true);
      break;
    case 'ping':
    case 'pong':
      v.setBigUint64(4, frame.token, true);
      v.setUint32(12, frame.id, true);
      v.setUint32(16, frame.phone_time_ms, true);
      break;
    case 'stats':
      v.setBigUint64(4, frame.token, true);
      v.setUint16(12, frame.rtt_p50_ms, true);
      v.setUint16(14, frame.rtt_p95_ms, true);
      break;
    case 'release':
      v.setBigUint64(4, frame.token, true);
      break;
    case 'status':
      bytes.set(text, 4);
      break;
  }
  return bytes;
}

/** Bytes to a frame object; throws ProtoError whose `kind` matches vectors.json. */
export function decode(input) {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  if (bytes.length < 4) throw new ProtoError('too_short', 'frame shorter than header');
  if (bytes[0] !== 0x53 || bytes[1] !== 0x53) throw new ProtoError('bad_magic', 'bad magic');
  if (bytes[2] !== VERSION) throw new ProtoError('bad_version', `version ${bytes[2]}`);
  const type = NAMES[bytes[3]];
  if (type === undefined) throw new ProtoError('unknown_type', `type ${bytes[3]}`);
  if (type === 'status') {
    try {
      return { type, text: new TextDecoder('utf-8', { fatal: true }).decode(bytes.subarray(4)) };
    } catch {
      throw new ProtoError('bad_status_text', 'STATUS is not UTF-8');
    }
  }
  const length = bytes.length - 4;
  if (length !== PAYLOAD_LEN[type]) throw new ProtoError('bad_length', `${type}: ${length} != ${PAYLOAD_LEN[type]}`);
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const token = () => v.getBigUint64(4, true);
  switch (type) {
    case 'hello': {
      const pin = v.getUint16(13, true);
      if (pin !== NO_PIN && pin > MAX_PIN) throw new ProtoError('bad_pin', `PIN ${pin}`);
      return { type, slot: v.getUint8(4), token: v.getBigUint64(5, true), pin: pin === NO_PIN ? null : pin };
    }
    case 'welcome':
      return { type, slot: v.getUint8(4), token: v.getBigUint64(5, true) };
    case 'error': {
      const code = v.getUint8(4);
      if (!(code in ERROR_NAMES)) throw new ProtoError('bad_error_code', `code ${code}`);
      return { type, code };
    }
    case 'state':
      return { type, token: token(), seq: v.getUint16(12, true), state: readReport(v, 14) };
    case 'heartbeat':
      return { type, token: token(), seq: v.getUint16(12, true) };
    case 'ping':
    case 'pong':
      return { type, token: token(), id: v.getUint32(12, true), phone_time_ms: v.getUint32(16, true) };
    case 'stats':
      return { type, token: token(), rtt_p50_ms: v.getUint16(12, true), rtt_p95_ms: v.getUint16(14, true) };
    case 'release':
      return { type, token: token() };
    default:
      return { type };
  }
}
