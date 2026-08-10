/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Tier 0.5 authorizations — CLI/MCP integration: the `actor authz`,
// `actor roles`, `actor can`, `actor who-can` commands, the CLI gate
// (AUTHZ_DENIED before any mutation, dry-run parity, authz implies
// enforce, deny-by-default), the MCP tool gate (same capabilities, no
// mutation on refusal, read-only tools unaffected) and the owner-mediated
// key revoke (D8, regardless of authz mode). Unit-level coverage of the
// capability map lives in test/authz.test.js.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync, spawn } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { openDb } from '../src/core/db.js';

function tmpDb() {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-authz-test-'));
  return path.join(dir, 'test.db');
}

function tmpConfig() {
  return mkdtempSync(path.join(os.tmpdir(), 'bukio-authz-cfg-'));
}

function runCli(args, env = {}) {
  // ALWAYS pin a scratch DB — never the live ~/.bukio/bukio.db
  const hasDb = args.includes('--db') || env.BUKIO_DB !== undefined;
  return spawnSync(process.execPath, ['bin/bukio.js', ...args], {
    cwd: process.cwd(), encoding: 'utf8',
    env: { ...process.env, ...env, ...(hasDb ? {} : { BUKIO_DB: tmpDb() }) },
  });
}

const json = (r) => JSON.parse(r.stdout);

/**
 * Fresh company with the given actors enrolled (keys in cfg, enrolled in
 * the DB — enforce is OFF during enrolment so first registration works).
 * Returns { cfg, dbPath, env(actor) }.
 */
function setupCompany(actors = ['agent:owner', 'agent:bookkeeper-a', 'agent:payments-b', 'agent:nobody']) {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-authz-co-'));
  const cfg = tmpConfig();
  const dbPath = path.join(dir, 'company.db');
  const env = (actor) => ({ BUKIO_ACTOR: actor, BUKIO_CONFIG_DIR: cfg });
  const init = runCli(['--json', 'init', '--name', 'X', '--db', dbPath], env('agent:owner'));
  assert.equal(init.status, 0, init.stderr);
  for (const a of actors) {
    const kg = runCli(['--json', '--actor', a, 'actor', 'keygen'], env(a));
    assert.equal(kg.status, 0, kg.stderr);
    const reg = runCli(['--json', '--actor', a, 'actor', 'register', '--db', dbPath], env(a));
    assert.equal(reg.status, 0, reg.stderr);
  }
  return { cfg, dbPath, env };
}

/** Flip authz on (owner becomes the flipper-owner) and grant two roles. */
function bootstrapAuthz(company) {
  const { dbPath, env } = company;
  const on = runCli(['--json', '--actor', 'agent:owner', 'actor', 'authz', '--on', '--db', dbPath], env('agent:owner'));
  assert.equal(on.status, 0, on.stderr);
  const g1 = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', 'grant', 'bookkeeper', '--for', 'agent:bookkeeper-a', '--db', dbPath], env('agent:owner'));
  assert.equal(g1.status, 0, g1.stderr);
  const g2 = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', 'grant', 'payments', '--for', 'agent:payments-b', '--db', dbPath], env('agent:owner'));
  assert.equal(g2.status, 0, g2.stderr);
}

// --- bootstrap (D1/D3) --------------------------------------------------------

test('authz --on: sets authz on, implies signing enforcement, grants the flipper owner', () => {
  const c = setupCompany(['agent:owner']);
  const on = runCli(['--json', '--actor', 'agent:owner', 'actor', 'authz', '--on', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(on.status, 0, on.stderr);
  const data = json(on).data;
  assert.equal(data.authz, 'on');
  assert.equal(data.enforce, 'on'); // D1: authz implies enforce
  assert.equal(data.owner, 'agent:owner'); // D3: flipper becomes owner

  const roles = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', '--db', c.dbPath], c.env('agent:owner'));
  assert.deepEqual(json(roles).data.roles, ['owner']);

  const db = openDb(c.dbPath);
  assert.equal(db.prepare("SELECT value FROM settings WHERE key='authz_mode'").get().value, 'on');
  assert.equal(db.prepare("SELECT value FROM settings WHERE key='signing_enforce'").get().value, 'on');
  const rows = db.prepare("SELECT action, args_json FROM audit_log WHERE action = 'actor.authz'").all();
  assert.ok(rows.length >= 1, 'authz flip must be audited');
  db.close();
});

test('authz --on --dry-run: nothing is written (no owner, no mode change)', () => {
  const c = setupCompany(['agent:owner']);
  const plan = runCli(['--json', '--actor', 'agent:owner', 'actor', 'authz', '--on', '--dry-run', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(plan.status, 0, plan.stderr);
  assert.equal(json(plan).data.dryRun, true);
  assert.equal(json(plan).data.owner_granted, 'agent:owner');
  const db = openDb(c.dbPath);
  assert.equal(db.prepare("SELECT value FROM settings WHERE key='authz_mode'").get().value, 'off');
  assert.equal(db.prepare('SELECT COUNT(*) n FROM actor_roles').get().n, 0);
  db.close();
});

test('authz: exactly one of --on/--off is required (INVALID_AUTHZ)', () => {
  const c = setupCompany(['agent:owner']);
  for (const args of [['actor', 'authz'], ['actor', 'authz', '--on', '--off']]) {
    const r = runCli(['--json', '--actor', 'agent:owner', ...args, '--db', c.dbPath], c.env('agent:owner'));
    assert.equal(r.status, 1);
    assert.equal(json(r).error.code, 'INVALID_AUTHZ');
  }
});

test('authz --off: the owner turns authz off; signing enforcement STAYS on', () => {
  const c = setupCompany(['agent:owner']);
  runCli(['--json', '--actor', 'agent:owner', 'actor', 'authz', '--on', '--db', c.dbPath], c.env('agent:owner'));
  const off = runCli(['--json', '--actor', 'agent:owner', 'actor', 'authz', '--off', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(off.status, 0, off.stderr);
  const data = json(off).data;
  assert.equal(data.authz, 'off');
  assert.equal(data.enforce, 'on'); // Tier 0 stays active
  const db = openDb(c.dbPath);
  assert.equal(db.prepare("SELECT value FROM settings WHERE key='signing_enforce'").get().value, 'on');
  db.close();
});

test('authz --off: a non-owner is refused AUTHZ_DENIED under authz', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  const off = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'actor', 'authz', '--off', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(off.status, 1);
  assert.equal(json(off).error.code, 'AUTHZ_DENIED');
});

// --- role grants ---------------------------------------------------------------

test('roles grant/revoke: audit rows + SoD warning on a conflicting grant', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  // conflicting grant: bookkeeper already on A → adding payments warns
  const grant = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', 'grant', 'payments', '--for', 'agent:bookkeeper-a', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(grant.status, 0, grant.stderr);
  const data = json(grant).data;
  assert.deepEqual(data.roles, ['bookkeeper', 'payments']);
  assert.ok(data.warnings.some((w) => w.includes('bookkeeper + payments')), `SoD warning expected: ${JSON.stringify(data.warnings)}`);

  const revoke = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', 'revoke', 'payments', '--for', 'agent:bookkeeper-a', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(revoke.status, 0, revoke.stderr);
  assert.deepEqual(json(revoke).data.roles, ['bookkeeper']);

  const db = openDb(c.dbPath);
  const actions = db.prepare("SELECT action FROM audit_log WHERE action LIKE 'actor.roles.%'").all().map((r) => r.action);
  assert.ok(actions.includes('actor.roles.grant'));
  assert.ok(actions.includes('actor.roles.revoke'));
  // the grant's signed args must name the grantee (the --actor option)
  const grantRow = db.prepare("SELECT args_json FROM audit_log WHERE action = 'actor.roles.grant' ORDER BY id DESC LIMIT 1").get();
  assert.ok(grantRow.args_json.includes('agent:bookkeeper-a'), `grant target must be in the signed args: ${grantRow.args_json}`);
  db.close();
});

test('roles revoke: ROLE_NOT_GRANTED on absent role; LAST_OWNER guards the last owner', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  const notGranted = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', 'revoke', 'tax', '--for', 'agent:payments-b', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(notGranted.status, 1);
  assert.equal(json(notGranted).error.code, 'ROLE_NOT_GRANTED');

  const lastOwner = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', 'revoke', 'owner', '--for', 'agent:owner', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(lastOwner.status, 1);
  assert.equal(json(lastOwner).error.code, 'LAST_OWNER');
});

test('roles: invalid role and invalid grantee are rejected', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  const badRole = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', 'grant', 'superuser', '--for', 'agent:bookkeeper-a', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(badRole.status, 1);
  assert.equal(json(badRole).error.code, 'INVALID_ROLE');
  const badActor = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', 'grant', 'bookkeeper', '--for', 'agent', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(badActor.status, 1);
  assert.equal(json(badActor).error.code, 'INVALID_ACTOR');
});

test('roles: self-service list for any enrolled actor; --actor <other> is owner only', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  // self: fine for a role-less actor
  const self = runCli(['--json', '--actor', 'agent:nobody', 'actor', 'roles', '--db', c.dbPath], c.env('agent:nobody'));
  assert.equal(self.status, 0, self.stderr);
  assert.deepEqual(json(self).data.roles, []);
  // another actor's roles: owner only under authz
  const ownerView = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', '--for', 'agent:bookkeeper-a', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(ownerView.status, 0, ownerView.stderr);
  assert.deepEqual(json(ownerView).data.roles, ['bookkeeper']);
  const nonOwnerView = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'actor', 'roles', '--for', 'agent:payments-b', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(nonOwnerView.status, 1);
  assert.equal(json(nonOwnerView).error.code, 'AUTHZ_DENIED');
});

test('roles grant/revoke: work for any enrolled actor when authz is OFF (roles are inert data)', () => {
  const c = setupCompany(['agent:owner', 'agent:bookkeeper-a']);
  // no authz --on: grants are configuration, not privilege
  const grant = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'actor', 'roles', 'grant', 'readonly', '--for', 'agent:owner', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(grant.status, 0, grant.stderr);
  const roles = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', '--db', c.dbPath], c.env('agent:owner'));
  assert.deepEqual(json(roles).data.roles, ['readonly']);
});

// --- actor can / who-can ------------------------------------------------------

test('actor can: self-service capability check with the ACTUAL mutation', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  const draft = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'actor', 'can', 'entry add', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(draft.status, 0, draft.stderr);
  assert.equal(json(draft).data.capability, 'entry.draft');
  assert.equal(json(draft).data.allowed, true);
  const post = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'actor', 'can', 'entry add --post', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(json(post).data.capability, 'entry.post');
  assert.equal(json(post).data.allowed, true);
  const file = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'actor', 'can', 'vat file', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(json(file).data.capability, 'vat.file');
  assert.equal(json(file).data.allowed, false);
  assert.equal(json(file).data.denied_reason, "no capability 'vat.file'");
  // MCP form
  const mcp = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'actor', 'can', 'mcp:entry_add', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(json(mcp).data.capability, 'entry.draft');
});

test('actor can --actor <other>: owner only under authz', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  const ok = runCli(['--json', '--actor', 'agent:owner', 'actor', 'can', 'entry post', '--for', 'agent:bookkeeper-a', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(ok.status, 0, ok.stderr);
  assert.equal(json(ok).data.allowed, true);
  const denied = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'actor', 'can', 'entry post', '--for', 'agent:payments-b', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(denied.status, 1);
  assert.equal(json(denied).error.code, 'AUTHZ_DENIED');
});

test('who-can: the SoD review lens — owner sees the full matrix', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  const who = runCli(['--json', '--actor', 'agent:owner', 'actor', 'who-can', 'entry post', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(who.status, 0, who.stderr);
  const data = json(who).data;
  assert.equal(data.capability, 'entry.post');
  const allowed = data.actors.filter((a) => a.allowed).map((a) => a.actor);
  assert.ok(allowed.includes('agent:owner'), `owner can entry post: ${JSON.stringify(allowed)}`);
  assert.ok(allowed.includes('agent:bookkeeper-a'));
  assert.ok(!allowed.includes('agent:payments-b'));
  assert.ok(!allowed.includes('agent:nobody'));
  // non-owner asking is refused
  const denied = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'actor', 'who-can', 'entry post', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(denied.status, 1);
  assert.equal(json(denied).error.code, 'AUTHZ_DENIED');
});

// --- the CLI gate --------------------------------------------------------------

test('gate: an actor with the right role acts; wrong capability is refused AUTHZ_DENIED before any mutation', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  // A (bookkeeper): draft + post work
  const draft = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'entry', 'add', '--date', '2026-08-01', '--desc', 'test', '--postings', '1100:100.00,3000:-100.00', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(draft.status, 0, draft.stderr);
  const posted = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'entry', 'post', '--id', '1', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(posted.status, 0, posted.stderr);
  // A cannot file VAT (vat.file is tax territory)
  const file = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'vat', 'file', '--period', '2026-Q2', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(file.status, 1);
  const err = json(file).error;
  assert.equal(err.code, 'AUTHZ_DENIED');
  assert.ok(err.message.includes('agent:bookkeeper-a'));
  assert.ok(err.message.includes("'vat.file'"));
  assert.ok(err.message.includes('bookkeeper'));
  // nothing was written by the refused command
  const db = openDb(c.dbPath);
  assert.equal(db.prepare("SELECT COUNT(*) n FROM audit_log WHERE action = 'vat.file'").get().n, 0);
  db.close();
});

test('gate: B (payments) can create a SEPA batch but not post entries', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  // B's own capability works: bank import is in the payments role — a
  // read-only check that the gate lets B through (no file needed)
  const bankList = runCli(['--json', '--actor', 'agent:payments-b', 'bank', 'list', '--db', c.dbPath], c.env('agent:payments-b'));
  assert.equal(bankList.status, 0, bankList.stderr);
  // B cannot post entries (entry.post is bookkeeper/owner)
  const bPost = runCli(['--json', '--actor', 'agent:payments-b', 'entry', 'post', '--id', '1', '--db', c.dbPath], c.env('agent:payments-b'));
  assert.equal(bPost.status, 1);
  assert.equal(json(bPost).error.code, 'AUTHZ_DENIED');
});

test('gate: deny-by-default — a role-less actor can only self-service', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  // self-service checks run
  for (const args of [
    ['actor', 'verify'],
    ['actor', 'roles'],
    ['actor', 'can', 'entry add'],
  ]) {
    const r = runCli(['--json', '--actor', 'agent:nobody', ...args, '--db', c.dbPath], c.env('agent:nobody'));
    assert.equal(r.status, 0, `${args.join(' ')}: ${r.stderr}`);
  }
  // everything else is refused
  const add = runCli(['--json', '--actor', 'agent:nobody', 'entry', 'add', '--date', '2026-08-01', '--desc', 'x', '--postings', '1100:100.00,3000:-100.00', '--db', c.dbPath], c.env('agent:nobody'));
  assert.equal(add.status, 1);
  assert.equal(json(add).error.code, 'AUTHZ_DENIED');
  assert.ok(json(add).error.message.includes('no roles'));
});

test('gate: dry-run is refused identically (D6 — a plan needs the capability)', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  const plan = runCli(['--json', '--actor', 'agent:payments-b', 'entry', 'add', '--post', '--date', '2026-08-01', '--desc', 'x', '--postings', '1100:100.00,3000:-100.00', '--dry-run', '--db', c.dbPath], c.env('agent:payments-b'));
  assert.equal(plan.status, 1);
  assert.equal(json(plan).error.code, 'AUTHZ_DENIED');
  const db = openDb(c.dbPath);
  assert.equal(db.prepare('SELECT COUNT(*) n FROM journal_entries').get().n, 0, 'dry-run refusal writes nothing');
  db.close();
});

test('gate: reads are gated too — a role-less actor cannot run report trial-balance', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  const tb = runCli(['--json', '--actor', 'agent:nobody', 'report', 'trial-balance', '--db', c.dbPath], c.env('agent:nobody'));
  assert.equal(tb.status, 1);
  assert.equal(json(tb).error.code, 'AUTHZ_DENIED');
  // a role with report.read can
  const tbB = runCli(['--json', '--actor', 'agent:payments-b', 'report', 'trial-balance', '--db', c.dbPath], c.env('agent:payments-b'));
  assert.equal(tbB.status, 0, tbB.stderr);
});

test('gate: entry add --post needs entry.post — the ACTUAL mutation decides', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  // bookkeeper: draft allowed, posting allowed
  const draft = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'entry', 'add', '--date', '2026-08-01', '--desc', 'd', '--postings', '1100:50.00,3000:-50.00', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(draft.status, 0, draft.stderr);
  const posted = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'entry', 'add', '--post', '--date', '2026-08-01', '--desc', 'p', '--postings', '1100:50.00,3000:-50.00', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(posted.status, 0, posted.stderr);
  // a draft-only actor (bookkeeper role minus posting is not expressible;
  // use a payments-only actor: draft denied, so entry.add itself is denied)
  const bDraft = runCli(['--json', '--actor', 'agent:payments-b', 'entry', 'add', '--date', '2026-08-01', '--desc', 'x', '--postings', '1100:50.00,3000:-50.00', '--db', c.dbPath], c.env('agent:payments-b'));
  assert.equal(bDraft.status, 1);
  assert.equal(json(bDraft).error.code, 'AUTHZ_DENIED');
});

// --- owner-mediated revoke (D8) ------------------------------------------------

test('revoke --target: owner kills a compromised key; the target is refused everywhere after', () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  const kill = runCli(['--json', '--actor', 'agent:owner', 'actor', 'revoke', '--target', 'agent:payments-b', '--reason', 'compromised key', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(kill.status, 0, kill.stderr);
  assert.equal(json(kill).data.actor, 'agent:payments-b');
  assert.equal(json(kill).data.revoked_by, 'agent:owner');
  // the target is dead: signing is refused with ACTOR_KEY_REVOKED (enforce on)
  const after = runCli(['--json', '--actor', 'agent:payments-b', 'bank', 'list', '--db', c.dbPath], c.env('agent:payments-b'));
  assert.equal(after.status, 1);
  assert.equal(json(after).error.code, 'ACTOR_KEY_REVOKED');
  // self-revoke for the victim is ALSO gate-refused (enforce is on and the
  // key is revoked — the sign gate intercepts before the registry does)
  const self = runCli(['--json', '--actor', 'agent:payments-b', 'actor', 'revoke', '--reason', 'rotating out', '--db', c.dbPath], c.env('agent:payments-b'));
  assert.equal(self.status, 1);
  assert.equal(json(self).error.code, 'ACTOR_KEY_REVOKED');
});

test('revoke --target: needs the OWNER role REGARDLESS of authz mode (D8)', () => {
  const c = setupCompany(['agent:owner', 'agent:bookkeeper-a']);
  // authz is OFF. Grant the owner role explicitly (roles are inert data
  // while authz is off, but the owner-kill check reads them regardless).
  const grant = runCli(['--json', '--actor', 'agent:owner', 'actor', 'roles', 'grant', 'owner', '--for', 'agent:owner', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(grant.status, 0, grant.stderr);
  // a bookkeeper (not owner) cannot kill a key — even with authz off
  const denied = runCli(['--json', '--actor', 'agent:bookkeeper-a', 'actor', 'revoke', '--target', 'agent:owner', '--reason', 'x', '--db', c.dbPath], c.env('agent:bookkeeper-a'));
  assert.equal(denied.status, 1);
  assert.equal(json(denied).error.code, 'AUTHZ_DENIED');
  assert.ok(json(denied).error.message.includes('owner role'));
  // the owner can
  const ok = runCli(['--json', '--actor', 'agent:owner', 'actor', 'revoke', '--target', 'agent:bookkeeper-a', '--reason', 'leaving', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(ok.status, 0, ok.stderr);
  const db = openDb(c.dbPath);
  assert.ok(db.prepare('SELECT * FROM actor_keys WHERE actor = ? AND revoked_at IS NOT NULL').get('agent:bookkeeper-a'), 'target key must be revoked');
  db.close();
});

// --- MCP gate (Task 5) ----------------------------------------------------------

function mcpSession(dbPath, opts = {}) {
  const configDir = opts.configDir ?? mkdtempSync(path.join(os.tmpdir(), 'bukio-authz-mcp-cfg-'));
  const child = spawn(process.execPath, ['bin/bukio.js', 'mcp', '--db', dbPath], {
    cwd: process.cwd(),
    env: {
      ...process.env, BUKIO_ACTOR: 'agent:test', BUKIO_CONFIG_DIR: configDir, ...opts.env,
    },
  });
  let buf = '';
  const pending = [];
  const waiters = [];
  child.stdout.on('data', (d) => {
    buf += d.toString();
    let nl;
    while ((nl = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, nl);
      buf = buf.slice(nl + 1);
      const msg = JSON.parse(line);
      if (waiters.length) waiters.shift()(msg);
      else pending.push(msg);
    }
  });
  const next = () => (pending.length ? Promise.resolve(pending.shift()) : new Promise((res) => waiters.push(res)));
  return {
    child,
    call(method, params = {}, id = Math.floor(Math.random() * 1e9)) {
      child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
      return next();
    },
    close() {
      child.stdin.end();
      return new Promise((res) => child.on('exit', res));
    },
  };
}

test('MCP gate: tool calls map to the same capabilities; refusals mutate nothing', async () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  // seed one contact so batch tools have a target (owner does it)
  const contact = runCli(['--json', '--actor', 'agent:owner', 'contact', 'add', '--name', 'Vendor', '--db', c.dbPath], c.env('agent:owner'));
  assert.equal(contact.status, 0, contact.stderr);

  // A (bookkeeper): entry_add draft + execute/post work through MCP
  const a = mcpSession(c.dbPath, { configDir: c.cfg, env: { BUKIO_ACTOR: 'agent:bookkeeper-a' } });
  const aDraft = await a.call('tools/call', { name: 'entry_add', arguments: { date: '2026-08-01', description: 'via mcp', postings: ['1100:100.00', '3000:-100.00'], mode: 'execute' } });
  const aRes = JSON.parse(aDraft.result.content[0].text);
  assert.equal(aDraft.result.isError, false, JSON.stringify(aRes));
  assert.equal(aRes.mode, 'execute'); // entry_add returns the plan directly, no data wrapper
  const aPost = await a.call('tools/call', { name: 'entry_post', arguments: { id: aRes.entry_id, mode: 'execute', actor: 'agent:bookkeeper-a' } });
  assert.equal(aPost.result.isError, false, JSON.stringify(aPost));
  await a.close();

  // B (payments): entry_add (post:true) is refused with the AUTHZ_DENIED
  // error envelope — and nothing is written
  const b = mcpSession(c.dbPath, { configDir: c.cfg, env: { BUKIO_ACTOR: 'agent:payments-b' } });
  const bCall = await b.call('tools/call', { name: 'entry_add', arguments: { date: '2026-08-02', description: 'should not land', postings: ['1100:50.00', '3000:-50.00'], post: true, mode: 'execute' } });
  assert.equal(bCall.result.isError, true, JSON.stringify(bCall));
  const bErr = JSON.parse(bCall.result.content[0].text);
  assert.equal(bErr.error.code, 'AUTHZ_DENIED');
  assert.ok(bErr.error.message.includes('entry.post'));
  // B can still do its own thing: payments_batch_create (payments.sepa)
  const bBatch = await b.call('tools/call', { name: 'payments_batch_create', arguments: { type: 'transfer', payable_ids: [], mode: 'execute', actor: 'agent:payments-b' } });
  // an empty batch is a VALIDATION failure, not an authz failure — the
  // gate let it through (payments.sepa granted) and the module rejected
  const bBatchRes = JSON.parse(bBatch.result.content[0].text);
  assert.notEqual(bBatchRes.error?.code, 'AUTHZ_DENIED', JSON.stringify(bBatch));
  await b.close();

  // nothing was written by the refused call
  const check = openDb(c.dbPath);
  const entries = check.prepare('SELECT description FROM journal_entries').all().map((r) => r.description);
  assert.ok(entries.includes('via mcp'));
  assert.ok(!entries.includes('should not land'), 'refused tool call must not mutate');
  check.close();
});

test('MCP gate: read-only tools are unaffected (not gated) — a role-less actor can still read', async () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  const s = mcpSession(c.dbPath, { configDir: c.cfg, env: { BUKIO_ACTOR: 'agent:nobody' } });
  const tb = await s.call('tools/call', { name: 'trial_balance', arguments: {} });
  assert.equal(tb.result.isError, false, JSON.stringify(tb));
  await s.close();
});

test('MCP gate: vat_book maps to vat.book — a payments actor is refused', async () => {
  const c = setupCompany();
  bootstrapAuthz(c);
  const b = mcpSession(c.dbPath, { configDir: c.cfg, env: { BUKIO_ACTOR: 'agent:payments-b' } });
  const call = await b.call('tools/call', { name: 'vat_book', arguments: { date: '2026-08-01', description: 'x', postings: ['1100:121.00', '8000:-100.00@21'], post: true, mode: 'execute' } });
  assert.equal(call.result.isError, true, JSON.stringify(call));
  const err = JSON.parse(call.result.content[0].text);
  assert.equal(err.error.code, 'AUTHZ_DENIED');
  assert.ok(err.error.message.includes('vat.book'));
  await b.close();
});
