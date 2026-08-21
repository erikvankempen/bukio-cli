/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildDigest, canonicalJson } from '../src/core/canonical.js';

const base = { v: 1, actor: 'agent:bartholomeus', cmd: 'entry add', args: { date: '2026-08-10' }, ts: '2026-08-10T12:00:00.000Z', nonce: 'n1' };

test('canonical: same input -> same digest regardless of key order', () => {
  const a = buildDigest(base);
  const shuffled = { nonce: 'n1', ts: '2026-08-10T12:00:00.000Z', args: { date: '2026-08-10' }, cmd: 'entry add', actor: 'agent:bartholomeus', v: 1 };
  const b = buildDigest(shuffled);
  assert.equal(a, b);
  assert.match(a, /^[0-9a-f]{64}$/);
});

test('canonical: different args -> different digest', () => {
  const a = buildDigest(base);
  const b = buildDigest({ ...base, args: { date: '2026-08-11' } });
  assert.notEqual(a, b);
});

test('canonical: different actor, cmd, ts or nonce -> different digest', () => {
  const d = buildDigest(base);
  assert.notEqual(buildDigest({ ...base, actor: 'human:erik' }), d);
  assert.notEqual(buildDigest({ ...base, cmd: 'entry post' }), d);
  assert.notEqual(buildDigest({ ...base, ts: '2026-08-10T13:00:00.000Z' }), d);
  assert.notEqual(buildDigest({ ...base, nonce: 'n2' }), d);
});

test('canonical: excludes --actor, --sign-key and --json from the signed args', () => {
  const withIdentityFlags = buildDigest({ ...base, args: { date: '2026-08-10', actor: 'human:erik', signKey: '/tmp/k.key', json: true } });
  const plain = buildDigest({ ...base, args: { date: '2026-08-10' } });
  assert.equal(withIdentityFlags, plain);
});

test('canonical: includes --dry-run in the signed args', () => {
  const withDry = buildDigest({ ...base, args: { date: '2026-08-10', dryRun: true } });
  const without = buildDigest({ ...base, args: { date: '2026-08-10' } });
  assert.notEqual(withDry, without);
});

test('canonical: nested args (postings, lines) are stable and order-insensitive', () => {
  const a = buildDigest({ ...base, args: { postings: [{ code: '1100', amountCents: 12100 }, { code: '1200', amountCents: -12100 }] } });
  const b = buildDigest({ ...base, args: { postings: [{ code: '1200', amountCents: -12100 }, { code: '1100', amountCents: 12100 }] } });
  assert.equal(a, b);
});

test('canonical: canonicalJson is deterministic pretty-printed JSON with sorted keys', () => {
  const json = canonicalJson({ b: 2, a: { d: 4, c: 3 }, z: [1, { y: 2, x: 1 }] });
  const reparsed = JSON.parse(json);
  assert.deepEqual(reparsed, { b: 2, a: { d: 4, c: 3 }, z: [1, { y: 2, x: 1 }] });
  assert.equal(JSON.stringify(Object.keys(JSON.parse(json))), '["a","b","z"]');
  // stable across calls
  assert.equal(json, canonicalJson({ b: 2, a: { d: 4, c: 3 }, z: [1, { y: 2, x: 1 }] }));
});
