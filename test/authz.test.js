/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Tier 0.5 authorizations — capability map, command→capability coverage,
// canAct, SoD warnings, exemption set and the authz gate (unit level).
// The CLI/MCP gate behaviour lives in test/authz-cli.test.js.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { openDb } from '../src/core/db.js';
import {
  capabilityOf, canAct, sodWarnings, isAuthzExempt, checkAuthz, ROLES, ROLE_CAPABILITIES,
  CAPABILITY_DESCRIPTIONS, SOD_PAIRS, SOD_CAPABILITY_PAIR,
} from '../src/core/authz.js';
import { grantRole } from '../src/core/actor-registry.js';

/** Open a fresh in-memory company DB. */
function freshDb() {
  const db = openDb(':memory:');
  db.prepare("INSERT INTO company (id, name) VALUES (1, 'X')").run();
  return db;
}

/** Turn authz on for a fresh DB (migration 020 pre-seeds the row as 'off'). */
function turnAuthzOn(db) {
  db.prepare("UPDATE settings SET value = 'on' WHERE key = 'authz_mode'").run();
}

/** Every real CLI command path (from the src/cli/*.js command tree). */
const CLI_PATHS = [
  'init', 'account add', 'account list', 'account show', 'account deactivate',
  'account reactivate', 'account import',
  'actor keygen', 'actor register', 'actor list', 'actor revoke', 'actor enforce',
  'actor unlock', 'actor lock', 'actor verify',
  'assets scheme add', 'assets scheme list', 'assets add', 'assets list',
  'assets show', 'assets run', 'assets register', 'assets dispose',
  'assets pause', 'assets resume',
  'attach add', 'attach list', 'attach show', 'attach remove',
  'audit', 'audit verify', 'backup', 'restore',
  'bank add', 'bank list', 'bank import', 'bank transactions',
  'bank match auto', 'bank match suggest', 'bank match link', 'bank match post',
  'bank ignore', 'bank unignore',
  'company show', 'company update', 'company logo',
  'compliance status', 'compliance mark',
  'contact add', 'contact update', 'contact list', 'contact statement',
  'entry add', 'entry post', 'entry reverse', 'entry list', 'entry show',
  'export xaf', 'fx fetch', 'fx set', 'fx show', 'fx list',
  'icp readout',
  'import opening-balances', 'import journal', 'import contacts', 'import xaf', 'import invoice',
  'invoice create', 'invoice finalize', 'invoice list', 'invoice show',
  'invoice pdf', 'invoice ubl', 'invoice credit', 'invoice peppol-send',
  'invoice pay', 'invoice email', 'invoice reminders',
  'item add', 'item list', 'item show', 'item update',
  'month-end',
  'payments payables add', 'payments payables list', 'payments payables pay',
  'payments mandate add', 'payments mandate list', 'payments mandate remove',
  'payments batch create', 'payments batch list', 'payments batch show',
  'payments batch delete', 'payments batch export',
  'recurring add', 'recurring list', 'recurring show', 'recurring pause',
  'recurring resume', 'recurring preview', 'recurring run',
  'report trial-balance', 'report balance-sheet', 'report pnl', 'report journal',
  'report aging', 'report sales',
  'update', 'vat enable', 'vat codes', 'vat book', 'vat readout', 'vat file', 'vat settle',
  'year-end close', 'year-end status', 'year-end report', 'jaarrekening report', 'mcp',
  'server start', 'server token',
];

// --- capability mapping ------------------------------------------------------

test('capabilityOf: entry add resolves by the ACTUAL mutation (--post)', () => {
  assert.equal(capabilityOf('entry add', {}), 'entry.draft');
  assert.equal(capabilityOf('entry add', { post: false }), 'entry.draft');
  assert.equal(capabilityOf('entry add', { post: true }), 'entry.post');
  // MCP parity
  assert.equal(capabilityOf('mcp:entry_add', {}), 'entry.draft');
  assert.equal(capabilityOf('mcp:entry_add', { post: true }), 'entry.post');
});

test('capabilityOf: every documented §3 command maps to exactly one capability', () => {
  const agents = readFileSync(path.join(process.cwd(), 'AGENTS.md'), 'utf8');
  const section = agents.split('## 3. Command quick reference')[1].split('## 4.')[0];
  const spans = [...section.matchAll(/`bukio ([^`]+)`/g)].map((m) => m[1]);
  assert.ok(spans.length > 40, `expected a substantial §3 surface, got ${spans.length}`);
  // every real command path (test below) — used to recognise compact
  // family rows like 'account add/list/show' → parent prefix 'account'
  const realPaths = CLI_PATHS;
  const seen = new Set();
  const unmapped = [];
  for (const span of spans) {
    const tokens = span.split(/\s+/);
    if (tokens[0] === 'bukio') tokens.shift();
    // strip flags + placeholders: 'entry add --date ...' → 'entry add';
    // a '<...>' token (e.g. '<cmd>') is a placeholder row, '[' starts
    // an option group, '/' is compact family notation ('account
    // add/list/show') — all of those end the path
    const words = [];
    for (const t of tokens) {
      if (t.startsWith('-') || t.startsWith('[') || t.includes('<') || t.includes('/')) break;
      words.push(t);
    }
    const cmd = words.join(' ');
    if (!cmd || cmd === 'bukio') continue;
    if (seen.has(cmd)) continue;
    seen.add(cmd);
    if (isAuthzExempt(cmd)) continue; // exempt commands are not mapped by design
    if (capabilityOf(cmd)) continue;
    // a compact family row whose path is only a PREFIX of real commands
    // ('account' for 'account add/list/...') documents those members —
    // covered by the exhaustive path list
    if (realPaths.some((p) => p.startsWith(`${cmd} `))) continue;
    unmapped.push(cmd);
  }
  assert.deepEqual(unmapped, [], 'documented §3 commands without a capability mapping (unmapped = silent deny under authz)');
});

test('capabilityOf: every real CLI command path maps or is authz-exempt', () => {
  // Full surface from the CLI command tree (src/cli/*.js) — the §3 test
  // only sees first-paths of doc rows; this list is exhaustive.
  const allPaths = CLI_PATHS;
  const missing = [];
  for (const cmd of allPaths) {
    if (isAuthzExempt(cmd)) continue;
    if (!capabilityOf(cmd)) missing.push(cmd);
  }
  assert.deepEqual(missing, [], 'real CLI commands without a capability mapping');
});

test('capabilityOf: every MCP mutating tool maps to exactly one capability', () => {
  const mutatingTools = [
    'entry_add', 'entry_post', 'entry_reverse', 'vat_book',
    'invoice_create', 'invoice_finalize', 'invoice_credit', 'invoice_pay',
    'invoice_email', 'invoice_import', 'item_add', 'item_update',
    'attachment_add', 'attachment_remove',
    'payments_mandate_add', 'payments_batch_create', 'payments_batch_export',
    'recurring_run', 'year_end_close', 'fx_set', 'contact_add',
    'asset_add', 'assets_run', 'asset_dispose',
  ];
  const missing = [];
  for (const tool of mutatingTools) {
    const cap = capabilityOf(`mcp:${tool}`, {});
    if (!cap) missing.push(tool);
  }
  assert.deepEqual(missing, [], 'MCP mutating tools without a capability mapping');
});

test('capabilityOf: unmapped commands return null (fail closed)', () => {
  assert.equal(capabilityOf('totally made-up'), null);
  assert.equal(capabilityOf('mcp:made_up_tool'), null);
  assert.equal(capabilityOf(''), null);
});

// --- canAct ------------------------------------------------------------------

test('canAct: deny-by-default — no roles grants nothing', () => {
  const db = freshDb();
  assert.equal(canAct(db, 'agent:nobody', 'report.read'), false);
  assert.equal(canAct(db, 'agent:nobody', 'entry.draft'), false);
  db.close();
});

test('canAct: owner passes EVERYTHING; roles grant only their capabilities', () => {
  const db = freshDb();
  grantRole(db, { actor: 'human:erik', role: 'owner', grantedBy: 'human:erik' });
  grantRole(db, { actor: 'agent:invoicing', role: 'bookkeeper', grantedBy: 'human:erik' });
  grantRole(db, { actor: 'agent:pay', role: 'payments', grantedBy: 'human:erik' });
  grantRole(db, { actor: 'agent:auditor', role: 'readonly', grantedBy: 'human:erik' });

  assert.equal(canAct(db, 'human:erik', 'admin.actor'), true);
  assert.equal(canAct(db, 'human:erik', 'payments.sepa'), true);
  assert.equal(canAct(db, 'human:erik', 'report.read'), true);

  assert.equal(canAct(db, 'agent:invoicing', 'entry.post'), true);
  assert.equal(canAct(db, 'agent:invoicing', 'invoice.manage'), true);
  assert.equal(canAct(db, 'agent:invoicing', 'payments.sepa'), false);
  assert.equal(canAct(db, 'agent:invoicing', 'vat.file'), false);

  assert.equal(canAct(db, 'agent:pay', 'payments.sepa'), true);
  assert.equal(canAct(db, 'agent:pay', 'bank.import'), true);
  assert.equal(canAct(db, 'agent:pay', 'entry.post'), false);

  assert.equal(canAct(db, 'agent:auditor', 'report.read'), true);
  assert.equal(canAct(db, 'agent:auditor', 'entry.draft'), false);
  db.close();
});

test('canAct: fail closed — a null capability is never granted', () => {
  const db = freshDb();
  grantRole(db, { actor: 'human:erik', role: 'owner', grantedBy: 'human:erik' });
  assert.equal(canAct(db, 'human:erik', null), false);
  db.close();
});

test('role definitions are consistent: every listed capability is a real capability', () => {
  for (const [role, caps] of Object.entries(ROLE_CAPABILITIES)) {
    assert.ok(ROLES.includes(role), `unknown role ${role}`);
    if (caps === null) continue; // owner
    for (const c of caps) {
      assert.ok(CAPABILITY_DESCRIPTIONS[c], `role ${role} references unknown capability ${c}`);
    }
  }
});

// --- SoD warnings ------------------------------------------------------------

test('sodWarnings: the documented conflict pairs warn; clean sets stay quiet', () => {
  assert.deepEqual(sodWarnings(['bookkeeper']), []);
  assert.deepEqual(sodWarnings(['readonly']), []);
  assert.deepEqual(sodWarnings(['bookkeeper', 'payments']), [
    'bookkeeper + payments: the same actor books AND authorises money out',
    'entry.post + payments.sepa: the same actor moves the ledger AND authorises money out (strongest pair)',
  ]);
  assert.deepEqual(sodWarnings(['bookkeeper', 'tax']), [
    'bookkeeper + tax: the same actor books AND files tax',
  ]);
  assert.deepEqual(sodWarnings(['payments', 'tax']), [
    'payments + tax: the same actor authorises money out AND files tax',
  ]);
  // owner is exempt from the warning (the operator is allowed everything)
  assert.deepEqual(sodWarnings(['owner', 'bookkeeper', 'payments', 'tax']), []);
});

test('sodWarnings: every documented pair is real (map stays in sync)', () => {
  for (const pair of SOD_PAIRS) {
    assert.ok(pair.roles.every((r) => ROLES.includes(r)), `unknown role in ${pair.roles}`);
    assert.ok(pair.message.length > 10);
  }
  assert.ok(SOD_CAPABILITY_PAIR.capabilities.every((c) => CAPABILITY_DESCRIPTIONS[c]));
});

// --- exemption set -----------------------------------------------------------

test('isAuthzExempt: self-service + bootstrap commands are exempt; owner actions are not', () => {
  assert.equal(isAuthzExempt('actor keygen'), true);
  assert.equal(isAuthzExempt('actor unlock'), true);
  assert.equal(isAuthzExempt('actor lock'), true);
  assert.equal(isAuthzExempt('mcp'), true);
  assert.equal(isAuthzExempt('actor register'), true);
  assert.equal(isAuthzExempt('actor verify'), true);
  assert.equal(isAuthzExempt('actor roles', {}), true); // self
  assert.equal(isAuthzExempt('actor can', {}), true); // self
  assert.equal(isAuthzExempt('actor revoke', { reason: 'x' }), true); // self-revoke
  // the same commands aimed at OTHERS are owner territory (the "other
  // actor" option is `--for` — `--actor` is the root identity flag and
  // cannot be re-declared on a subcommand)
  assert.equal(isAuthzExempt('actor roles', { for: 'agent:other' }), false);
  assert.equal(isAuthzExempt('actor can', { for: 'agent:other' }), false);
  assert.equal(isAuthzExempt('actor revoke', { target: 'agent:other' }), false);
  // everything else is checked
  assert.equal(isAuthzExempt('entry add'), false);
  assert.equal(isAuthzExempt('report trial-balance'), false);
  assert.equal(isAuthzExempt('actor who-can'), false);
  assert.equal(isAuthzExempt('actor list'), false);
  assert.equal(isAuthzExempt('mcp:entry_add'), false);
});

// --- the gate ----------------------------------------------------------------

test('checkAuthz: authz off (default) → no refusals', () => {
  const db = freshDb();
  assert.doesNotThrow(() => checkAuthz(db, 'agent:nobody', 'entry add'));
  assert.doesNotThrow(() => checkAuthz(db, 'agent:nobody', 'actor roles grant'));
  db.close();
});

test('checkAuthz: unmapped command denies under authz (fail closed)', () => {
  const db = freshDb();
  turnAuthzOn(db);
  grantRole(db, { actor: 'human:erik', role: 'owner', grantedBy: 'human:erik' });
  assert.throws(() => checkAuthz(db, 'human:erik', 'made-up cmd'), { code: 'AUTHZ_DENIED' });
  db.close();
});

test('checkAuthz: AUTHZ_DENIED message names the actor, missing capability and roles', () => {
  const db = freshDb();
  turnAuthzOn(db);
  grantRole(db, { actor: 'agent:invoicing', role: 'bookkeeper', grantedBy: 'human:erik' });
  try {
    checkAuthz(db, 'agent:invoicing', 'vat file'); // vat.file — bookkeeper does NOT have it
    assert.fail('should have thrown');
  } catch (err) {
    assert.equal(err.code, 'AUTHZ_DENIED');
    assert.ok(err.message.includes('agent:invoicing'));
    assert.ok(err.message.includes("'vat.file'"));
    assert.ok(err.message.includes('bookkeeper'));
    assert.ok(err.message.includes('owner'));
  }
  db.close();
});

test('checkAuthz: dry-run is refused identically (capability required for the plan)', () => {
  const db = freshDb();
  turnAuthzOn(db);
  grantRole(db, { actor: 'agent:invoicing', role: 'bookkeeper', grantedBy: 'human:erik' });
  // entry add --post (dry-run or not) needs entry.post — bookkeeper has it
  assert.doesNotThrow(() => checkAuthz(db, 'agent:invoicing', 'entry add', { post: true }));
  // vat file needs vat.file — bookkeeper does NOT have it
  assert.throws(() => checkAuthz(db, 'agent:invoicing', 'vat file'), { code: 'AUTHZ_DENIED' });
  db.close();
});

test('checkAuthz: owner-mediated revoke needs the owner role REGARDLESS of authz mode (D8)', () => {
  const db = freshDb();
  grantRole(db, { actor: 'agent:invoicing', role: 'bookkeeper', grantedBy: 'human:erik' });
  // authz is OFF — but the owner-kill is still owner-only
  assert.throws(
    () => checkAuthz(db, 'agent:invoicing', 'actor revoke', { target: 'agent:other', reason: 'x' }),
    { code: 'AUTHZ_DENIED' },
  );
  grantRole(db, { actor: 'human:erik', role: 'owner', grantedBy: 'human:erik' });
  assert.doesNotThrow(() => checkAuthz(db, 'human:erik', 'actor revoke', { target: 'agent:other', reason: 'x' }));
  db.close();
});

test('checkAuthz: no DB → no authz state → never refuses', () => {
  assert.doesNotThrow(() => checkAuthz(null, 'agent:nobody', 'entry add', { post: true }));
  assert.doesNotThrow(() => checkAuthz(null, 'agent:nobody', 'actor revoke', { target: 'x', reason: 'y' }));
});
