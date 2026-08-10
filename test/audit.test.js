/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import Database from 'better-sqlite3';
import crypto from 'node:crypto';
import { mkdtempSync, copyFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { openDb, migrate } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { record, list, setPendingSignature, verifyTrail } from '../src/audit/index.js';
import { buildDigest } from '../src/core/canonical.js';
import { generateKeyPair, sign } from '../src/core/sign.js';
import { enrolActor, revokeActor } from '../src/core/actor-registry.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
});

test('record + list with filters', () => {
  record(db, { actor: 'human', action: 'company.init', command: 'init', args: { name: 'X' }, outcome: 'ok' });
  record(db, { actor: 'agent:test', action: 'entry.create', command: 'entry add', args: { n: 1 }, outcome: 'ok', entryIds: [7] });

  const all = list(db);
  assert.equal(all.length, 2);
  assert.equal(all[0].action, 'entry.create');
  assert.deepEqual(all[0].entry_ids, [7]);
  assert.deepEqual(all[0].args, { n: 1 });

  const byActor = list(db, { actor: 'agent:test' });
  assert.equal(byActor.length, 1);
  const bySince = list(db, { since: '2999-01-01T00:00:00.000Z' });
  assert.equal(bySince.length, 0);
});

test('audit log is append-only: UPDATE and DELETE are blocked', () => {
  record(db, { actor: 'human', action: 'company.init', outcome: 'ok' });
  assert.throws(() => db.prepare('UPDATE audit_log SET outcome = ? WHERE action = ?').run('hacked', 'company.init'),
    /append-only/);
  assert.throws(() => db.prepare('DELETE FROM audit_log').run(), /append-only/);
  assert.equal(list(db).length, 1);
});

test('args null is stored and read back as null', () => {
  record(db, { actor: 'human', action: 'plain', outcome: 'ok' });
  const row = list(db)[0];
  assert.equal(row.args, null);
});

// --- migration 018: actor signing (Tier 0) ---------------------------------

const SIG_COLUMNS = ['digest_hash', 'sig_keyid', 'sig_nonce', 'sig_ts', 'sig', 'sig_status'];

function auditColumns(db) {
  return db.prepare('PRAGMA table_info(audit_log)').all().map((c) => c.name);
}

test('migration 018: fresh DB gains the six signature columns + actor_keys + settings', () => {
  for (const col of SIG_COLUMNS) assert.ok(auditColumns(db).includes(col), `${col} missing`);
  const keys = db.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").all().map((r) => r.name);
  assert.ok(keys.includes('actor_keys'));
  assert.ok(keys.includes('settings'));
  assert.equal(db.pragma('user_version', { simple: true }), 20);
});

test('migration 019: actor_keys gains a composite (actor, keyid) primary key', () => {
  const pk = db.prepare('PRAGMA table_info(actor_keys)').all().filter((c) => c.pk > 0)
    .map((c) => ({ name: c.name, order: c.pk }))
    .sort((a, b) => a.order - b.order);
  assert.deepEqual(pk.map((c) => c.name), ['actor', 'keyid']);
});

test('migration 019: a v18 DB with single-row actor_keys upgrades without data loss', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-audit-v18-'));
  const v18 = path.join(dir, 'v18.db');
  const raw = new Database(v18);
  raw.pragma('user_version = 18');
  raw.exec(`CREATE TABLE actor_keys (
    actor TEXT PRIMARY KEY, keyid TEXT NOT NULL, public_key TEXT NOT NULL,
    enrolled_at TEXT NOT NULL, revoked_at TEXT, revoked_reason TEXT
  )`);
  // a real v18 DB carries the 018 additions: settings (signing_enforce)
  // + the audit signature columns; migration 020 reads settings, so the
  // fixture must include it
  raw.exec('CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT)');
  raw.prepare('INSERT INTO actor_keys (actor, keyid, public_key, enrolled_at, revoked_at, revoked_reason) VALUES (?, ?, ?, ?, ?, ?)')
    .run('agent:bartholomeus', 'ab'.repeat(16), 'PUBKEY-PEM', '2026-08-10T00:00:00.000Z', '2026-08-10T01:00:00.000Z', 'test');
  raw.close();

  const upgraded = openDb(v18);
  const row = upgraded.prepare("SELECT * FROM actor_keys WHERE actor = 'agent:bartholomeus'").get();
  assert.equal(row.keyid, 'ab'.repeat(16));
  assert.equal(row.public_key, 'PUBKEY-PEM');
  assert.equal(row.revoked_reason, 'test');
  // composite PK: the same keyid can now be enrolled for a second actor
  upgraded.prepare('INSERT INTO actor_keys (actor, keyid, public_key, enrolled_at) VALUES (?, ?, ?, ?)')
    .run('agent:other', 'ab'.repeat(16), 'PUBKEY-PEM-2', '2026-08-10T02:00:00.000Z');
  upgraded.close();
});

test('migration 018: existing DB keeps legacy rows with sig_status = unsigned', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-audit-legacy-'));
  const legacyPath = path.join(dir, 'legacy.db');
  const raw = new Database(legacyPath);
  raw.pragma('user_version = 17');
  raw.exec(`CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    actor TEXT NOT NULL, action TEXT NOT NULL, command TEXT,
    args_json TEXT, outcome TEXT, entry_ids TEXT
  )`);
  raw.prepare('INSERT INTO audit_log (actor, action, outcome) VALUES (?, ?, ?)')
    .run('human:erik', 'company.init', 'ok');
  raw.close();

  const upgraded = openDb(legacyPath);
  for (const col of SIG_COLUMNS) assert.ok(auditColumns(upgraded).includes(col), `${col} missing`);
  assert.ok(upgraded.prepare("SELECT name FROM sqlite_master WHERE name = 'actor_keys'").get());
  assert.ok(upgraded.prepare("SELECT name FROM sqlite_master WHERE name = 'settings'").get());
  const row = list(upgraded)[0];
  assert.equal(row.actor, 'human:erik');
  assert.equal(row.sig_status, 'unsigned');
  assert.equal(row.digest_hash, null);
  upgraded.close();
});

test('migration 018: re-running migrate on the current version is a no-op', () => {
  const version = db.pragma('user_version', { simple: true });
  migrate(db);
  assert.equal(db.pragma('user_version', { simple: true }), version);
});

test('record: signature fields are stored and read back; plain records default to unsigned', () => {
  record(db, {
    actor: 'agent:bartholomeus', action: 'entry.add', command: 'entry add',
    args: { date: '2026-08-10' }, outcome: 'ok',
    digestHash: 'ab'.repeat(32), sigKeyid: 'deadbeef'.repeat(4), sigNonce: 'uuid-1',
    sigTs: '2026-08-10T12:00:00.000Z', sig: 'c2lnbmF0dXJl', sigStatus: 'verified',
  });
  const signed = list(db)[0];
  assert.equal(signed.digest_hash, 'ab'.repeat(32));
  assert.equal(signed.sig_keyid, 'deadbeef'.repeat(4));
  assert.equal(signed.sig_nonce, 'uuid-1');
  assert.equal(signed.sig_ts, '2026-08-10T12:00:00.000Z');
  assert.equal(signed.sig, 'c2lnbmF0dXJl');
  assert.equal(signed.sig_status, 'verified');

  record(db, { actor: 'human:erik', action: 'company.init', outcome: 'ok' });
  const plain = list(db)[0]; // newest first
  assert.equal(plain.sig_status, 'unsigned');
  assert.equal(plain.sig_keyid, null);
});

// --- verifyTrail (Task 7) ---------------------------------------------------

/** Enrol a key and return { publicKey, privateKey, keyid }. */
function enrol(db, actor) {
  const kp = generateKeyPair();
  const keyid = kp.keyid;
  enrolActor(db, { actor, keyid, publicKey: kp.publicKey });
  return { ...kp, keyid };
}

/** Record a signed row exactly the way the CLI gate would. */
function signedRow(db, { actor, cmd, args, key }) {
  const ts = new Date().toISOString();
  const nonce = crypto.randomUUID();
  const digest = buildDigest({ actor, cmd, args, ts, nonce });
  const sig = sign(digest, key.privateKey);
  setPendingSignature({
    digestHash: digest, sigKeyid: key.keyid, sigNonce: nonce, sigTs: ts, sig,
    sigStatus: 'verified', signedArgs: args,
  });
  record(db, { actor, action: 'test.action', command: cmd, outcome: 'ok' });
}

test('verifyTrail: clean signed trail -> all ok with matching summary counts', () => {
  const key = enrol(db, 'agent:bartholomeus');
  signedRow(db, { actor: 'agent:bartholomeus', cmd: 'entry add', args: { date: '2026-08-10', desc: 'x' }, key });
  signedRow(db, { actor: 'agent:bartholomeus', cmd: 'entry post', args: { id: 1 }, key });
  setPendingSignature(null); // no gate in this test: the legacy row is truly unsigned
  record(db, { actor: 'human:erik', action: 'company.init', outcome: 'ok' });

  const { rows, summary } = verifyTrail(db);
  assert.equal(summary.total, 3);
  assert.equal(summary.ok, 2);
  assert.equal(summary.unsigned, 1);
  assert.equal(summary.tampered, 0);
  assert.equal(summary.invalid_signature, 0);
  assert.equal(summary.unknown_key, 0);
  assert.equal(summary.revoked, 0);
  assert.deepEqual(rows.map((r) => r.status).sort(), ['ok', 'ok', 'unsigned']);
});

test('verifyTrail: tampered args_json -> tampered (digest no longer matches)', () => {
  const key = enrol(db, 'agent:bartholomeus');
  // an attacker who can write rows stores a digest that does not match the
  // stored args (args_json is the exact signed payload — a changed digest
  // means the payload was altered)
  const ts = new Date().toISOString();
  const nonce = crypto.randomUUID();
  const args = { date: '2026-08-10' };
  const realDigest = buildDigest({ actor: 'agent:bartholomeus', cmd: 'entry add', args, ts, nonce });
  const tamperedDigest = buildDigest({ actor: 'agent:bartholomeus', cmd: 'entry add', args: { date: '2026-01-01' }, ts, nonce });
  setPendingSignature({
    digestHash: tamperedDigest, sigKeyid: key.keyid, sigNonce: nonce, sigTs: ts,
    sig: sign(realDigest, key.privateKey), sigStatus: 'verified', signedArgs: args,
  });
  record(db, { actor: 'agent:bartholomeus', action: 'test.action', command: 'entry add', outcome: 'ok' });

  const { summary, rows } = verifyTrail(db);
  assert.equal(summary.tampered, 1);
  assert.equal(rows[0].status, 'tampered');
});

test('verifyTrail: corrupted signature -> invalid-signature', () => {
  const key = enrol(db, 'agent:bartholomeus');
  const ts = new Date().toISOString();
  const nonce = crypto.randomUUID();
  const args = { date: '2026-08-10' };
  const digest = buildDigest({ actor: 'agent:bartholomeus', cmd: 'entry add', args, ts, nonce });
  setPendingSignature({
    digestHash: digest, sigKeyid: key.keyid, sigNonce: nonce, sigTs: ts,
    sig: 'bm90LWEtc2lnbmF0dXJl', sigStatus: 'verified', signedArgs: args, // garbage sig
  });
  record(db, { actor: 'agent:bartholomeus', action: 'test.action', command: 'entry add', outcome: 'ok' });

  const { summary, rows } = verifyTrail(db);
  assert.equal(summary.invalid_signature, 1);
  assert.equal(rows[0].status, 'invalid-signature');
});

test('verifyTrail: keyid not in the registry -> unknown-key', () => {
  const ts = new Date().toISOString();
  const nonce = crypto.randomUUID();
  const args = { date: '2026-08-10' };
  const digest = buildDigest({ actor: 'agent:bartholomeus', cmd: 'entry add', args, ts, nonce });
  const stranger = generateKeyPair();
  setPendingSignature({
    digestHash: digest, sigKeyid: 'ff'.repeat(16), sigNonce: nonce, sigTs: ts,
    sig: sign(digest, stranger.privateKey), sigStatus: 'verified', signedArgs: args,
  });
  record(db, { actor: 'agent:bartholomeus', action: 'test.action', command: 'entry add', outcome: 'ok' });

  const { summary, rows } = verifyTrail(db);
  assert.equal(summary.unknown_key, 1);
  assert.equal(rows[0].status, 'unknown-key');
});

test('verifyTrail: signature from a since-revoked key -> revoked (valid at the time)', () => {
  const key = enrol(db, 'human:erik');
  signedRow(db, { actor: 'human:erik', cmd: 'entry add', args: { date: '2026-08-10' }, key });
  revokeActor(db, { actor: 'human:erik', reason: 'lost laptop' });

  const { summary, rows } = verifyTrail(db);
  assert.equal(summary.revoked, 1);
  assert.equal(summary.ok, 0);
  assert.equal(rows[0].status, 'revoked');
});

test('verifyTrail: rotation keeps old rows verifiable (old revoked, new ok)', () => {
  const old = enrol(db, 'agent:bartholomeus');
  signedRow(db, { actor: 'agent:bartholomeus', cmd: 'entry add', args: { date: '2026-08-01' }, key: old });
  revokeActor(db, { actor: 'agent:bartholomeus', reason: 'rotation' });
  const fresh = enrol(db, 'agent:bartholomeus');
  signedRow(db, { actor: 'agent:bartholomeus', cmd: 'entry add', args: { date: '2026-08-10' }, key: fresh });

  const { summary, rows } = verifyTrail(db);
  assert.equal(summary.total, 2);
  assert.equal(summary.ok, 1);
  assert.equal(summary.revoked, 1);
  assert.equal(summary.unknown_key, 0);
  assert.equal(rows[0].status, 'revoked'); // oldest first: the old key's row
  assert.equal(rows[1].status, 'ok');
});

test('verifyTrail: --limit checks only the newest N rows', () => {
  const key = enrol(db, 'agent:bartholomeus');
  signedRow(db, { actor: 'agent:bartholomeus', cmd: 'entry add', args: { n: 1 }, key });
  signedRow(db, { actor: 'agent:bartholomeus', cmd: 'entry add', args: { n: 2 }, key });
  signedRow(db, { actor: 'agent:bartholomeus', cmd: 'entry add', args: { n: 3 }, key });

  const { summary } = verifyTrail(db, { limit: 2 });
  assert.equal(summary.total, 2);
  assert.equal(summary.ok, 2);
});

test('verifyTrail: works on a copied DB file with no external files (self-contained)', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-verify-copy-'));
  const source = path.join(dir, 'company.db');
  const copy = path.join(dir, 'company-copy.db');
  const fileDb = openDb(source);
  try {
    const key = enrol(fileDb, 'agent:bartholomeus');
    signedRow(fileDb, { actor: 'agent:bartholomeus', cmd: 'entry add', args: { date: '2026-08-10' }, key });
    signedRow(fileDb, { actor: 'agent:bartholomeus', cmd: 'entry post', args: { id: 1 }, key });
    setPendingSignature(null);
    record(fileDb, { actor: 'human:erik', action: 'company.init', outcome: 'ok' });
  } finally {
    fileDb.close();
  }

  copyFileSync(source, copy);
  const copied = openDb(copy);
  try {
    const { summary, rows } = verifyTrail(copied);
    assert.equal(summary.total, 3);
    assert.equal(summary.ok, 2);
    assert.equal(summary.unsigned, 1);
    assert.equal(rows.length, 3);
  } finally {
    copied.close();
  }
});

test('verifyTrail: --since filters rows by timestamp', () => {
  const key = enrol(db, 'agent:bartholomeus');
  signedRow(db, { actor: 'agent:bartholomeus', cmd: 'entry add', args: { n: 1 }, key });
  const { rows } = verifyTrail(db, { since: '2999-01-01T00:00:00.000Z' });
  assert.equal(rows.length, 0);
});
