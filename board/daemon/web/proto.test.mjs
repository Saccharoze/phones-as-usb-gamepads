// proto.mjs against the vectors the Rust and Kotlin code are tested with.
// Run: node --test board/daemon/web/proto.test.mjs

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { decode, decodeReport, encode, encodeReport } from './proto.mjs';

const vectors = JSON.parse(readFileSync(new URL('../../proto/testdata/vectors.json', import.meta.url), 'utf8'));
const hex = (s) => Uint8Array.from(s.match(/../g) ?? [], (byte) => parseInt(byte, 16));
const withBigToken = (frame) => ('token' in frame ? { ...frame, token: BigInt(`0x${frame.token}`) } : frame);

test('reports match the vectors', () => {
  for (const c of vectors.reports) {
    assert.deepEqual(encodeReport(c.state), hex(c.hex), `encode ${c.name}`);
    assert.deepEqual(decodeReport(hex(c.hex)), c.state, `decode ${c.name}`);
  }
});

test('frames match the vectors', () => {
  for (const c of vectors.frames) {
    const frame = withBigToken(c.frame);
    assert.deepEqual(encode(frame), hex(c.hex), `encode ${c.name}`);
    assert.deepEqual(decode(hex(c.hex)), frame, `decode ${c.name}`);
  }
});

test('invalid frames are rejected for the right reason', () => {
  for (const c of vectors.invalid_frames) {
    assert.throws(() => decode(hex(c.hex)), (e) => e.kind === c.error, c.name);
  }
});

test('every frame type has a vector', () => {
  assert.equal(new Set(vectors.frames.map((c) => c.frame.type)).size, 11);
});
