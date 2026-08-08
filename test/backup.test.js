/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import {
  mkdtempSync, rmSync, writeFileSync, readFileSync, existsSync, mkdirSync, readdirSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';
import {
  encryptBackupFile, decryptBackupFile, isEncryptedBackup, pruneBackups, BACKUP_MAGIC,
} from '../src/cli/backup.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

function cli(dbPath, args, { expectFail = false, env = {} } = {}) {
  const fullEnv = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test', ...env };
  try {
    const stdout = execFileSync(process.execPath, [BIN, '--json', ...args], { env: fullEnv, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    if (expectFail) return { code: err.status, out: JSON.parse(err.stdout), err: err.stderr };
    throw err;
  }
}

function tmpDir() {
  return mkdtempSync(path.join(os.tmpdir(), 'bukio-backup-test-'));
}

let t;
let dbPath;

test.beforeEach(() => {
  t = tmpDir();
  dbPath = path.join(t, 'test.db');
  cli(dbPath, ['init', '--name', 'Test Coaching', '--kvk', '12345678', '--legal-form', 'eenmanszaak', '--vat', 'off']);
  cli(dbPath, ['entry', 'add', '--date', '2026-08-10', '--desc', 'Startkapitaal', '--postings', '1100:10000.00,3000:-10000.00', '--post']);
});

test.afterEach(() => {
  delete process.env.HOME;
  rmSync(t, { recursive: true, force: true });
});

function trialBalanceSum(file) {
  const db = openDb(file);
  try {
    const rows = db.prepare('SELECT SUM(amount_cents) s FROM postings p JOIN journal_entries e ON e.id = p.entry_id WHERE e.state = \'posted\'').get();
    return rows.s;
  } finally {
    db.close();
  }
}

test('backup --encrypt: magic header, round-trips byte-identical via restore', () => {
  const encPath = path.join(t, 'enc.db.enc');
  const r = cli(dbPath, ['backup', '--out', encPath, '--encrypt', '--passphrase', 'hunter2']);
  assert.equal(r.code, 0);
  assert.equal(r.out.data.encrypted, true);
  assert.ok(existsSync(encPath));

  const head = readFileSync(encPath).subarray(0, BACKUP_MAGIC.length).toString('latin1');
  assert.equal(head, BACKUP_MAGIC);
  assert.equal(isEncryptedBackup(encPath), true);

  const restored = path.join(t, 'restored.db');
  const rr = cli(dbPath, ['restore', '--from', encPath, '--to', restored, '--passphrase', 'hunter2']);
  assert.equal(rr.code, 0);
  assert.equal(rr.out.data.encrypted, true);
  assert.equal(trialBalanceSum(dbPath), trialBalanceSum(restored));
});

test('restore: encrypted file without passphrase → BACKUP_PASSPHRASE_REQUIRED; wrong → BACKUP_PASSPHRASE_WRONG', () => {
  const encPath = path.join(t, 'enc.db.enc');
  cli(dbPath, ['backup', '--out', encPath, '--encrypt', '--passphrase', 'hunter2']);
  const restored = path.join(t, 'r.db');

  const noPass = cli(dbPath, ['restore', '--from', encPath, '--to', restored], { expectFail: true });
  assert.equal(noPass.out.error.code, 'BACKUP_PASSPHRASE_REQUIRED');
  assert.ok(!existsSync(restored));

  const wrong = cli(dbPath, ['restore', '--from', encPath, '--to', restored, '--passphrase', 'wrong'], { expectFail: true });
  assert.equal(wrong.out.error.code, 'BACKUP_PASSPHRASE_WRONG');
  assert.ok(!existsSync(restored));
});

test('restore: passphrase from BUKIO_BACKUP_PASSPHRASE env works', () => {
  const encPath = path.join(t, 'enc.db.enc');
  cli(dbPath, ['backup', '--out', encPath, '--encrypt', '--passphrase', 'envpass']);
  const restored = path.join(t, 'r.db');
  const r = cli(dbPath, ['restore', '--from', encPath, '--to', restored], { env: { BUKIO_BACKUP_PASSPHRASE: 'envpass' } });
  assert.equal(r.code, 0);
  assert.equal(trialBalanceSum(dbPath), trialBalanceSum(restored));
});

test('tampered encrypted backup → BACKUP_PASSPHRASE_WRONG', () => {
  const encPath = path.join(t, 'enc.db.enc');
  cli(dbPath, ['backup', '--out', encPath, '--encrypt', '--passphrase', 'hunter2']);
  const bytes = readFileSync(encPath);
  bytes[bytes.length - 5] ^= 0xff; // flip a ciphertext byte
  writeFileSync(encPath, bytes);
  const restored = path.join(t, 'r.db');
  const r = cli(dbPath, ['restore', '--from', encPath, '--to', restored, '--passphrase', 'hunter2'], { expectFail: true });
  assert.equal(r.out.error.code, 'BACKUP_PASSPHRASE_WRONG');
});

test('unit: encrypt/decrypt round-trip and wrong key', () => {
  const plain = path.join(t, 'plain.db');
  const enc = path.join(t, 'plain.db.enc');
  writeFileSync(plain, 'sqlite bytes 123');
  const size = encryptBackupFile(plain, enc, 'pass');
  assert.ok(size > 0);
  const dec = decryptBackupFile(enc, 'pass');
  assert.equal(dec.toString('utf8'), 'sqlite bytes 123');
  assert.throws(() => decryptBackupFile(enc, 'nope'), (e) => e.code === 'BACKUP_PASSPHRASE_WRONG');
});

test('--keep N prunes oldest backups in the default folder; dry-run deletes nothing', () => {
  const home = path.join(t, 'home');
  const backupDir = path.join(home, '.bukio', 'backups');
  mkdirSync(backupDir, { recursive: true });
  // four fake backups, oldest first
  const names = ['bukio-2026-08-01T00-00-00.db', 'bukio-2026-08-02T00-00-00.db', 'bukio-2026-08-03T00-00-00.db', 'bukio-2026-08-04T00-00-00.db'];
  for (const n of names) writeFileSync(path.join(backupDir, n), 'x');
  // an unrelated file must never be pruned
  writeFileSync(path.join(backupDir, 'notes.txt'), 'keep me');

  process.env.HOME = home;

  // dry-run: nothing deleted
  const dry = cli(dbPath, ['backup', '--keep', '2', '--dry-run']);
  assert.equal(dry.code, 0);
  assert.equal(dry.out.data.pruned.length, 2);
  assert.equal(readdirSync(backupDir).length, 5);

  // real run: prunes AFTER the new backup exists — keep N TOTAL, not N+1
  const real = cli(dbPath, ['backup', '--keep', '2']);
  assert.equal(real.code, 0);
  assert.equal(real.out.data.pruned.length, 3); // 4 old + 1 new = 5 -> keep 2
  const remaining = readdirSync(backupDir).sort();
  assert.ok(!remaining.includes('bukio-2026-08-01T00-00-00.db'));
  assert.ok(!remaining.includes('bukio-2026-08-02T00-00-00.db'));
  assert.ok(!remaining.includes('bukio-2026-08-03T00-00-00.db'));
  assert.ok(remaining.includes('bukio-2026-08-04T00-00-00.db'));
  // exactly the 2 newest remain (08-04 + the new backup)
  assert.equal(remaining.filter((f) => f.startsWith('bukio-') && f.endsWith('.db')).length, 2);
  assert.ok(remaining.includes('notes.txt'));
});

test('--keep validation: non-integer, zero, and with --out all rejected', () => {
  assert.equal(cli(dbPath, ['backup', '--keep', 'abc'], { expectFail: true }).out.error.code, 'INVALID_KEEP');
  assert.equal(cli(dbPath, ['backup', '--keep', '0'], { expectFail: true }).out.error.code, 'INVALID_KEEP');
  const r = cli(dbPath, ['backup', '--keep', '2', '--out', path.join(t, 'x.db')], { expectFail: true });
  assert.equal(r.out.error.code, 'INVALID_KEEP');
});

test('plain backup/restore still works (regression) + both actions audited', () => {
  const backupPath = path.join(t, 'plain.db');
  const r = cli(dbPath, ['backup', '--out', backupPath]);
  assert.equal(r.code, 0);
  assert.equal(r.out.data.encrypted, false);
  assert.equal(isEncryptedBackup(backupPath), false);

  const restored = path.join(t, 'restored-plain.db');
  cli(dbPath, ['restore', '--from', backupPath, '--to', restored]);
  assert.equal(trialBalanceSum(dbPath), trialBalanceSum(restored));

  // audit rows in the SOURCE (backup) and RESTORED (restore) DBs
  let db = openDb(dbPath);
  try {
    const row = db.prepare("SELECT * FROM audit_log WHERE action = 'backup' ORDER BY id DESC LIMIT 1").get();
    assert.ok(row);
    assert.equal(row.actor, 'agent:test');
    assert.equal(JSON.parse(row.args_json).encrypted, false);
  } finally {
    db.close();
  }
  db = openDb(restored);
  try {
    const row = db.prepare("SELECT * FROM audit_log WHERE action = 'restore' ORDER BY id DESC LIMIT 1").get();
    assert.ok(row);
    assert.equal(row.actor, 'agent:test');
    assert.equal(JSON.parse(row.args_json).encrypted, false);
  } finally {
    db.close();
  }
});

test('pruneBackups: empty/missing folder is a no-op', () => {
  process.env.HOME = path.join(t, 'nohome');
  const r = pruneBackups(3);
  assert.deepEqual(r.pruned, []);
});
