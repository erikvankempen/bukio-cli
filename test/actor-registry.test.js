/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { openDb } from '../src/core/db.js';
import {
  enrolActor, revokeActor, getActorKey, getKeyByKeyid, canAct, setEnforce, getEnforce,
} from '../src/core/actor-registry.js';
import { generateKeyPair, keyidOf } from '../src/core/sign.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
});

function keyPair() {
  const { publicKey, privateKey } = generateKeyPair();
  return { publicKey, privateKey, keyid: keyidOf(publicKey) };
}

test('enrol: new actor writes a registry row with keyid, public key and timestamp', () => {
  const { publicKey, keyid } = keyPair();
  const row = enrolActor(db, { actor: 'agent:bartholomeus', keyid, publicKey });
  assert.equal(row.actor, 'agent:bartholomeus');
  assert.equal(row.keyid, keyid);
  assert.equal(row.public_key, publicKey);
  assert.ok(row.enrolled_at);
  assert.equal(row.revoked_at, null);
  assert.equal(getActorKey(db, 'agent:bartholomeus').keyid, keyid);
});

test('enrol: duplicate enrol while an active key exists fails ALREADY_ENROLLED', () => {
  const a = keyPair();
  const b = keyPair();
  enrolActor(db, { actor: 'human:erik', keyid: a.keyid, publicKey: a.publicKey });
  assert.throws(
    () => enrolActor(db, { actor: 'human:erik', keyid: b.keyid, publicKey: b.publicKey }),
    { code: 'ALREADY_ENROLLED' },
  );
  // the original key is untouched
  assert.equal(getActorKey(db, 'human:erik').keyid, a.keyid);
});

test('enrol: invalid actor or missing key material is rejected', () => {
  const { publicKey, keyid } = keyPair();
  assert.throws(() => enrolActor(db, { actor: 'human', keyid, publicKey }), { code: 'INVALID_ACTOR' });
  assert.throws(() => enrolActor(db, { actor: 'human:erik', keyid: '', publicKey }), { code: 'INVALID_KEY' });
  assert.throws(() => enrolActor(db, { actor: 'human:erik', keyid, publicKey: null }), { code: 'INVALID_KEY' });
});

test('revoke: marks the row with reason and keeps it (history retained)', () => {
  const { publicKey, keyid } = keyPair();
  enrolActor(db, { actor: 'agent:bartholomeus', keyid, publicKey });
  const revoked = revokeActor(db, { actor: 'agent:bartholomeus', reason: 'key rotation' });
  assert.ok(revoked.revoked_at);
  assert.equal(revoked.revoked_reason, 'key rotation');
  assert.equal(revoked.keyid, keyid); // row kept, not deleted
  // no active key remains; the revoked row is still findable by keyid
  assert.equal(getActorKey(db, 'agent:bartholomeus'), null);
  assert.equal(getKeyByKeyid(db, keyid).revoked_at, revoked.revoked_at);
});

test('revoke: requires a reason; unknown or already-revoked actors are rejected', () => {
  const { publicKey, keyid } = keyPair();
  assert.throws(() => revokeActor(db, { actor: 'human:erik', reason: '' }), { code: 'INVALID_REASON' });
  assert.throws(() => revokeActor(db, { actor: 'human:erik', reason: '   ' }), { code: 'INVALID_REASON' });
  assert.throws(() => revokeActor(db, { actor: 'human:erik', reason: 'nope' }), { code: 'NOT_ENROLLED' });
  enrolActor(db, { actor: 'human:erik', keyid, publicKey });
  revokeActor(db, { actor: 'human:erik', reason: 'lost laptop' });
  // no active key left, so a second revoke reports NOT_ENROLLED
  assert.throws(() => revokeActor(db, { actor: 'human:erik', reason: 'again' }), { code: 'NOT_ENROLLED' });
});

test('canAct: true for enrolled, false for unknown and revoked actors', () => {
  const { publicKey, keyid } = keyPair();
  assert.equal(canAct(db, 'agent:bartholomeus'), false); // not enrolled
  enrolActor(db, { actor: 'agent:bartholomeus', keyid, publicKey });
  assert.equal(canAct(db, 'agent:bartholomeus'), true);
  revokeActor(db, { actor: 'agent:bartholomeus', reason: 'compromised' });
  assert.equal(canAct(db, 'agent:bartholomeus'), false);
});

test('rotation: re-enrol after revocation adds a fresh active key; the old key row is retained', () => {
  const old = keyPair();
  const fresh = keyPair();
  enrolActor(db, { actor: 'human:erik', keyid: old.keyid, publicKey: old.publicKey });
  revokeActor(db, { actor: 'human:erik', reason: 'rotate' });
  const row = enrolActor(db, { actor: 'human:erik', keyid: fresh.keyid, publicKey: fresh.publicKey });
  assert.equal(row.keyid, fresh.keyid);
  assert.equal(row.revoked_at, null);
  assert.equal(canAct(db, 'human:erik'), true);
  // the active row is the fresh key; the OLD key row stays as history so
  // audit verify can still validate signatures made with it
  assert.equal(getActorKey(db, 'human:erik').keyid, fresh.keyid);
  assert.equal(getKeyByKeyid(db, old.keyid).revoked_at !== null, true);
  assert.equal(getKeyByKeyid(db, old.keyid).public_key, old.publicKey);
});

test('enforce: flag defaults to off, toggles per DB, and is independent', () => {
  assert.equal(getEnforce(db), 'off');
  setEnforce(db, true);
  assert.equal(getEnforce(db), 'on');
  setEnforce(db, false);
  assert.equal(getEnforce(db), 'off');

  const other = openDb(':memory:');
  assert.equal(getEnforce(other), 'off');
  setEnforce(other, true);
  assert.equal(getEnforce(db), 'off'); // untouched
  assert.equal(getEnforce(other), 'on');
  other.close();
});

test('registry is per company DB: same actor, independent enrolments', () => {
  const a = keyPair();
  const b = keyPair();
  enrolActor(db, { actor: 'agent:bartholomeus', keyid: a.keyid, publicKey: a.publicKey });

  const other = openDb(':memory:');
  assert.equal(getActorKey(other, 'agent:bartholomeus'), null); // not enrolled here yet
  enrolActor(other, { actor: 'agent:bartholomeus', keyid: b.keyid, publicKey: b.publicKey });
  assert.equal(getActorKey(other, 'agent:bartholomeus').keyid, b.keyid);
  assert.equal(getActorKey(db, 'agent:bartholomeus').keyid, a.keyid); // DB A untouched
  assert.equal(canAct(other, 'agent:bartholomeus'), true);

  revokeActor(db, { actor: 'agent:bartholomeus', reason: 'a-only' });
  assert.equal(canAct(db, 'agent:bartholomeus'), false);
  assert.equal(canAct(other, 'agent:bartholomeus'), true); // B independent
  other.close();
});

test('registry persists to disk and survives reopen (file-backed DB)', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-registry-'));
  const p = path.join(dir, 'company.db');
  const { publicKey, keyid } = keyPair();
  const first = openDb(p);
  enrolActor(first, { actor: 'system:month-end', keyid, publicKey });
  setEnforce(first, true);
  first.close();

  const reopened = openDb(p);
  assert.equal(getActorKey(reopened, 'system:month-end').keyid, keyid);
  assert.equal(getEnforce(reopened), 'on');
  reopened.close();
});
