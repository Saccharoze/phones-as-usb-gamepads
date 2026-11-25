// The browser pad: pick a player, then play. Same layout, stick maths and
// timing as the Android app (Layout.kt, InputModel.kt, BoardClient.kt), with
// the app's default settings.

import { AXIS_MAX, BUTTON_BIT, ERROR_NAMES, HAT_NEUTRAL, decode, encode, neutral } from './proto.mjs';

// Layout.kt: a control sits at an anchor (fractions of width and height) plus
// an offset and a radius in units of screen height.
const FACE = 0.075;
const SHOULDER = 0.07;
const SMALL = 0.055;
const spot = (ax, ay, dx, dy, r) => ({ ax, ay, dx, dy, r });
const face = (dx, dy) => spot(1, 0.72, -0.30 + dx, dy, FACE);
const CONTROLS = [
  { kind: 'trigger', left: true, label: 'LT', spot: spot(0, 0.11, 0.12, 0, SHOULDER) },
  { kind: 'press', button: 'LB', spot: spot(0, 0.11, 0.30, 0, SHOULDER) },
  { kind: 'trigger', left: false, label: 'RT', spot: spot(1, 0.11, -0.12, 0, SHOULDER) },
  { kind: 'press', button: 'RB', spot: spot(1, 0.11, -0.30, 0, SHOULDER) },
  { kind: 'press', button: 'View', spot: spot(0.5, 0.13, -0.16, 0, SMALL) },
  { kind: 'press', button: 'Menu', spot: spot(0.5, 0.13, 0.16, 0, SMALL) },
  { kind: 'press', button: 'L3', spot: spot(0.5, 0.42, -0.20, 0, SMALL) },
  { kind: 'press', button: 'R3', spot: spot(0.5, 0.42, 0.20, 0, SMALL) },
  { kind: 'dpad', spot: spot(0.5, 0.78, -0.42, 0, 0.15) },
  { kind: 'press', button: 'A', spot: face(0, 0.15) },
  { kind: 'press', button: 'B', spot: face(0.15, 0) },
  { kind: 'press', button: 'X', spot: face(-0.15, 0) },
  { kind: 'press', button: 'Y', spot: face(0, -0.15) },
];
const DPAD = CONTROLS.find((c) => c.kind === 'dpad');
const HIT_SLOP = 1.25;
const STICK_ZONE_TOP = 0.24;
const STICK_TRAVEL = 0.16;
const DPAD_DEAD = 0.25;
const DEAD_ZONE = 0.10;

// BoardClient.kt timing.
const KEEPALIVE_MS = 100;
const PING_EVERY_MS = 1000;
const STATS_EVERY_MS = 5000;
const STATUS_EVERY_MS = 1500;
const CLAIM_TIMEOUT_MS = 2500;
const RECONNECT_MS = 1000;

const BACKGROUND = '#121214';
const IDLE = '#3a3a40';
const LABEL = '#e8e8ec';
const PLAYER_COLORS = ['#4f8ef7', '#f2784b', '#3dbe7a', '#b57cf2'];
const STORE_KEY = 'sending-stones-session';

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
const accent = (slot) => PLAYER_COLORS[(slot - 1) % PLAYER_COLORS.length];

// ---- Touch to pad state (InputModel.kt) ----

let W = 1;
let H = 1;
/** pointerId -> what that finger holds, from touch-down until it lifts. */
const bindings = new Map();

const centre = (s) => ({ x: s.ax * W + s.dx * H, y: s.ay * H + s.dy * H });

function stickAxes(dx, dy) {
  const length = Math.hypot(dx, dy);
  const travel = STICK_TRAVEL * H;
  const magnitude = length / travel;
  if (magnitude <= DEAD_ZONE || travel <= 0) return [0, 0];
  const unit = clamp((magnitude - DEAD_ZONE) / (1 - DEAD_ZONE), 0, 1) / length;
  const axis = (v) => clamp(Math.round(v * AXIS_MAX), -AXIS_MAX, AXIS_MAX);
  return [axis(dx * unit), axis(dy * unit)];
}

/** Offset from the D-pad centre to a hat value: 0 up, clockwise, 8 none. */
function dpadHat(x, y) {
  const c = centre(DPAD.spot);
  const dx = x - c.x;
  const dy = y - c.y;
  if (Math.hypot(dx, dy) < DPAD.spot.r * H * DPAD_DEAD) return HAT_NEUTRAL;
  const degrees = ((Math.atan2(dx, -dy) * 180) / Math.PI + 360) % 360;
  return Math.floor((degrees + 22.5) / 45) % 8;
}

/** Nearest control whose enlarged circle holds the touch. */
function hit(x, y) {
  let best = null;
  let bestDistance = Infinity;
  for (const control of CONTROLS) {
    const c = centre(control.spot);
    const d = Math.hypot(x - c.x, y - c.y) / (control.spot.r * H);
    if (d <= HIT_SLOP && d < bestDistance) {
      best = control;
      bestDistance = d;
    }
  }
  return best;
}

function bind(x, y) {
  const control = hit(x, y);
  if (control?.kind === 'press') return { kind: 'press', button: control.button };
  if (control?.kind === 'trigger') return { kind: 'trigger', left: control.left };
  if (control?.kind === 'dpad') return { kind: 'dpad', hat: dpadHat(x, y) };
  if (y < STICK_ZONE_TOP * H) return null;
  const left = x < W / 2;
  // One finger per stick: a second thumb on the same half is ignored.
  for (const b of bindings.values()) if (b.kind === 'stick' && b.left === left) return null;
  return { kind: 'stick', left, ox: x, oy: y, x, y };
}

function padState() {
  const s = neutral();
  for (const b of bindings.values()) {
    if (b.kind === 'press') s.buttons |= 1 << BUTTON_BIT[b.button];
    else if (b.kind === 'trigger') s[b.left ? 'left_trigger' : 'right_trigger'] = 255;
    else if (b.kind === 'dpad' && b.hat !== HAT_NEUTRAL) s.hat = b.hat;
    else if (b.kind === 'stick') {
      const [ax, ay] = stickAxes(b.x - b.ox, b.y - b.oy);
      if (b.left) [s.left_x, s.left_y] = [ax, ay];
      else [s.right_x, s.right_y] = [ax, ay];
    }
  }
  return s;
}

// ---- Session with the board ----

const session = {
  slot: 0,
  pin: null,
  token: 0n,
  seq: 0,
  playing: false,
  /** connecting | connected | paused | no reply */
  link: 'connecting',
  rtt: [],
};
let ws = null;
let sentState = null;
let sendQueued = false;
const timers = {};

function loadSaved() {
  try {
    const saved = JSON.parse(localStorage.getItem(STORE_KEY));
    return saved && { slot: saved.slot, token: BigInt(`0x${saved.token}`) };
  } catch {
    return null;
  }
}

function save(slot, token) {
  try {
    if (token === 0n) localStorage.removeItem(STORE_KEY);
    else localStorage.setItem(STORE_KEY, JSON.stringify({ slot, token: token.toString(16) }));
  } catch {
    // No storage (private window): a reload then asks for a new slot.
  }
}

function connect() {
  if (ws) return;
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
  ws = new WebSocket(`${scheme}//${location.host}/ws`);
  ws.binaryType = 'arraybuffer';
  ws.onopen = () => (session.playing ? claim() : askStatus());
  ws.onmessage = (event) => {
    let frame;
    try {
      frame = decode(event.data);
    } catch {
      return;
    }
    onFrame(frame);
  };
  ws.onclose = () => {
    ws = null;
    stopTimers();
    if (document.hidden) return;
    if (session.playing) setLink('connecting');
    else showMessage('No answer from the board. Is stonesd running?');
    timers.reconnect = setTimeout(connect, RECONNECT_MS);
  };
}

function send(frame) {
  if (ws?.readyState === WebSocket.OPEN) ws.send(encode(frame));
}

function stopTimers() {
  for (const key of Object.keys(timers)) {
    clearTimeout(timers[key]);
    clearInterval(timers[key]);
    delete timers[key];
  }
}

function askStatus() {
  send({ type: 'status_req' });
  timers.status = setInterval(() => send({ type: 'status_req' }), STATUS_EVERY_MS);
}

/** HELLO with the saved token (resume) or 0 (new claim). */
function claim() {
  clearTimeout(timers.claim);
  setLink('connecting');
  send({ type: 'hello', slot: session.slot, token: session.token, pin: session.pin });
  timers.claim = setTimeout(() => setLink('no reply'), CLAIM_TIMEOUT_MS);
}

function onFrame(frame) {
  switch (frame.type) {
    case 'status':
      if (!session.playing) showStatus(JSON.parse(frame.text));
      break;
    case 'welcome':
      if (!session.playing || frame.slot !== session.slot) break;
      clearTimeout(timers.claim);
      session.token = frame.token;
      session.seq = 0;
      save(session.slot, session.token);
      setLink('connected');
      startSending();
      break;
    case 'error':
      if (!session.playing) break;
      if (frame.code === 5 && session.token !== 0n && session.link === 'connected') {
        // The board restarted, or another phone took the slot over with the PIN.
        stopTimers();
        session.token = 0n;
        save(session.slot, 0n);
        claim();
      } else {
        save(session.slot, 0n);
        leave(`Player ${session.slot}: ${ERROR_NAMES[frame.code]}`);
      }
      break;
    case 'pong': {
      const rtt = (nowMs() - frame.phone_time_ms) >>> 0;
      if (rtt > 60000) break;
      session.rtt.push(rtt);
      if (session.rtt.length > 64) session.rtt.shift();
      draw();
      break;
    }
  }
}

const nowMs = () => Math.floor(performance.now()) >>> 0;

/** Nearest rank, as the app's RttWindow. */
function percentile(pct) {
  if (session.rtt.length === 0) return null;
  const sorted = [...session.rtt].sort((a, b) => a - b);
  return sorted[Math.max(1, Math.ceil((pct * sorted.length) / 100)) - 1];
}

function startSending() {
  let pingId = 0;
  sentState = null;
  sendState();
  timers.ping = setInterval(() => {
    send({ type: 'ping', token: session.token, id: pingId, phone_time_ms: nowMs() });
    pingId = (pingId + 1) >>> 0;
  }, PING_EVERY_MS);
  timers.stats = setInterval(() => {
    send({ type: 'stats', token: session.token, rtt_p50_ms: percentile(50) ?? 0, rtt_p95_ms: percentile(95) ?? 0 });
  }, STATS_EVERY_MS);
}

/** The whole state on every change and every 100 ms: a lost one is corrected by the next. */
function sendState() {
  clearTimeout(timers.keepalive);
  if (session.link !== 'connected') return;
  const state = padState();
  session.seq = (session.seq + 1) & 0xffff;
  send({ type: 'state', token: session.token, seq: session.seq, state });
  sentState = JSON.stringify(state);
  timers.keepalive = setTimeout(sendState, KEEPALIVE_MS);
}

/** At most one STATE per display frame. */
function inputChanged() {
  draw();
  if (sendQueued || session.link !== 'connected') return;
  sendQueued = true;
  requestAnimationFrame(() => {
    sendQueued = false;
    if (JSON.stringify(padState()) !== sentState) sendState();
  });
}

function setLink(link) {
  session.link = link;
  draw();
}

// ---- Screens ----

const $ = (id) => document.getElementById(id);
const picker = $('picker');
const canvas = $('pad');
const ctx = canvas.getContext('2d');
const leaveButton = $('leave');
const pinInput = $('pin');
let lastStatus = null;
/** Why the last claim failed; stays on the picker until the next pick. */
let notice = '';

function showMessage(text) {
  $('message').textContent = text;
}

function showStatus(status) {
  lastStatus = status;
  showMessage(notice);
  $('pin-row').hidden = !status.pin_required;
  const saved = loadSaved();
  const pin = pinInput.value;
  const list = $('slots');
  list.replaceChildren();
  for (const s of status.slots) {
    const free = s.state === 'free';
    // This browser's own slot from before a reload: resuming needs no PIN.
    const mine = !free && saved?.slot === s.slot;
    const button = document.createElement('button');
    button.className = 'slot';
    button.style.background = accent(s.slot);
    button.disabled = !(mine || (free && (!status.pin_required || pin.length === 4)));
    const name = document.createElement('b');
    name.textContent = `Player ${s.slot}`;
    const state = document.createElement('span');
    state.textContent = free ? 'free' : mine ? 'yours' : s.state;
    button.append(name, state);
    button.onclick = () => play(s.slot, mine ? saved.token : 0n, pin.length === 4 ? Number(pin) : null);
    list.append(button);
  }
  $('held').hidden = status.slots.some((s) => s.state === 'free');
}

function play(slot, token, pin) {
  notice = '';
  Object.assign(session, { slot, token, pin, playing: true, rtt: [] });
  picker.hidden = true;
  canvas.hidden = false;
  leaveButton.hidden = false;
  history.pushState({ pad: true }, '');
  // Each may be refused (iPhone Safari has no fullscreen for pages, and the
  // screen wake lock needs HTTPS): the pad works without them.
  document.documentElement.requestFullscreen?.({ navigationUI: 'hide' })
    .then(() => screen.orientation?.lock?.('landscape'))
    .catch(() => {});
  navigator.wakeLock?.request('screen').catch(() => {});
  resize();
  stopTimers();
  // Not open yet: the claim goes out when it opens.
  if (ws?.readyState === WebSocket.OPEN) claim();
  else connect();
}

/** Back to the picker: neutral, RELEASE, and the token is forgotten. */
function leave(message = '') {
  if (!session.playing) return;
  bindings.clear();
  if (session.link === 'connected') {
    session.seq = (session.seq + 1) & 0xffff;
    send({ type: 'state', token: session.token, seq: session.seq, state: neutral() });
    send({ type: 'release', token: session.token });
    save(session.slot, 0n);
  }
  Object.assign(session, { playing: false, token: 0n, link: 'connecting' });
  stopTimers();
  canvas.hidden = true;
  leaveButton.hidden = true;
  picker.hidden = false;
  if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
  if (history.state?.pad) history.back();
  notice = message;
  showMessage(message);
  if (ws?.readyState === WebSocket.OPEN) askStatus();
  else connect();
}

/** Page hidden or phone locked: neutral now, then silence; the token is kept. */
function pause() {
  bindings.clear();
  if (session.playing && session.link === 'connected') {
    session.seq = (session.seq + 1) & 0xffff;
    send({ type: 'state', token: session.token, seq: session.seq, state: neutral() });
    session.link = 'paused';
  }
  stopTimers();
  ws?.close();
}

// ---- Drawing (ControllerScreen.kt) ----

function resize() {
  const dpr = window.devicePixelRatio || 1;
  W = window.innerWidth;
  H = window.innerHeight;
  canvas.width = Math.round(W * dpr);
  canvas.height = Math.round(H * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  leaveButton.style.right = `${W / 2 + 0.08 * H}px`;
  draw();
}

function circle(x, y, r, color, stroke = 0) {
  ctx.beginPath();
  ctx.arc(x, y, r, 0, 2 * Math.PI);
  if (stroke) {
    ctx.lineWidth = stroke;
    ctx.strokeStyle = color;
    ctx.stroke();
  } else {
    ctx.fillStyle = color;
    ctx.fill();
  }
}

function label(text, x, y, size, align = 'center') {
  ctx.fillStyle = LABEL;
  ctx.font = `${size}px system-ui, sans-serif`;
  ctx.textAlign = align;
  ctx.textBaseline = 'middle';
  ctx.fillText(text, x, y);
}

function draw() {
  if (canvas.hidden) return;
  ctx.fillStyle = BACKGROUND;
  ctx.fillRect(0, 0, W, H);
  if (H > W) {
    label('Turn the phone sideways', W / 2, H / 2, 18);
    return;
  }
  const color = accent(session.slot);
  const held = [...bindings.values()];
  for (const control of CONTROLS) {
    const c = centre(control.spot);
    const r = control.spot.r * H;
    if (control.kind === 'dpad') {
      const hat = held.find((b) => b.kind === 'dpad')?.hat ?? HAT_NEUTRAL;
      circle(c.x, c.y, r, IDLE, r * 0.08);
      // Up, right, down, left; a diagonal lights its two neighbours.
      for (const [dir, ux, uy] of [[0, 0, -1], [2, 1, 0], [4, 0, 1], [6, -1, 0]]) {
        const on = hat !== HAT_NEUTRAL && [0, 1, 7].includes((hat - dir + 8) % 8);
        circle(c.x + ux * r * 0.6, c.y + uy * r * 0.6, r * 0.28, on ? color : IDLE);
      }
      continue;
    }
    const on = held.some((b) =>
      control.kind === 'press' ? b.kind === 'press' && b.button === control.button : b.kind === 'trigger' && b.left === control.left);
    circle(c.x, c.y, r, on ? color : IDLE);
    label(control.label ?? control.button, c.x, c.y, clamp(r * 0.55, 9, 22));
  }
  const travel = STICK_TRAVEL * H;
  for (const b of held) {
    if (b.kind !== 'stick') continue;
    const dx = b.x - b.ox;
    const dy = b.y - b.oy;
    const k = Math.min(1, travel / (Math.hypot(dx, dy) || 1));
    circle(b.ox, b.oy, travel, IDLE, travel * 0.06);
    circle(b.ox + dx * k, b.oy + dy * k, travel * 0.45, color);
  }
  drawTopBar(color);
}

function drawTopBar(color) {
  const x = W / 2;
  const y = H * 0.05;
  circle(x, y, H * 0.035, color);
  label(`P${session.slot}`, x, y, clamp(H * 0.035 * 0.55, 9, 22));
  const p50 = percentile(50);
  const [dot, text] = {
    connected: ['#3ccb7f', p50 === null ? '…' : `${p50} ms`],
    connecting: ['#e5b83b', 'connecting'],
    paused: ['#e5b83b', 'paused'],
    'no reply': ['#e5484d', 'no reply'],
  }[session.link];
  circle(x + H * 0.08, y, H * 0.012, dot);
  label(text, x + H * 0.11, y, 12, 'left');
}

// ---- Events ----

function position(event) {
  const rect = canvas.getBoundingClientRect();
  return [event.clientX - rect.left, event.clientY - rect.top];
}

// No preventDefault here: `touch-action: none` already stops scrolling and
// zoom, and Firefox drops the other fingers of the same touch event after one.
canvas.addEventListener('pointerdown', (event) => {
  if (H > W) return;
  const [x, y] = position(event);
  const binding = bind(x, y);
  if (!binding) return;
  canvas.setPointerCapture(event.pointerId);
  bindings.set(event.pointerId, binding);
  const tick = binding.kind !== 'stick' && !(binding.kind === 'dpad' && binding.hat === HAT_NEUTRAL);
  if (tick) navigator.vibrate?.(10);
  inputChanged();
});

canvas.addEventListener('pointermove', (event) => {
  const binding = bindings.get(event.pointerId);
  if (!binding) return;
  const [x, y] = position(event);
  if (binding.kind === 'stick') {
    binding.x = x;
    binding.y = y;
  } else if (binding.kind === 'dpad') {
    const hat = dpadHat(x, y);
    if (hat === binding.hat) return;
    if (hat !== HAT_NEUTRAL) navigator.vibrate?.(10);
    binding.hat = hat;
  } else {
    return;
  }
  inputChanged();
});

for (const type of ['pointerup', 'pointercancel', 'lostpointercapture']) {
  canvas.addEventListener(type, (event) => {
    if (bindings.delete(event.pointerId)) inputChanged();
  });
}

canvas.addEventListener('contextmenu', (event) => event.preventDefault());
leaveButton.addEventListener('click', () => leave());
pinInput.addEventListener('input', () => {
  pinInput.value = pinInput.value.replace(/\D/g, '').slice(0, 4);
  if (lastStatus) showStatus(lastStatus);
});
window.addEventListener('resize', resize);
window.addEventListener('popstate', () => leave());
document.addEventListener('visibilitychange', () => {
  if (document.hidden) pause();
  else connect();
});

connect();
