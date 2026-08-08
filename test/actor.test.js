/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { isValidActor, actorError } from '../src/core/actor.js';

function tmpDb() {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-actor-test-'));
  return path.join(dir, 'test.db');
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
