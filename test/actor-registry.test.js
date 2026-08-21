/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
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
  setAuthz, getAuthz, grantRole, revokeRole, getRoles, hasRole, listRoleGrants,
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

// --- Tier 0.5: role registry (migration 020) --------------------------------

test('authz: flag defaults to off, toggles per DB, and is independent', () => {
  assert.equal(getAuthz(db), 'off');
  setAuthz(db, true);
  assert.equal(getAuthz(db), 'on');
  setAuthz(db, false);
  assert.equal(getAuthz(db), 'off');

  const other = openDb(':memory:');
  assert.equal(getAuthz(other), 'off');
  setAuthz(other, true);
  assert.equal(getAuthz(db), 'off'); // untouched
  assert.equal(getAuthz(other), 'on');
  other.close();
});

test('grantRole: writes a row with granted_by/granted_at; idempotent on repeat', () => {
  const row = grantRole(db, { actor: 'agent:invoicing', role: 'bookkeeper', grantedBy: 'human:erik' });
  assert.equal(row.actor, 'agent:invoicing');
  assert.equal(row.role, 'bookkeeper');
  assert.equal(row.granted_by, 'human:erik');
  assert.ok(row.granted_at);
  assert.deepEqual(getRoles(db, 'agent:invoicing'), ['bookkeeper']);
  // repeat grant is a no-op, not an error
  grantRole(db, { actor: 'agent:invoicing', role: 'bookkeeper', grantedBy: 'human:erik' });
  assert.deepEqual(getRoles(db, 'agent:invoicing'), ['bookkeeper']);
});

test('grantRole: rejects invalid actors and invalid roles', () => {
  assert.throws(() => grantRole(db, { actor: 'agent', role: 'bookkeeper', grantedBy: 'human:erik' }), { code: 'INVALID_ACTOR' });
  assert.throws(() => grantRole(db, { actor: 'agent:invoicing', role: 'superuser', grantedBy: 'human:erik' }), { code: 'INVALID_ROLE' });
  assert.throws(() => grantRole(db, { actor: 'agent:invoicing', role: 'bookkeeper', grantedBy: null }), { code: 'INVALID_ACTOR' });
});

test('revokeRole: removes the row; revoking a role not held fails ROLE_NOT_GRANTED', () => {
  grantRole(db, { actor: 'agent:tax', role: 'tax', grantedBy: 'human:erik' });
  revokeRole(db, { actor: 'agent:tax', role: 'tax' });
  assert.deepEqual(getRoles(db, 'agent:tax'), []);
  assert.throws(() => revokeRole(db, { actor: 'agent:tax', role: 'tax' }), { code: 'ROLE_NOT_GRANTED' });
  assert.throws(() => revokeRole(db, { actor: 'agent:tax', role: 'bogus' }), { code: 'INVALID_ROLE' });
});

test('revokeRole: the LAST owner can never be revoked (flipper-bootstrap guarantee)', () => {
  grantRole(db, { actor: 'human:erik', role: 'owner', grantedBy: 'human:erik' });
  assert.throws(() => revokeRole(db, { actor: 'human:erik', role: 'owner' }), { code: 'LAST_OWNER' });
  // once a second owner exists, the first may step down
  grantRole(db, { actor: 'agent:backup', role: 'owner', grantedBy: 'human:erik' });
  revokeRole(db, { actor: 'human:erik', role: 'owner' });
  assert.deepEqual(getRoles(db, 'human:erik'), []);
  assert.equal(hasRole(db, 'agent:backup', 'owner'), true);
});

test('roles: per-company independence — grants in one DB do not leak to another', () => {
  grantRole(db, { actor: 'agent:invoicing', role: 'bookkeeper', grantedBy: 'human:erik' });
  const other = openDb(':memory:');
  assert.deepEqual(getRoles(other, 'agent:invoicing'), []);
  assert.equal(hasRole(other, 'agent:invoicing', 'bookkeeper'), false);
  grantRole(other, { actor: 'agent:invoicing', role: 'readonly', grantedBy: 'human:erik' });
  assert.deepEqual(getRoles(db, 'agent:invoicing'), ['bookkeeper']); // DB A untouched
  other.close();
});

test('listRoleGrants: every grant row with grantor + timestamp', () => {
  grantRole(db, { actor: 'agent:invoicing', role: 'bookkeeper', grantedBy: 'human:erik' });
  grantRole(db, { actor: 'agent:tax', role: 'tax', grantedBy: 'human:erik' });
  const rows = listRoleGrants(db);
  assert.equal(rows.length, 2);
  const byActor = Object.fromEntries(rows.map((r) => [r.actor, r]));
  assert.equal(byActor['agent:invoicing'].role, 'bookkeeper');
  assert.equal(byActor['agent:tax'].granted_by, 'human:erik');
});

test('roles: authz flag and role grants persist to disk and survive reopen', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-role-registry-'));
  const p = path.join(dir, 'company.db');
  const first = openDb(p);
  setAuthz(first, true);
  grantRole(first, { actor: 'human:erik', role: 'owner', grantedBy: 'human:erik' });
  first.close();

  const reopened = openDb(p);
  assert.equal(getAuthz(reopened), 'on');
  assert.equal(hasRole(reopened, 'human:erik', 'owner'), true);
  reopened.close();
});
