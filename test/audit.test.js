/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import Database from 'better-sqlite3';
import { mkdtempSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { openDb, migrate } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { record, list } from '../src/audit/index.js';

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
  assert.equal(db.pragma('user_version', { simple: true }), 18);
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
