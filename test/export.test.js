/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, getAccountByCode } from '../src/core/accounts.js';
import { createEntry, postEntry } from '../src/core/entries.js';
import { exportXaf } from '../src/export/index.js';
import { importXaf } from '../src/import/index.js';
import { list } from '../src/audit/index.js';

let db;
let tmpDir;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
  seedCompany();
  tmpDir = mkdtempSync(path.join(os.tmpdir(), 'bukio-export-test-'));
});

afterEach(() => {
  db.close();
  rmSync(tmpDir, { recursive: true, force: true });
});

function post(date, description, postings, extra = {}) {
  const e = createEntry(db, {
    date, description, postings,
    source: extra.source ?? 'manual', sourceRef: extra.sourceRef ?? null,
    actor: 'agent:test',
  });
  return postEntry(db, { id: e.id, actor: 'agent:test' });
}

function seedCompany() {
  db.prepare(
    'INSERT INTO company (id, name, registration_id, legal_form, vat_module, kor_flag, fiscal_year_end) VALUES (1, ?, ?, ?, 0, 0, ?)',
  ).run('Test Coaching', '12345678', 'eenmanszaak', '12-31');
}

function seedScenario() {
  post('2026-01-05', 'Startkapitaal', [
    { code: '1100', amountCents: 1000000 }, { code: '3000', amountCents: -1000000 },
  ]);
  post('2026-02-10', 'Omzet', [
    { code: '1100', amountCents: 121000 }, { code: '8000', amountCents: -121000 },
  ]);
  // 3-leg VAT split
  post('2026-03-01', 'Factuur met btw', [
    { code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2100', amountCents: -2100 },
  ]);
  // a draft must NOT appear in the export
  createEntry(db, {
    date: '2026-04-01', description: 'Concept', postings: [
      { code: '4300', amountCents: 5000 }, { code: '1100', amountCents: -5000 },
    ], actor: 'agent:test',
  });
}

// --- export xaf (module) ---------------------------------------------------

test('export xaf: writes a 4.0 file with header, chart and one Mutatie per posted entry', () => {
  seedScenario();
  const out = path.join(tmpDir, 'jaar-2026.xaf');
  const res = exportXaf(db, { year: '2026', out, actor: 'agent:test' });

  assert.equal(res.ok, true);
  assert.equal(res.year, '2026');
  assert.equal(res.company.name, 'Test Coaching'); // default seeded chart has no company? see below
  assert.equal(res.rekeningen, 29);
  assert.equal(res.mutaties, 3); // draft excluded

  const xml = readFileSync(out, 'utf8');
  assert.match(xml, /<Xaf xmlns="http:\/\/www\.auditfiles\.nl\/XAF\/4\.0">/);
  assert.match(xml, /<Version>4\.0<\/Version>/);
  assert.match(xml, /<Boekstuknummer>1<\/Boekstuknummer>/);
  assert.match(xml, /<Boekstuknummer>3<\/Boekstuknummer>/);
  assert.doesNotMatch(xml, /Concept/); // drafts are not exported
  // every Boeking carries rekening + tegenrekening + a positive bedrag
  const boekingen = xml.match(/<Boeking>/g) ?? [];
  assert.equal(boekingen.length, 4); // 1 + 1 + 2 (3-leg splits into two pairs)
  assert.match(xml, /<Bedrag>10000\.00<\/Bedrag>/);
  assert.doesNotMatch(xml, /-10000\.00/); // bedrag is always positive in XAF
});

test('export xaf: 3-leg entry round-trips through the importer losslessly', () => {
  seedScenario();
  const out = path.join(tmpDir, 'roundtrip.xaf');
  exportXaf(db, { year: '2026', out, actor: 'agent:test' });

  // fresh DB, import the file back
  const db2 = openDb(':memory:');
  seedDefaultChart(db2);
  const res = importXaf(db2, { xmlText: readFileSync(out, 'utf8'), actor: 'agent:test' });
  assert.equal(res.imported, 3);

  // the 3-leg entry: 1100:12100 / 8000:-10000 / 2100:-2100 must reconstruct
  const e = db2.prepare("SELECT * FROM journal_entries WHERE description = 'Factuur met btw'").get();
  assert.ok(e);
  const postings = db2.prepare(
    'SELECT a.code, p.amount_cents FROM postings p JOIN accounts a ON a.id = p.account_id WHERE p.entry_id = ? ORDER BY a.code',
  ).all(e.id);
  // the importer emits two postings per Boeking — sum per account code
  const map = {};
  for (const p of postings) map[p.code] = (map[p.code] ?? 0) + p.amount_cents;
  assert.equal(map['1100'], 12100);
  assert.equal(map['8000'], -10000);
  assert.equal(map['2100'], -2100);
  db2.close();
});

test('export xaf: follows the FISCAL year for non-calendar fiscal years', () => {
  // FY ends 06-30 -> exporting year 2026 covers 2025-07-01..2026-06-30
  db.prepare("UPDATE company SET fiscal_year_end = '06-30'").run();
  post('2025-06-15', 'Te vroeg', [{ code: '1100', amountCents: 1000 }, { code: '8000', amountCents: -1000 }]); // before
  post('2025-09-01', 'Binnen', [{ code: '1100', amountCents: 2000 }, { code: '8000', amountCents: -2000 }]); // inside
  post('2026-06-30', 'Laatste', [{ code: '1100', amountCents: 3000 }, { code: '8000', amountCents: -3000 }]); // inside (FY end)
  post('2026-07-15', 'Te laat', [{ code: '1100', amountCents: 4000 }, { code: '8000', amountCents: -4000 }]); // after

  const out = path.join(tmpDir, 'fiscaal-2026.xaf');
  const res = exportXaf(db, { year: '2026', out, actor: 'agent:test' });
  assert.equal(res.mutaties, 2); // only the in-window entries

  const xml = readFileSync(out, 'utf8');
  assert.match(xml, /<StartDate>2025-07-01<\/StartDate>/);
  assert.match(xml, /<EndDate>2026-06-30<\/EndDate>/);
  assert.doesNotMatch(xml, /Te vroeg/);
  assert.doesNotMatch(xml, /Te laat/);
});

test('export xaf: records an export.xaf audit row', () => {
  seedScenario();
  const out = path.join(tmpDir, 'audited.xaf');
  exportXaf(db, { year: '2026', out, actor: 'agent:test' });
  const rows = list(db, {});
  const row = rows.find((r) => r.action === 'export.xaf');
  assert.ok(row, 'export.xaf audit row expected');
  assert.equal(row.actor, 'agent:test');
  assert.equal(row.args.year, '2026');
  assert.equal(row.args.out, out);
});

test('export xaf: throws EXPORT_EMPTY_YEAR for a year with no posted entries', () => {
  post('2025-12-31', 'Beginbalans', [
    { code: '1100', amountCents: 10000 }, { code: '3000', amountCents: -10000 },
  ]);
  assert.throws(() => exportXaf(db, { year: '2027', out: path.join(tmpDir, 'empty.xaf'), actor: 'agent:test' }), (e) => e.code === 'EXPORT_EMPTY_YEAR');
});

test('export xaf: escaping — ampersands and < in descriptions survive XML', () => {
  post('2026-01-02', 'Kosten & "<extra>"', [
    { code: '4300', amountCents: 1000 }, { code: '1100', amountCents: -1000 },
  ]);
  const out = path.join(tmpDir, 'escape.xaf');
  exportXaf(db, { year: '2026', out, actor: 'agent:test' });
  const xml = readFileSync(out, 'utf8');
  assert.match(xml, /Kosten &amp; &quot;&lt;extra&gt;&quot;/);
});

// --- export xaf (CLI) ------------------------------------------------------

function cli(dbPath, args) {
  const root = path.resolve(import.meta.dirname, '..');
  return execFileSync('node', [path.join(root, 'bin/bukio.js'), '--db', dbPath, ...args], {
    encoding: 'utf8', env: { ...process.env, BUKIO_ACTOR: 'agent:test' },
  });
}

test('cli: bukio export xaf --year --out writes a file', () => {
  seedScenario();
  const file = path.join(tmpDir, 'live.db');
  // copy the in-memory db to a file for the CLI
  db.prepare('VACUUM INTO ?').run(file);
  const out = path.join(tmpDir, 'cli.xaf');
  const stdout = cli(file, ['export', 'xaf', '--year', '2026', '--out', out]);
  assert.match(stdout, /wrote .*cli\.xaf/);
  assert.ok(readFileSync(out, 'utf8').includes('<Xaf'));
});

// --- audit csv/xlsx --------------------------------------------------------

test('audit: csv format exports rows with headers', () => {
  seedScenario();
  const file = path.join(tmpDir, 'live2.db');
  db.prepare('VACUUM INTO ?').run(file);
  const out = path.join(tmpDir, 'audit.csv');
  const stdout = cli(file, ['audit', '--format', 'csv', '--out', out, '--limit', '5']);
  assert.match(stdout, /wrote .*audit\.csv/);
  const csv = readFileSync(out, 'utf8');
  assert.match(csv, /^id,timestamp,actor,action,command,args,outcome,entry_ids/);
  assert.match(csv, /entry\.post/);
});

test('audit: xlsx format requires --out and writes a workbook', async () => {
  seedScenario();
  const file = path.join(tmpDir, 'live3.db');
  db.prepare('VACUUM INTO ?').run(file);
  const out = path.join(tmpDir, 'audit.xlsx');
  const stdout = cli(file, ['audit', '--format', 'xlsx', '--out', out, '--limit', '5']);
  assert.match(stdout, /wrote .*audit\.xlsx/);
  const buf = readFileSync(out);
  assert.equal(buf[0], 0x50); // PK zip magic
  assert.equal(buf[1], 0x4b);

  // no --out → OUT_REQUIRED error (CLI errors go to stderr)
  let failed = false;
  try {
    cli(file, ['audit', '--format', 'xlsx']);
  } catch (err) {
    failed = true;
    const combined = `${err.stdout ?? ''}${err.stderr ?? ''}`;
    assert.match(combined, /OUT_REQUIRED/);
  }
  assert.equal(failed, true);
});

test('export xaf: unknown-year-only drafts → EXPORT_EMPTY_YEAR via CLI', () => {
  const file = path.join(tmpDir, 'live4.db');
  db.prepare('VACUUM INTO ?').run(file);
  let failed = false;
  try {
    cli(file, ['export', 'xaf', '--year', '2030', '--out', path.join(tmpDir, 'x.xaf')]);
  } catch (err) {
    failed = true;
    const combined = `${err.stdout ?? ''}${err.stderr ?? ''}`;
    assert.match(combined, /EXPORT_EMPTY_YEAR/);
  }
  assert.equal(failed, true);
});
