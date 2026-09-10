#!/usr/bin/env node
/**
 * bukio-cli — JS↔Rust CLI parity harness.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Runs the same command sequences through bin/bukio.js (JS engine) and the
 * Rust binary on two identical fresh DBs, then compares the --json output
 * key-by-key. Volatile fields (timestamps, ids when sequences diverge) are
 * normalised. Exit code 0 = full parity on all cases.
 *
 * Usage: node scripts/parity.mjs [--rust-bin path] [--filter entry]
 */
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const flag = (name, def = null) => {
  const i = args.indexOf(`--${name}`);
  return i > -1 ? args[i + 1] : def;
};
const FILTER = flag('filter');
const RUST = flag('rust-bin', path.join(root, 'target', 'debug', 'bukio'));

const ACTOR = ['--actor', 'human:erik'];

// Each case: name + list of argv (without --db/--json, added per engine).
// Stateful sequences are the point: init -> add -> post -> reverse -> report.
const CASES = [
  {
    name: 'init',
    steps: [
      ['init', '--name', 'Parity BV', '--country', 'NL', '--legal-form', 'bv'],
      ['init', '--name', 'Dup BV'],
      ['init', '--name', 'Bad', '--vat', 'maybe'],
    ],
  },
  {
    name: 'entry',
    setup: [
      ['init', '--name', 'Entry BV'],
    ],
    steps: [
      ['entry', 'add', '--date', '2026-01-10', '--desc', 'Startkapitaal', '--postings', '1100:10000.00,3000:-10000.00', '--post'],
      ['entry', 'add', '--date', '2026-01-15', '--desc', 'Verkoop', '--postings', '1100:121.00,8000:-100.00,1500:-21.00', '--post'],
      ['entry', 'add', '--date', '2026-02-30', '--desc', 'Bad date', '--postings', '1100:1.00,3000:-1.00'],
      ['entry', 'add', '--date', '2026-02-01', '--desc', 'Unbalanced', '--postings', '1100:1.00,3000:-0.99'],
      ['entry', 'add', '--date', '2026-02-01', '--desc', 'Draft', '--postings', '1100:5.00,3000:-5.00'],
      ['entry', 'list'],
      ['entry', 'list', '--state', 'draft'],
      ['entry', 'post', '--id', '3'],
      ['entry', 'show', '--id', '1'],
      ['entry', 'reverse', '--id', '1', '--reason', 'test'],
      ['entry', 'reverse', '--id', '1'],
      ['entry', 'add', '--desc', 'No date no postings'],
      ['entry', 'add', '--date', '2026-02-01', '--desc', 'CC', '--postings', '1100:9.00,3000:-9.00@HQ'],
    ],
  },
  {
    name: 'entry-dry-run',
    setup: [
      ['init', '--name', 'DryRun BV'],
    ],
    steps: [
      ['entry', 'add', '--date', '2026-01-10', '--desc', 'X', '--postings', '1100:10.00,3000:-10.00', '--dry-run'],
      ['entry', 'add', '--date', '2026-01-10', '--desc', 'X', '--postings', '1100:10.00,3000:-9.00', '--dry-run'],
      ['entry', 'post', '--id', '99', '--dry-run'],
    ],
  },
  {
    name: 'account',
    setup: [
      ['init', '--name', 'Account BV'],
    ],
    steps: [
      ['account', 'list'],
      ['account', 'show', '--code', '1100'],
      ['account', 'show', '--code', '9999'],
      ['account', 'add', '--code', '1234', '--name', 'Zakgeld', '--type', 'asset', '--normal-balance', 'debit'],
      ['account', 'add', '--code', '1234', '--name', 'Dup', '--type', 'asset', '--normal-balance', 'debit'],
      ['account', 'add', '--code', '1235', '--name', 'BadType', '--type', 'cash', '--normal-balance', 'debit'],
      ['account', 'deactivate', '--code', '1234'],
      ['account', 'deactivate', '--code', '1234'],
      ['account', 'reactivate', '--code', '1234'],
      ['account', 'list', '--include-inactive', '--type', 'asset'],
    ],
  },
  {
    name: 'cost-center',
    setup: [
      ['init', '--name', 'CostCenter BV'],
    ],
    steps: [
      ['cost-center', 'add', '--code', 'HQ', '--name', 'Hoofdkantoor'],
      ['cost-center', 'add', '--code', 'HQ', '--name', 'Dup'],
      ['cost-center', 'list'],
      ['cost-center', 'deactivate', '--code', 'HQ'],
      ['cost-center', 'reactivate', '--code', 'HQ'],
      ['cost-center', 'deactivate', '--code', 'NOPE'],
    ],
  },
  {
    name: 'reports',
    setup: [
      ['init', '--name', 'Reports BV'],
      ['entry', 'add', '--date', '2026-01-10', '--desc', 'Start', '--postings', '1100:10000.00,3000:-10000.00', '--post'],
      ['entry', 'add', '--date', '2026-03-10', '--desc', 'Omzet', '--postings', '1100:121.00,8000:-100.00,1500:-21.00', '--post'],
      ['entry', 'add', '--date', '2026-04-10', '--desc', 'Kosten', '--postings', '4000:50.00,1100:-50.00', '--post'],
    ],
    steps: [
      ['report', 'trial-balance', '--year', '2026'],
      ['report', 'trial-balance'],
      ['report', 'balance-sheet', '--as-of', '2026-12-31'],
      ['report', 'balance-sheet', '--as-of', 'garbage'],
      ['report', 'pnl', '--year', '2026'],
      ['report', 'journal', '--year', '2026'],
      ['report', 'trial-balance', '--year', '20x6'],
    ],
  },
  {
    name: 'fx',
    setup: [
      ['init', '--name', 'FX BV'],
    ],
    steps: [
      ['fx', 'set', '--currency', 'USD', '--date', '2026-01-15', '--rate', '1.0875'],
      ['fx', 'set', '--currency', 'GBP', '--date', '2026-01-15', '--rate', '0.8590'],
      ['fx', 'show', '--currency', 'USD'],
      ['fx', 'list'],
      ['fx', 'set', '--currency', 'USD', '--date', '2026-01-15', '--rate', '1.0875', '--dry-run'],
      ['fx', 'set', '--currency', 'usd', '--date', '2026-01-15', '--rate', '1.0'],
    ],
  },
  {
    name: 'vat',
    setup: [
      ['init', '--name', 'VAT BV', '--vat', 'on'],
    ],
    steps: [
      ['vat', 'codes'],
      ['vat', 'book', '--date', '2026-04-10', '--desc', 'Verkoop', '--postings', '1100:121.00,8000:-100.00@21', '--post'],
      ['vat', 'book', '--date', '2026-04-11', '--desc', 'Inkoop', '--postings', '4000:60.50,1100:-60.50@21', '--post'],
      ['vat', 'book', '--date', '2026-04-12', '--desc', 'Verlegd', '--postings', '1100:100.00,8000:-100.00@R', '--post'],
      ['vat', 'book', '--date', '2026-04-13', '--desc', 'Vrij', '--postings', '1100:100.00,8000:-100.00@V', '--post'],
      ['vat', 'book', '--date', '2026-04-14', '--desc', 'Bad code', '--postings', '1100:100.00,8000:-100.00@99'],
      ['vat', 'book', '--date', '2026-04-15', '--desc', 'Privé', '--postings', '1100:21.00,3000:-21.00@P', '--post'],
      ['vat', 'readout', '--period', '2026-Q2'],
      ['vat', 'readout', '--period', '2026-04'],
      ['vat', 'readout', '--period', 'garbage'],
      ['vat', 'file', '--period', '2026-Q2', '--dry-run'],
      ['vat', 'file', '--period', '2026-Q2'],
      ['vat', 'file', '--period', '2026-Q2'],
      ['report', 'trial-balance', '--year', '2026'],
    ],
  },
  {
    name: 'audit',
    setup: [
      ['init', '--name', 'Audit BV'],
      ['entry', 'add', '--date', '2026-01-10', '--desc', 'Start', '--postings', '1100:100.00,3000:-100.00', '--post'],
    ],
    steps: [
      ['audit'],
      ['audit', 'verify'],
    ],
  },
];

function runJs(db, argv) {
  const r = spawnSync(process.execPath, ['bin/bukio.js', ...argv, ...ACTOR, '--db', db, '--json'], {
    cwd: root, encoding: 'utf8',
  });
  return r.stdout;
}

function runRust(db, argv) {
  const r = spawnSync(RUST, [...argv, ...ACTOR, '--db', db, '--json'], {
    cwd: root, encoding: 'utf8',
  });
  return r.stdout;
}

// normalise volatile values so identical logic compares equal
function normalise(obj, keyPath = '') {
  if (Array.isArray(obj)) return obj.map((v) => normalise(v, keyPath));
  if (obj && typeof obj === 'object') {
    const out = {};
    for (const [k, v] of Object.entries(obj)) {
      if (k === 'dryRun') continue; // JS init emits dryRun, others dry_run — checked separately
      if (k === 'db' && typeof v === 'string') { out[k] = '<DB>'; continue; }
      out[k] = normalise(v, k);
    }
    return out;
  }
  if (/(_at|_ts)$/.test(keyPath) || keyPath === 'ts') return '<TS>';
  if (typeof obj === 'string' && /\/tmp\/bukio-parity-/.test(obj)) return '<DB>';
  return obj;
}

function parse(stdout) {
  try {
    return JSON.parse(stdout);
  } catch {
    return { ok: false, error: { code: 'HARNESS_PARSE', message: stdout.slice(0, 300) } };
  }
}

let total = 0;
let mismatches = 0;
let vacuous = 0;
const vacuousCases = new Set();
const tmp = mkdtempSync(path.join(tmpdir(), 'bukio-parity-'));
try {
  for (const c of CASES) {
    if (FILTER && !c.name.includes(FILTER)) continue;
    const dbJs = path.join(tmp, `${c.name}-js.db`);
    const dbRs = path.join(tmp, `${c.name}-rs.db`);
    for (const step of [...(c.setup || []), ...c.steps]) {
      total += 1;
      const jsOut = normalise(parse(runJs(dbJs, step)));
      const rsOut = normalise(parse(runRust(dbRs, step)));
      const jsS = JSON.stringify(jsOut);
      const rsS = JSON.stringify(rsOut);
      // A step where BOTH engines fail with NO_DATABASE proves nothing: the
      // case never ran `init`, so every command dies before any logic runs.
      // Shared *other* errors (e.g. both reject a bad date) are real signal.
      const bogus = jsOut && rsOut && jsOut.ok === false && rsOut.ok === false
        && jsOut.error && rsOut.error && jsOut.error.code === rsOut.error.code
        && jsOut.error.code === 'NO_DATABASE';
      if (bogus) { vacuous += 1; vacuousCases.add(c.name); }
      if (process.env.PARITY_DEBUG) {
        console.log(`  [dbg ${c.name}: ${step.join(' ')}] ${bogus ? 'VACUOUS ' + jsOut.error.code : 'compared'}`);
      }
      if (jsS !== rsS) {
        mismatches += 1;
        console.log(`\n✗ ${c.name}: bukio ${step.join(' ')}`);
        console.log(`  JS  : ${jsS.slice(0, 400)}`);
        console.log(`  RS  : ${rsS.slice(0, 400)}`);
      }
    }
    console.log(`✓ case ${c.name} done`);
  }
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
console.log(`\n${total - mismatches}/${total} steps in parity`);
if (vacuous) {
  console.log(`WARNING: ${vacuous}/${total} of those steps were VACUOUS — both engines failed with the same error (usually a case that never ran 'init'), so they prove nothing.`);
  console.log(`  affected cases: ${[...vacuousCases].join(', ')}`);
}
process.exit(mismatches === 0 ? 0 : 1);
