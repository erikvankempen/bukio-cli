/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, existsSync, statSync, readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { isValidActor, actorError } from '../src/core/actor.js';
import { openDb } from '../src/core/db.js';
import { readSessionKey, sessionFilePath, verifySignatureBundle, isNonceUsed } from '../src/cli/util.js';
import { buildDigest } from '../src/core/canonical.js';
import { generateKeyPair, sign } from '../src/core/sign.js';
import { enrolActor, revokeActor } from '../src/core/actor-registry.js';

function tmpDb() {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-actor-test-'));
  return path.join(dir, 'test.db');
}

function tmpConfig() {
  return mkdtempSync(path.join(os.tmpdir(), 'bukio-actor-cfg-'));
}

function envWithoutSigningPassphrase(extra = {}) {
  const env = { ...process.env, ...extra };
  delete env.BUKIO_SIGNING_PASSPHRASE;
  return env;
}

test('isValidActor: role:name formats', () => {
  assert.equal(isValidActor('agent:bartholomeus'), true);
  assert.equal(isValidActor('human:erik'), true);
  assert.equal(isValidActor('system:close'), true);
  assert.equal(isValidActor('agent:a.b_c-1'), true);
  // bare roles and malformed names are rejected
  assert.equal(isValidActor('human'), false);
  assert.equal(isValidActor('agent'), false);
  assert.equal(isValidActor('agent:'), false);
  assert.equal(isValidActor(':erik'), false);
  assert.equal(isValidActor('human erik'), false);
  assert.equal(isValidActor('human:john smith'), false);
  assert.equal(isValidActor(''), false);
  assert.equal(isValidActor(null), false);
});

test('actorError: helpful messages for missing and malformed actors', () => {
  const missing = actorError(null);
  assert.equal(missing.code, 'ACTOR_REQUIRED');
  assert.ok(missing.message.includes('agent:bartholomeus'));
  const bad = actorError('human');
  assert.equal(bad.code, 'INVALID_ACTOR');
  assert.ok(bad.message.includes("'<role>:<name>'"));
  assert.equal(actorError('human:erik'), null);
});

function runCli(args, env = {}) {
  return spawnSync(process.execPath, ['bin/bukio.js', ...args], {
    cwd: process.cwd(), encoding: 'utf8', env: { ...process.env, ...env },
  });
}

test('CLI: missing actor fails with ACTOR_REQUIRED', () => {
  const r = runCli(['init', '--name', 'X', '--db', tmpDb()], { BUKIO_ACTOR: '' });
  assert.equal(r.status, 1);
  assert.ok(r.stderr.includes('ACTOR_REQUIRED'));
  assert.ok(r.stderr.includes('human:erik'));
});

test('CLI: bare role without a name is rejected (INVALID_ACTOR)', () => {
  const r2 = runCli(['--actor', 'human', 'init', '--name', 'X', '--db', tmpDb()], { BUKIO_ACTOR: '' });
  assert.equal(r2.status, 1);
  assert.ok(r2.stderr.includes('INVALID_ACTOR'));
  assert.ok(r2.stderr.includes('human:erik'));
});

test('CLI: named actor works; JSON error shape on --json', () => {
  const r = runCli(['--actor', 'human:erik', 'init', '--name', 'X', '--db', tmpDb()], { BUKIO_ACTOR: '' });
  assert.equal(r.status, 0, r.stderr);
  const bad = runCli(['--json', '--actor', 'human', 'init', '--name', 'Y', '--db', tmpDb()], { BUKIO_ACTOR: '' });
  assert.equal(bad.status, 1);
  const parsed = JSON.parse(bad.stdout);
  assert.equal(parsed.ok, false);
  assert.equal(parsed.error.code, 'INVALID_ACTOR');
  assert.equal(parsed.error.message.includes('agent:bartholomeus'), true);
});

test('CLI: BUKIO_ACTOR env satisfies the requirement', () => {
  const r = runCli(['init', '--name', 'X', '--db', tmpDb()], { BUKIO_ACTOR: 'human:erik' });
  assert.equal(r.status, 0, r.stderr);
});

test('CLI: BUKIO_ACTOR env is recorded in the audit trail', () => {
  const db = tmpDb();
  const r = runCli(['init', '--name', 'X', '--db', db], { BUKIO_ACTOR: 'human:erik' });
  assert.equal(r.status, 0, r.stderr);
  const audit = runCli(['audit', '--db', db, '--json'], { BUKIO_ACTOR: 'human:erik' });
  assert.equal(audit.status, 0, audit.stderr);
  const rows = JSON.parse(audit.stdout).data.entries;
  assert.ok(rows.length > 0);
  // every recorded action must carry the env actor — not 'human' / undefined
  for (const row of rows) {
    assert.equal(row.actor, 'human:erik', `audit row ${row.id} actor should come from BUKIO_ACTOR`);
  }
});

// --- actor identity commands (Tier 0: keygen/register/list/revoke/enforce/unlock/lock/verify) ---

function keyFile(cfg, actor) {
  return path.join(cfg, 'keys', actor.replace(':', '-') + '.key');
}

test('actor keygen: agent key writes a plain 0600 key file (BUKIO_CONFIG_DIR respected)', () => {
  const cfg = tmpConfig();
  const r = runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'keygen'], { BUKIO_CONFIG_DIR: cfg });
  assert.equal(r.status, 0, r.stderr);
  const data = JSON.parse(r.stdout).data;
  assert.equal(data.actor, 'agent:bartholomeus');
  assert.equal(data.encrypted, false);
  assert.match(data.keyid, /^[0-9a-f]{32}$/);
  const file = keyFile(cfg, 'agent:bartholomeus');
  assert.ok(existsSync(file), 'key file written');
  assert.equal(statSync(file).mode & 0o777, 0o600);
  const pem = readFileSync(file, 'utf8');
  assert.ok(pem.includes('-----BEGIN PRIVATE KEY-----'));
  assert.ok(!pem.includes('ENCRYPTED'));
});

test('actor keygen: human key is passphrase-encrypted via BUKIO_SIGNING_PASSPHRASE', () => {
  const cfg = tmpConfig();
  const r = runCli(
    ['--json', '--actor', 'human:erik', 'actor', 'keygen'],
    { BUKIO_CONFIG_DIR: cfg, BUKIO_SIGNING_PASSPHRASE: 'hunter2' },
  );
  assert.equal(r.status, 0, r.stderr);
  const data = JSON.parse(r.stdout).data;
  assert.equal(data.encrypted, true);
  const pem = readFileSync(keyFile(cfg, 'human:erik'), 'utf8');
  assert.ok(pem.includes('-----BEGIN ENCRYPTED PRIVATE KEY-----'));
});

test('actor keygen: refuses to overwrite; --force replaces (rotation)', () => {
  const cfg = tmpConfig();
  const args = ['--json', '--actor', 'agent:bartholomeus', 'actor', 'keygen'];
  assert.equal(runCli(args, { BUKIO_CONFIG_DIR: cfg }).status, 0);
  const dup = runCli(args, { BUKIO_CONFIG_DIR: cfg });
  assert.equal(dup.status, 1);
  assert.equal(JSON.parse(dup.stdout).error.code, 'KEY_ALREADY_EXISTS');
  const forced = runCli([...args, '--force'], { BUKIO_CONFIG_DIR: cfg });
  assert.equal(forced.status, 0, forced.stderr);
});

test('actor keygen: human key without a passphrase in a non-interactive shell fails PASSPHRASE_REQUIRED', () => {
  const cfg = tmpConfig();
  const r = runCli(['--json', '--actor', 'human:erik', 'actor', 'keygen'],
    envWithoutSigningPassphrase({ BUKIO_CONFIG_DIR: cfg }));
  assert.equal(r.status, 1);
  assert.equal(JSON.parse(r.stdout).error.code, 'PASSPHRASE_REQUIRED');
});

test('actor register: enrols the local key into the current company DB and audits it', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = { BUKIO_CONFIG_DIR: cfg, BUKIO_ACTOR: '' };
  assert.equal(runCli(['--actor', 'human:erik', 'init', '--name', 'X', '--db', db], base).status, 0);
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'keygen'], { BUKIO_CONFIG_DIR: cfg });
  const reg = runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'register', '--db', db], base);
  assert.equal(reg.status, 0, reg.stderr);
  const data = JSON.parse(reg.stdout).data;
  assert.equal(data.enrolled, true);
  assert.match(data.keyid, /^[0-9a-f]{32}$/);

  const handle = openDb(db);
  try {
    const row = handle.prepare('SELECT * FROM actor_keys WHERE actor = ?').get('agent:bartholomeus');
    assert.equal(row.keyid, data.keyid);
    assert.equal(row.revoked_at, null);
    const audit = handle.prepare("SELECT * FROM audit_log WHERE action = 'actor.register'").get();
    assert.ok(audit, 'register writes an audit row');
    assert.equal(audit.actor, 'agent:bartholomeus');
  } finally {
    handle.close();
  }
});

test('actor revoke: requires a reason; revoke marks the row and audits it', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = { BUKIO_CONFIG_DIR: cfg, BUKIO_ACTOR: '' };
  runCli(['--actor', 'human:erik', 'init', '--name', 'X', '--db', db], base);
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'keygen'], { BUKIO_CONFIG_DIR: cfg });
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'register', '--db', db], base);

  const noReason = runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'revoke', '--db', db], base);
  assert.equal(noReason.status, 1);
  assert.equal(JSON.parse(noReason.stdout).error.code, 'INVALID_REASON');

  const ok = runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'revoke', '--db', db, '--reason', 'test rotation'], base);
  assert.equal(ok.status, 0, ok.stderr);
  const handle = openDb(db);
  try {
    const row = handle.prepare('SELECT * FROM actor_keys WHERE actor = ?').get('agent:bartholomeus');
    assert.ok(row.revoked_at);
    assert.equal(row.revoked_reason, 'test rotation');
    assert.ok(handle.prepare("SELECT * FROM audit_log WHERE action = 'actor.revoke'").get());
  } finally {
    handle.close();
  }
});

test('actor enforce: --on/--off toggles the per-company flag and audits it', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = { BUKIO_CONFIG_DIR: cfg, BUKIO_ACTOR: '' };
  runCli(['--actor', 'human:erik', 'init', '--name', 'X', '--db', db], base);
  const handle = openDb(db);
  try {
    assert.equal(handle.prepare("SELECT value FROM settings WHERE key = 'signing_enforce'").get(), undefined);
  } finally {
    handle.close();
  }
  const on = runCli(['--json', '--actor', 'human:erik', 'actor', 'enforce', '--on', '--db', db], base);
  assert.equal(on.status, 0, on.stderr);
  assert.equal(JSON.parse(on.stdout).data.enforce, 'on');
  const off = runCli(['--json', '--actor', 'human:erik', 'actor', 'enforce', '--off', '--db', db], base);
  assert.equal(off.status, 0, off.stderr);
  assert.equal(JSON.parse(off.stdout).data.enforce, 'off');
  // both flips are audited
  const handle2 = openDb(db);
  try {
    assert.equal(handle2.prepare("SELECT value FROM settings WHERE key = 'signing_enforce'").get().value, 'off');
    assert.equal(handle2.prepare("SELECT COUNT(*) c FROM audit_log WHERE action = 'actor.enforce'").get().c, 2);
  } finally {
    handle2.close();
  }
});

test('actor unlock: wrong passphrase -> PASSPHRASE_INVALID; correct -> session key with expiry; lock clears it', () => {
  const cfg = tmpConfig();
  const env = { BUKIO_CONFIG_DIR: cfg, BUKIO_SIGNING_PASSPHRASE: 'correct horse' };
  const keygen = runCli(['--json', '--actor', 'human:erik', 'actor', 'keygen'], env);
  assert.equal(keygen.status, 0, keygen.stderr);

  const wrong = runCli(['--json', '--actor', 'human:erik', 'actor', 'unlock'],
    { BUKIO_CONFIG_DIR: cfg, BUKIO_SIGNING_PASSPHRASE: 'battery staple' });
  assert.equal(wrong.status, 1);
  assert.equal(JSON.parse(wrong.stdout).error.code, 'PASSPHRASE_INVALID');

  const ok = runCli(['--json', '--actor', 'human:erik', 'actor', 'unlock'], env);
  assert.equal(ok.status, 0, ok.stderr);
  const data = JSON.parse(ok.stdout).data;
  const session = path.join(cfg, 'sessions', 'human-erik.key');
  assert.equal(data.sessionFile, session);
  const raw = JSON.parse(readFileSync(session, 'utf8'));
  assert.ok(raw.keyPem.includes('-----BEGIN PRIVATE KEY-----'));
  assert.ok(new Date(raw.expiresAt).getTime() > Date.now() + 11 * 3600_000, 'expiry ~12h out');
  assert.equal(statSync(session).mode & 0o777, 0o600);

  const lock = runCli(['--json', '--actor', 'human:erik', 'actor', 'lock'], { BUKIO_CONFIG_DIR: cfg });
  assert.equal(lock.status, 0, lock.stderr);
  assert.equal(JSON.parse(lock.stdout).data.removed, true);
  assert.equal(existsSync(session), false);
});

test('actor unlock: agent keys are not unlocked per session', () => {
  const cfg = tmpConfig();
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'keygen'], { BUKIO_CONFIG_DIR: cfg });
  const r = runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'unlock'], { BUKIO_CONFIG_DIR: cfg });
  assert.equal(r.status, 1);
  assert.equal(JSON.parse(r.stdout).error.code, 'UNLOCK_NOT_APPLICABLE');
});

test('actor list: shows enrolled and revoked actors', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = { BUKIO_CONFIG_DIR: cfg, BUKIO_ACTOR: '' };
  runCli(['--actor', 'human:erik', 'init', '--name', 'X', '--db', db], base);
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'keygen'], { BUKIO_CONFIG_DIR: cfg });
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'register', '--db', db], base);
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'revoke', '--db', db, '--reason', 'test'], base);

  const r = runCli(['--json', '--actor', 'human:erik', 'actor', 'list', '--db', db], base);
  assert.equal(r.status, 0, r.stderr);
  const actors = JSON.parse(r.stdout).data.actors;
  assert.equal(actors.length, 1);
  assert.equal(actors[0].actor, 'agent:bartholomeus');
  assert.equal(actors[0].active, false); // revoked
  assert.ok(actors[0].revoked_at);
});

test('actor verify: reports key state against the current company registry', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = { BUKIO_CONFIG_DIR: cfg, BUKIO_ACTOR: '' };
  runCli(['--actor', 'human:erik', 'init', '--name', 'X', '--db', db], base);
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'keygen'], { BUKIO_CONFIG_DIR: cfg });
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'register', '--db', db], base);

  const v1 = runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'verify', '--db', db], base);
  assert.equal(v1.status, 0, v1.stderr);
  const d1 = JSON.parse(v1.stdout).data;
  assert.equal(d1.registered, true);
  assert.equal(d1.active, true);
  assert.equal(d1.keyFileExists, true);

  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'revoke', '--db', db, '--reason', 'test'], base);
  const v2 = runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'verify', '--db', db], base);
  assert.equal(JSON.parse(v2.stdout).data.active, false);
});

test('actor commands reject invalid actor strings with INVALID_ACTOR', () => {
  const r = runCli(['--json', '--actor', 'human', 'actor', 'keygen'], envWithoutSigningPassphrase({ BUKIO_CONFIG_DIR: tmpConfig() }));
  assert.equal(r.status, 1);
  assert.equal(JSON.parse(r.stdout).error.code, 'INVALID_ACTOR');
});

test('readSessionKey: expired or missing session files count as locked', () => {
  const cfg = tmpConfig();
  const prev = process.env.BUKIO_CONFIG_DIR;
  process.env.BUKIO_CONFIG_DIR = cfg;
  try {
    assert.equal(readSessionKey('human:erik'), null); // no file yet
    const session = sessionFilePath('human:erik');
    mkdirSync(path.dirname(session), { recursive: true });
    writeFileSync(session,
      JSON.stringify({ keyPem: 'x', expiresAt: '2000-01-01T00:00:00.000Z' }), { mode: 0o600 });
    assert.equal(readSessionKey('human:erik'), null); // expired
  } finally {
    if (prev === undefined) delete process.env.BUKIO_CONFIG_DIR;
    else process.env.BUKIO_CONFIG_DIR = prev;
  }
});

// --- sign-and-verify gate (Task 5) ----------------------------------------

const ENTRY_ARGS = ['entry', 'add', '--date', '2026-08-10', '--desc', 'Gate test', '--postings', '1100:100.00,8000:-100.00', '--post'];

function lastAuditRow(dbPath) {
  const handle = openDb(dbPath);
  try {
    return handle.prepare('SELECT * FROM audit_log ORDER BY id DESC LIMIT 1').get();
  } finally {
    handle.close();
  }
}

function entryCount(dbPath) {
  const handle = openDb(dbPath);
  try {
    return handle.prepare('SELECT COUNT(*) c FROM journal_entries').get().c;
  } finally {
    handle.close();
  }
}

/** init + keygen agent + register agent (enforce still off). */
function setupEnrolledAgent(cfg, db) {
  const base = { BUKIO_CONFIG_DIR: cfg, BUKIO_ACTOR: '' };
  assert.equal(runCli(['--actor', 'human:erik', 'init', '--name', 'X', '--db', db], base).status, 0);
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'keygen'], { BUKIO_CONFIG_DIR: cfg });
  const reg = runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'register', '--db', db], base);
  assert.equal(reg.status, 0, reg.stderr);
  return base;
}

test('sign gate: record mode + enrolled key -> command runs, audit row verified', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = setupEnrolledAgent(cfg, db);
  const r = runCli(['--json', '--actor', 'agent:bartholomeus', ...ENTRY_ARGS, '--db', db], base);
  assert.equal(r.status, 0, r.stderr);
  const row = lastAuditRow(db);
  assert.equal(row.sig_status, 'verified');
  assert.match(row.digest_hash, /^[0-9a-f]{64}$/);
  assert.match(row.sig_keyid, /^[0-9a-f]{32}$/);
  assert.ok(row.sig);
  assert.ok(row.sig_nonce);
  assert.ok(row.sig_ts);
});

test('sign gate: record mode + no key -> runs, logged unsigned', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = { BUKIO_CONFIG_DIR: cfg, BUKIO_ACTOR: '' };
  assert.equal(runCli(['--actor', 'human:erik', 'init', '--name', 'X', '--db', db], base).status, 0);
  const r = runCli(['--json', '--actor', 'system:month-end', ...ENTRY_ARGS, '--db', db], base);
  assert.equal(r.status, 0, r.stderr);
  const row = lastAuditRow(db);
  assert.equal(row.sig_status, 'unsigned');
  assert.equal(row.sig, null);
});

test('sign gate: enforce on + no key -> SIGNATURE_REQUIRED, nothing mutated (JSON contract)', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = setupEnrolledAgent(cfg, db);
  const on = runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'enforce', '--on', '--db', db], base);
  assert.equal(on.status, 0, on.stderr);

  const r = runCli(['--json', '--actor', 'system:month-end', ...ENTRY_ARGS, '--db', db], base);
  assert.equal(r.status, 1);
  const err = JSON.parse(r.stdout).error;
  assert.equal(err.code, 'SIGNATURE_REQUIRED');
  assert.equal(entryCount(db), 0); // nothing mutated
});

test('sign gate: enforce on + wrong key -> SIGNATURE_INVALID', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = setupEnrolledAgent(cfg, db);
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'enforce', '--on', '--db', db], base);
  // rotate the local key WITHOUT re-registering -> local key != registered key
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'keygen', '--force'], { BUKIO_CONFIG_DIR: cfg });
  const r = runCli(['--json', '--actor', 'agent:bartholomeus', ...ENTRY_ARGS, '--db', db], base);
  assert.equal(r.status, 1);
  assert.equal(JSON.parse(r.stdout).error.code, 'SIGNATURE_INVALID');
  assert.equal(entryCount(db), 0);
});

test('sign gate: locked human key -> PASSPHRASE_REQUIRED; env passphrase unlocks', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const withPass = { BUKIO_CONFIG_DIR: cfg, BUKIO_SIGNING_PASSPHRASE: 'hunter2' };
  assert.equal(runCli(['--actor', 'human:erik', 'init', '--name', 'X', '--db', db], { BUKIO_CONFIG_DIR: cfg, BUKIO_ACTOR: '' }).status, 0);
  assert.equal(runCli(['--json', '--actor', 'human:erik', 'actor', 'keygen'], withPass).status, 0);
  assert.equal(runCli(['--json', '--actor', 'human:erik', 'actor', 'register', '--db', db], withPass).status, 0);
  assert.equal(runCli(['--json', '--actor', 'human:erik', 'actor', 'enforce', '--on', '--db', db], withPass).status, 0);

  const locked = runCli(['--json', '--actor', 'human:erik', ...ENTRY_ARGS, '--db', db],
    envWithoutSigningPassphrase({ BUKIO_CONFIG_DIR: cfg }));
  assert.equal(locked.status, 1);
  assert.equal(JSON.parse(locked.stdout).error.code, 'PASSPHRASE_REQUIRED');

  const unlocked = runCli(['--json', '--actor', 'human:erik', ...ENTRY_ARGS, '--db', db], withPass);
  assert.equal(unlocked.status, 0, unlocked.stderr);
  assert.equal(lastAuditRow(db).sig_status, 'verified');
});

test('sign gate: unknown actor key -> ACTOR_KEY_UNKNOWN', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = setupEnrolledAgent(cfg, db);
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'enforce', '--on', '--db', db], base);
  // a key file exists for this actor, but it was never registered
  runCli(['--json', '--actor', 'system:cron', 'actor', 'keygen'], { BUKIO_CONFIG_DIR: cfg });
  const r = runCli(['--json', '--actor', 'system:cron', ...ENTRY_ARGS, '--db', db], base);
  assert.equal(r.status, 1);
  assert.equal(JSON.parse(r.stdout).error.code, 'ACTOR_KEY_UNKNOWN');
});

test('sign gate: revoked key -> ACTOR_KEY_REVOKED', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = setupEnrolledAgent(cfg, db);
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'enforce', '--on', '--db', db], base);
  // the gate verified the signature BEFORE the action, so the actor can
  // revoke its own key; the next command must be refused
  const revoke = runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'revoke', '--db', db, '--reason', 'test'], base);
  assert.equal(revoke.status, 0, revoke.stderr);
  const r = runCli(['--json', '--actor', 'agent:bartholomeus', ...ENTRY_ARGS, '--db', db], base);
  assert.equal(r.status, 1);
  assert.equal(JSON.parse(r.stdout).error.code, 'ACTOR_KEY_REVOKED');
});

test('sign gate: --dry-run fails identically before any mutation', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = setupEnrolledAgent(cfg, db);
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'enforce', '--on', '--db', db], base);
  const r = runCli(['--json', '--actor', 'system:month-end', 'entry', 'add', '--date', '2026-08-10', '--desc', 'X', '--postings', '1100:100.00,8000:-100.00', '--dry-run', '--db', db], base);
  assert.equal(r.status, 1);
  assert.equal(JSON.parse(r.stdout).error.code, 'SIGNATURE_REQUIRED');
  assert.equal(entryCount(db), 0);
});

test('sign gate: exempt commands keep working under enforcement (keygen, enforce --off)', () => {
  const cfg = tmpConfig();
  const db = tmpDb();
  const base = setupEnrolledAgent(cfg, db);
  runCli(['--json', '--actor', 'agent:bartholomeus', 'actor', 'enforce', '--on', '--db', db], base);
  // keygen is exempt (its own key does not exist yet)
  const kg = runCli(['--json', '--actor', 'system:new', 'actor', 'keygen'], { BUKIO_CONFIG_DIR: cfg });
  assert.equal(kg.status, 0, kg.stderr);
  // enforce --off is the recovery escape hatch and works unsigned
  const off = runCli(['--json', '--actor', 'system:new', 'actor', 'enforce', '--off', '--db', db], base);
  assert.equal(off.status, 0, off.stderr);
  // enforcement is off again: unsigned commands run
  const r = runCli(['--json', '--actor', 'system:new', ...ENTRY_ARGS, '--db', db], base);
  assert.equal(r.status, 0, r.stderr);
});

// --- verifySignatureBundle unit checks (stale / replay / registry states) ---

test('verifySignatureBundle: stale timestamp -> SIGNATURE_STALE under enforce', () => {
  const db = openDb(':memory:');
  try {
    const { publicKey, privateKey, keyid } = generateKeyPair();
    enrolActor(db, { actor: 'agent:bartholomeus', keyid, publicKey });
    const digest = buildDigest({ actor: 'agent:bartholomeus', cmd: 'entry add', args: {}, ts: '2026-08-10T12:00:00.000Z', nonce: 'fresh-1' });
    const sig = sign(digest, privateKey);
    const r = verifySignatureBundle(db, {
      actor: 'agent:bartholomeus', digest, sig, keyid, ts: '2026-08-10T12:00:00.000Z', nonce: 'fresh-1', enforce: true,
    });
    assert.equal(r.ok, false);
    assert.equal(r.code, 'SIGNATURE_STALE');
  } finally {
    db.close();
  }
});

test('verifySignatureBundle: reused nonce -> NONCE_REUSED even in record mode', () => {
  const cfg = tmpConfig();
  const prev = process.env.BUKIO_CONFIG_DIR;
  process.env.BUKIO_CONFIG_DIR = cfg;
  try {
    const db = openDb(':memory:');
    try {
      const { publicKey, privateKey, keyid } = generateKeyPair();
      enrolActor(db, { actor: 'agent:bartholomeus', keyid, publicKey });
      const ts = new Date().toISOString();
      const digest = buildDigest({ actor: 'agent:bartholomeus', cmd: 'entry add', args: {}, ts, nonce: 'same-nonce' });
      const sig = sign(digest, privateKey);
      const bundle = { actor: 'agent:bartholomeus', digest, sig, keyid, ts, nonce: 'same-nonce', enforce: false };
      const first = verifySignatureBundle(db, bundle);
      assert.equal(first.ok, true);
      assert.equal(first.status, 'verified');
      assert.ok(isNonceUsed(keyid, 'same-nonce'));
      const second = verifySignatureBundle(db, bundle);
      assert.equal(second.ok, false);
      assert.equal(second.code, 'NONCE_REUSED');
    } finally {
      db.close();
    }
  } finally {
    if (prev === undefined) delete process.env.BUKIO_CONFIG_DIR;
    else process.env.BUKIO_CONFIG_DIR = prev;
  }
});

test('verifySignatureBundle: record mode tolerates unknown/revoked/invalid as unsigned', () => {
  const ts = new Date().toISOString();
  // unknown actor -> unsigned, still ok
  {
    const db = openDb(':memory:');
    try {
      const { privateKey, keyid } = generateKeyPair();
      const digest = buildDigest({ actor: 'agent:bartholomeus', cmd: 'entry add', args: {}, ts, nonce: 'n-unk' });
      const sig = sign(digest, privateKey);
      const r = verifySignatureBundle(db, { actor: 'agent:bartholomeus', digest, sig, keyid, ts, nonce: 'n-unk', enforce: false });
      assert.equal(r.ok, true);
      assert.equal(r.status, 'unsigned');
    } finally {
      db.close();
    }
  }
  // revoked -> unsigned
  {
    const db = openDb(':memory:');
    try {
      const { publicKey, privateKey, keyid } = generateKeyPair();
      enrolActor(db, { actor: 'agent:bartholomeus', keyid, publicKey });
      revokeActor(db, { actor: 'agent:bartholomeus', reason: 'test' });
      const digest = buildDigest({ actor: 'agent:bartholomeus', cmd: 'entry add', args: {}, ts, nonce: 'n-rev' });
      const sig = sign(digest, privateKey);
      const r = verifySignatureBundle(db, { actor: 'agent:bartholomeus', digest, sig, keyid, ts, nonce: 'n-rev', enforce: false });
      assert.equal(r.ok, true);
      assert.equal(r.status, 'unsigned');
    } finally {
      db.close();
    }
  }
  // wrong key (signed by a key that is not the enrolled one) -> unsigned
  {
    const db = openDb(':memory:');
    try {
      const enrolled = generateKeyPair();
      const imposter = generateKeyPair();
      enrolActor(db, { actor: 'agent:bartholomeus', keyid: enrolled.keyid, publicKey: enrolled.publicKey });
      const digest = buildDigest({ actor: 'agent:bartholomeus', cmd: 'entry add', args: {}, ts, nonce: 'n-wrong' });
      const sig = sign(digest, imposter.privateKey);
      const r = verifySignatureBundle(db, { actor: 'agent:bartholomeus', digest, sig, keyid: imposter.keyid, ts, nonce: 'n-wrong', enforce: false });
      assert.equal(r.ok, true);
      assert.equal(r.status, 'unsigned');
    } finally {
      db.close();
    }
  }
});
