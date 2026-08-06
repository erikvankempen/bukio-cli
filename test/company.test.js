import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { openDb } from '../src/core/db.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

function cli(dbPath, args, { expectFail = false } = {}) {
  const env = { ...process.env, BUKIO_DB: dbPath };
  try {
    const stdout = execFileSync(process.execPath, [BIN, '--json', ...args], { env, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    if (expectFail) return { code: err.status, out: JSON.parse(err.stdout), err: err.stderr };
    throw err;
  }
}

function tmpDb() {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-company-test-'));
  return { dir, file: path.join(dir, 'test.db') };
}

let t;
test.beforeEach(() => {
  t = tmpDb();
  cli(t.file, ['init', '--name', 'literal:Test Coaching', '--kvk', 'literal:12345678', '--legal-form', 'eenmanszaak', '--vat', 'off']);
});
test.afterEach(() => {
  rmSync(t.dir, { recursive: true, force: true });
});

function getCompany(file) {
  const db = openDb(file);
  try {
    return db.prepare('SELECT * FROM company WHERE id = 1').get();
  } finally {
    db.close();
  }
}

test('company update: sets address/iban/city and audits', () => {
  const r = cli(t.file, [
    '--actor', 'agent:test', 'company', 'update',
    '--address', 'literal:Teststraat 1', '--postal-code', 'literal:1000 AA',
    '--city', 'literal:Amsterdam', '--iban', 'literal:NL91ABNA0417164300',
  ]);
  assert.equal(r.code, 0);
  assert.equal(r.out.data.company.address, 'literal:Teststraat 1');
  const c = getCompany(t.file);
  assert.equal(c.postal_code, 'literal:1000 AA');
  assert.equal(c.city, 'literal:Amsterdam');
  assert.equal(c.iban, 'literal:NL91ABNA0417164300');
  const db = openDb(t.file);
  try {
    const audit = db.prepare("SELECT * FROM audit_log WHERE action = 'company.update' ORDER BY id DESC LIMIT 1").get();
    assert.ok(audit);
    assert.equal(audit.actor, 'agent:test');
    assert.equal(audit.command, 'company update');
  } finally {
    db.close();
  }
});

test('company update: dry-run writes nothing', () => {
  cli(t.file, ['company', 'update', '--address', 'literal:Teststraat 1', '--city', 'literal:Amsterdam', '--dry-run']);
  const c = getCompany(t.file);
  assert.equal(c.address, null);
  assert.equal(c.city, null);
});

test('company update: no options -> NOTHING_TO_UPDATE', () => {
  const r = cli(t.file, ['company', 'update'], { expectFail: true });
  assert.equal(r.code, 1);
  assert.equal(r.out.error.code, 'NOTHING_TO_UPDATE');
});

test('company update: invalid IBAN rejected', () => {
  const r = cli(t.file, ['company', 'update', '--iban', 'nope'], { expectFail: true });
  assert.equal(r.code, 1);
  assert.equal(r.out.error.code, 'INVALID_IBAN');
});

test('company show: returns the company record', () => {
  cli(t.file, ['company', 'update', '--city', 'literal:Amsterdam']);
  const r = cli(t.file, ['company', 'show']);
  assert.equal(r.out.data.company.name, 'literal:Test Coaching');
  assert.equal(r.out.data.company.kvk, 'literal:12345678');
  assert.equal(r.out.data.company.city, 'literal:Amsterdam');
});
