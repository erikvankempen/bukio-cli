/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio update — self-update from origin/main.
// ALL git activity runs in mkdtemp fixtures (bare origin + clones); the real
// repository and the live database are NEVER touched by these tests.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, readFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { openDb } from '../src/core/db.js';
import { planUpdate, runUpdate } from '../src/update/index.js';

function sh(cwd, args) {
  return execFileSync('git', args, { cwd, encoding: 'utf8' }).trim();
}

/**
 * Build a fixture: bare origin with main at v1, plus a second clone that
 * pushes `v2Files` (default: a README change) on top. Returns { dir, origin,
 * work1 } where work1 is a clone sitting at v1 with origin/main at v2.
 */
function setupFixture(v2Files = [['README.md', 'v2 content\n']]) {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-upd-'));
  const origin = path.join(dir, 'origin.git');
  sh(dir, ['init', '--bare', 'origin.git']);
  sh(dir, ['--git-dir=origin.git', 'symbolic-ref', 'HEAD', 'refs/heads/main']);

  const work1 = path.join(dir, 'work1');
  sh(dir, ['clone', origin, 'work1']);
  sh(work1, ['config', 'user.email', 'test@example.com']);
  sh(work1, ['config', 'user.name', 'Test']);
  writeFileSync(path.join(work1, 'package.json'), '{"name":"bukio-cli","version":"1.0.0"}\n');
  writeFileSync(path.join(work1, 'README.md'), 'v1 content\n');
  sh(work1, ['add', '.']);
  sh(work1, ['commit', '-m', 'v1']);
  sh(work1, ['push', '-u', 'origin', 'main']);

  const work2 = path.join(dir, 'work2');
  sh(dir, ['clone', origin, 'work2']);
  sh(work2, ['config', 'user.email', 'test@example.com']);
  sh(work2, ['config', 'user.name', 'Test']);
  for (const [file, content] of v2Files) writeFileSync(path.join(work2, file), content);
  sh(work2, ['add', '.']);
  sh(work2, ['commit', '-m', 'v2']);
  sh(work2, ['push', 'origin', 'main']);

  return { dir, origin, work1 };
}

test('update plan: a non-clone directory is refused', () => {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-upd-'));
  assert.throws(
    () => planUpdate({ repoPath: dir }),
    (err) => err.code === 'UPDATE_NOT_A_CLONE',
  );
});

test('update plan: a non-official remote is refused', () => {
  const { work1 } = setupFixture();
  sh(work1, ['remote', 'set-url', 'origin', 'https://github.com/someone-else/bukio-cli.git']);
  assert.throws(
    () => planUpdate({ repoPath: work1 }),
    (err) => err.code === 'UPDATE_WRONG_REMOTE',
  );
});

test('update plan: shows the incoming commit and current version without warning', () => {
  const { work1 } = setupFixture();
  const plan = planUpdate({ repoPath: work1, verifyRemote: false });
  assert.equal(plan.current_version, '1.0.0');
  assert.equal(plan.incoming_count, 1);
  assert.match(plan.incoming[0], /v2/);
  assert.equal(plan.local_commits.length, 0);
  assert.equal(plan.modified_files.length, 0);
  assert.equal(plan.warning, null);
  assert.equal(plan.up_to_date, false);
});

test('update plan: local modifications are reported as overwrite warnings', () => {
  const { work1 } = setupFixture();
  writeFileSync(path.join(work1, 'README.md'), 'my customization\n');
  const plan = planUpdate({ repoPath: work1, verifyRemote: false });
  assert.deepEqual(plan.modified_files, ['README.md']);
  assert.match(plan.warning, /OVERWRITES LOCAL CUSTOMIZATIONS/);
  assert.match(plan.warning, /1 modified file/);
});

test('update: refuses to run without --yes', () => {
  const { work1 } = setupFixture();
  assert.throws(
    () => runUpdate({ repoPath: work1, yes: false, verifyRemote: false }),
    (err) => err.code === 'UPDATE_CONFIRM_REQUIRED',
  );
});

test('update: --yes resets the working tree to origin/main', () => {
  const { work1 } = setupFixture();
  const result = runUpdate({ repoPath: work1, yes: true, actor: 'agent:test', verifyRemote: false });
  assert.equal(result.updated, true);
  assert.equal(result.commits_applied, 1);
  assert.equal(result.version_after, '1.0.0'); // package.json unchanged in v2
  assert.equal(sh(work1, ['rev-parse', 'HEAD']), sh(work1, ['rev-parse', 'origin/main']));
  assert.equal(readFileSync(path.join(work1, 'README.md'), 'utf8'), 'v2 content\n');
  // now up to date
  const plan = planUpdate({ repoPath: work1, verifyRemote: false });
  assert.equal(plan.up_to_date, true);
});

test('update: --yes overwrites a local customization (tracked modification)', () => {
  const { work1 } = setupFixture();
  writeFileSync(path.join(work1, 'README.md'), 'my customization\n');
  const plan = planUpdate({ repoPath: work1, verifyRemote: false });
  assert.equal(plan.modified_files.length, 1);
  const result = runUpdate({ repoPath: work1, yes: true, actor: 'agent:test', verifyRemote: false });
  assert.equal(result.updated, true);
  assert.equal(readFileSync(path.join(work1, 'README.md'), 'utf8'), 'v2 content\n'); // customization gone
});

test('update: --yes drops local commits (warned in the plan)', () => {
  const { work1 } = setupFixture();
  writeFileSync(path.join(work1, 'README.md'), 'local commit content\n');
  sh(work1, ['add', '.']);
  sh(work1, ['commit', '-m', 'local change']);
  const plan = planUpdate({ repoPath: work1, verifyRemote: false });
  assert.equal(plan.local_commits.length, 1);
  assert.equal(plan.incoming_count, 1); // remote has v2 too — the reset still rewinds the local commit
  const result = runUpdate({ repoPath: work1, yes: true, actor: 'agent:test', verifyRemote: false });
  assert.equal(result.updated, true);
  assert.equal(readFileSync(path.join(work1, 'README.md'), 'utf8'), 'v2 content\n'); // local commit gone, now origin/main state
  assert.equal(sh(work1, ['rev-parse', 'HEAD']), sh(work1, ['rev-parse', 'origin/main']));
});

test('update: reinstalls dependencies when package.json changed', () => {
  const { work1 } = setupFixture([['package.json', '{"name":"bukio-cli","version":"1.0.1"}\n']]);
  const plan = planUpdate({ repoPath: work1, verifyRemote: false });
  assert.equal(plan.package_json_changed, true);
  const result = runUpdate({ repoPath: work1, yes: true, actor: 'agent:test', verifyRemote: false });
  assert.equal(result.version_after, '1.0.1');
  assert.equal(result.deps_installed, true);
  assert.equal(result.deps_error, null);
});

test('update: records an audit row when a company db exists', () => {
  const { work1 } = setupFixture();
  const db = openDb(':memory:'); // migrations create audit_log
  const result = runUpdate({ repoPath: work1, yes: true, db, actor: 'agent:test', verifyRemote: false });
  const rows = db.prepare("SELECT action, actor, args_json FROM audit_log WHERE action = 'update'").all();
  assert.equal(rows.length, 1);
  assert.equal(rows[0].actor, 'agent:test');
  assert.equal(JSON.parse(rows[0].args_json).commits, result.commits_applied);
  db.close();
});
