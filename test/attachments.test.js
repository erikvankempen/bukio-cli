/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync, existsSync, readFileSync, mkdirSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { openDb } from '../src/core/db.js';
import { createContact, createInvoice } from '../src/invoice/index.js';
import { createEntry } from '../src/core/entries.js';
import {
  addAttachment, listAttachments, getAttachment, removeAttachment,
  MAX_ATTACHMENT_BYTES, attachmentsDir,
} from '../src/core/attachments.js';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');

function cli(dbPath, args, { expectFail = false } = {}) {
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' };
  try {
    const stdout = execFileSync(process.execPath, [BIN, '--json', ...args], { env, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    if (expectFail) return { code: err.status, out: JSON.parse(err.stdout), err: err.stderr };
    throw err;
  }
}

function tmpDb() {
  const dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-attach-test-'));
  return { dir, file: path.join(dir, 'test.db') };
}

let t;
let invId;
let entryId;

test.beforeEach(() => {
  t = tmpDb();
  cli(t.file, ['init', '--name', 'Test Coaching', '--kvk', '12345678', '--legal-form', 'eenmanszaak', '--vat', 'off']);
  const db = openDb(t.file);
  try {
    const contact = createContact(db, { name: 'Acme BV', actor: 'agent:test' });
    const inv = createInvoice(db, { contactId: contact.id, lines: ['Ding @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    invId = inv.id;
    const entry = createEntry(db, {
      date: '2026-08-10', description: 'Startkapitaal',
      postings: [{ code: '1100', amountCents: 10000 }, { code: '3000', amountCents: -10000 }],
      actor: 'agent:test',
    });
    entryId = entry.id;
  } finally {
    db.close();
  }
});

test.afterEach(() => {
  rmSync(t.dir, { recursive: true, force: true });
});

function auditRows(db, action) {
  return db.prepare('SELECT * FROM audit_log WHERE action = ? ORDER BY id').all(action);
}

test('attach add (db mode): stores BLOB, round-trips byte-identical, infers mime', () => {
  const doc = Buffer.from('%PDF-1.4 fake invoice bytes');
  const file = path.join(t.dir, 'F2026-123.pdf');
  writeFileSync(file, doc);
  const db = openDb(t.file);
  try {
    const a = addAttachment(db, { kind: 'invoice', refId: invId, filePath: file, note: 'originel', actor: 'agent:test' });
    assert.ok(a.id > 0);
    assert.equal(a.mode, 'db');
    assert.equal(a.mime, 'application/pdf');
    assert.equal(a.sha256, createHash('sha256').update(doc).digest('hex'));
    assert.equal(a.size, doc.length);

    const row = db.prepare('SELECT * FROM attachments WHERE id = ?').get(a.id);
    assert.equal(row.mode, 'db');
    assert.ok(Buffer.isBuffer(row.data));
    assert.deepEqual(Buffer.from(row.data), doc);
    assert.equal(row.path, null);

    // getAttachment returns the bytes
    const got = getAttachment(db, a.id);
    assert.deepEqual(Buffer.from(got.data), doc);
  } finally {
    db.close();
  }
});

test('attach add: works for entries too', () => {
  const doc = Buffer.from('some xml invoice');
  const file = path.join(t.dir, 'factuur.xml');
  writeFileSync(file, doc);
  const db = openDb(t.file);
  try {
    const a = addAttachment(db, { kind: 'entry', refId: entryId, filePath: file, actor: 'agent:test' });
    assert.equal(a.kind, 'entry');
    assert.equal(a.ref_id, entryId);
    assert.equal(a.mime, 'application/xml');
  } finally {
    db.close();
  }
});

test('attach add: validation errors', () => {
  const db = openDb(t.file);
  const file = path.join(t.dir, 'x.pdf');
  writeFileSync(file, 'data');
  try {
    assert.throws(() => addAttachment(db, { kind: 'bogus', refId: invId, filePath: file }), (e) => e.code === 'INVALID_KIND');
    assert.throws(() => addAttachment(db, { kind: 'invoice', refId: null, filePath: file }), (e) => e.code === 'REF_REQUIRED');
    assert.throws(() => addAttachment(db, { kind: 'invoice', refId: 999999, filePath: file }), (e) => e.code === 'NOT_FOUND');
    assert.throws(() => addAttachment(db, { kind: 'invoice', refId: invId, filePath: path.join(t.dir, 'nope.pdf') }), (e) => e.code === 'ATTACHMENT_FILE_NOT_FOUND');
    assert.throws(() => addAttachment(db, { kind: 'invoice', refId: invId, filePath: file, store: 'bogus' }), (e) => e.code === 'INVALID_STORE');

    // duplicate
    addAttachment(db, { kind: 'invoice', refId: invId, filePath: file, actor: 'agent:test' });
    assert.throws(() => addAttachment(db, { kind: 'invoice', refId: invId, filePath: file, actor: 'agent:test' }), (e) => e.code === 'ATTACHMENT_DUPLICATE');

    // too large (sparse buffer — no real 25 MB write)
    const big = path.join(t.dir, 'big.pdf');
    writeFileSync(big, Buffer.alloc(MAX_ATTACHMENT_BYTES + 1));
    assert.throws(() => addAttachment(db, { kind: 'invoice', refId: invId, filePath: big }), (e) => e.code === 'ATTACHMENT_TOO_LARGE');
    // empty file → friendly error, not a raw CHECK-constraint failure
    const empty = path.join(t.dir, 'empty.pdf');
    writeFileSync(empty, '');
    assert.throws(() => addAttachment(db, { kind: 'invoice', refId: invId, filePath: empty }), (e) => e.code === 'ATTACHMENT_EMPTY');
  } finally {
    db.close();
  }
});

test('attach list: metadata only, no data column payload', () => {
  const doc = Buffer.from('hello');
  const file = path.join(t.dir, 'a.pdf');
  writeFileSync(file, doc);
  const db = openDb(t.file);
  try {
    addAttachment(db, { kind: 'invoice', refId: invId, filePath: file, note: 'n1', actor: 'agent:test' });
    // same bytes again → duplicate, so exactly one row remains
    assert.throws(
      () => addAttachment(db, { kind: 'invoice', refId: invId, filePath: file, note: 'n2', actor: 'agent:test' }),
      (e) => e.code === 'ATTACHMENT_DUPLICATE',
    );
    const rows = listAttachments(db, { kind: 'invoice', refId: invId });
    // second add is a duplicate — expect exactly one
    assert.equal(rows.length, 1);
    assert.equal(rows[0].file_name, 'a.pdf');
    assert.equal(rows[0].mode, 'db');
    assert.equal(rows[0].sha256, createHash('sha256').update(doc).digest('hex'));
    assert.ok(!('data' in rows[0]), 'list must not carry the BLOB');
    assert.equal(listAttachments(db, { kind: 'invoice', refId: 999999 }).length, 0);
  } finally {
    db.close();
  }
});

test('attach remove: deletes row + audits; unknown id errors', () => {
  const file = path.join(t.dir, 'a.pdf');
  writeFileSync(file, 'hello');
  const db = openDb(t.file);
  try {
    const a = addAttachment(db, { kind: 'invoice', refId: invId, filePath: file, actor: 'agent:test' });
    const r = removeAttachment(db, { id: a.id, actor: 'agent:test' });
    assert.equal(r.id, a.id);
    assert.equal(db.prepare('SELECT COUNT(*) c FROM attachments').get().c, 0);
    const rows = auditRows(db, 'attachments.remove');
    assert.equal(rows.length, 1);
    assert.equal(rows[0].actor, 'agent:test');
    assert.equal(JSON.parse(rows[0].args_json).attachment_id, a.id);

    assert.throws(() => removeAttachment(db, { id: a.id }), (e) => e.code === 'ATTACHMENT_NOT_FOUND');
    assert.throws(() => getAttachment(db, a.id), (e) => e.code === 'ATTACHMENT_NOT_FOUND');
  } finally {
    db.close();
  }
});

test('attach add: dry-run writes nothing and audits nothing', () => {
  const file = path.join(t.dir, 'a.pdf');
  writeFileSync(file, 'hello');
  const db = openDb(t.file);
  try {
    const plan = addAttachment(db, { kind: 'invoice', refId: invId, filePath: file, actor: 'agent:test', dryRun: true });
    assert.equal(plan.dryRun, true);
    assert.equal(plan.action, 'attachments.add');
    assert.equal(plan.sha256, createHash('sha256').update('hello').digest('hex'));
    assert.equal(db.prepare('SELECT COUNT(*) c FROM attachments').get().c, 0);
    assert.equal(auditRows(db, 'attachments.add').length, 0);

    // dry-run remove on a real attachment plans without deleting
    const a = addAttachment(db, { kind: 'invoice', refId: invId, filePath: file, actor: 'agent:test' });
    const r = removeAttachment(db, { id: a.id, actor: 'agent:test', dryRun: true });
    assert.equal(r.dryRun, true);
    assert.equal(db.prepare('SELECT COUNT(*) c FROM attachments').get().c, 1);
    assert.equal(auditRows(db, 'attachments.remove').length, 0);

    // dry-run remove of a nonexistent id still validates
    assert.throws(() => removeAttachment(db, { id: 999999, actor: 'agent:test', dryRun: true }), (e) => e.code === 'ATTACHMENT_NOT_FOUND');
  } finally {
    db.close();
  }
});

test('attach add: file mode copies to <db>-attachments/<sha256> and remove deletes it', () => {
  const doc = Buffer.from('%PDF-1.4 file-mode doc');
  const file = path.join(t.dir, 'F2026-124.pdf');
  writeFileSync(file, doc);
  const db = openDb(t.file);
  try {
    const a = addAttachment(db, { kind: 'invoice', refId: invId, filePath: file, store: 'file', actor: 'agent:test' });
    assert.equal(a.mode, 'file');
    const expected = path.join(attachmentsDir(t.file), a.sha256);
    assert.equal(a.path, expected);
    assert.ok(existsSync(expected), 'copy must exist on disk');
    assert.deepEqual(readFileSync(expected), doc);

    // DB row carries path, no BLOB
    const row = db.prepare('SELECT * FROM attachments WHERE id = ?').get(a.id);
    assert.equal(row.data, null);
    assert.equal(row.path, expected);

    // show round-trips
    const got = getAttachment(db, a.id);
    assert.deepEqual(Buffer.from(got.data), doc);

    // remove deletes the copy
    removeAttachment(db, { id: a.id, actor: 'agent:test' });
    assert.ok(!existsSync(expected), 'file-mode copy must be deleted with the row');
  } finally {
    db.close();
  }
});

test('attach get: file-mode with missing file on disk → ATTACHMENT_FILE_MISSING', () => {
  const file = path.join(t.dir, 'a.pdf');
  writeFileSync(file, 'hello');
  const db = openDb(t.file);
  try {
    const a = addAttachment(db, { kind: 'invoice', refId: invId, filePath: file, store: 'file', actor: 'agent:test' });
    rmSync(path.dirname(a.path), { recursive: true, force: true });
    assert.throws(() => getAttachment(db, a.id), (e) => e.code === 'ATTACHMENT_FILE_MISSING');
  } finally {
    db.close();
  }
});

// --- CLI e2e ----------------------------------------------------------------

test('cli: attach add/list/show --out/remove round-trip with audit', () => {
  const doc = Buffer.from('%PDF-1.4 cli doc');
  const file = path.join(t.dir, 'F2026-125.pdf');
  writeFileSync(file, doc);

  const added = cli(t.file, ['attach', 'add', '--invoice', String(invId), '--file', file, '--note', 'cli test']);
  assert.equal(added.code, 0);
  const id = added.out.data.id;
  assert.equal(added.out.data.mode, 'db');
  assert.equal(added.out.data.file_name, 'F2026-125.pdf');

  const listed = cli(t.file, ['attach', 'list', '--invoice', String(invId)]);
  assert.equal(listed.out.data.attachments.length, 1);
  assert.equal(listed.out.data.attachments[0].id, id);

  const outFile = path.join(t.dir, 'out', 'extracted.pdf');
  const shown = cli(t.file, ['attach', 'show', '--id', String(id), '--out', outFile]);
  assert.equal(shown.code, 0);
  assert.deepEqual(readFileSync(outFile), doc);

  // --out existing without --force → FILE_EXISTS
  const blocked = cli(t.file, ['attach', 'show', '--id', String(id), '--out', outFile], { expectFail: true });
  assert.equal(blocked.out.error.code, 'FILE_EXISTS');
  const forced = cli(t.file, ['attach', 'show', '--id', String(id), '--out', outFile, '--force']);
  assert.equal(forced.code, 0);

  // audit rows exist with the named actor
  const db = openDb(t.file);
  try {
    const rows = db.prepare("SELECT * FROM audit_log WHERE action = 'attachments.add'").all();
    assert.equal(rows.length, 1);
    assert.equal(rows[0].actor, 'agent:test');
    assert.equal(rows[0].command, 'attach add');
  } finally {
    db.close();
  }

  const removed = cli(t.file, ['attach', 'remove', '--id', String(id)]);
  assert.equal(removed.code, 0);
  assert.equal(cli(t.file, ['attach', 'list', '--invoice', String(invId)]).out.data.attachments.length, 0);
});

test('cli: attach add rejects both refs, and unknown store', () => {
  const file = path.join(t.dir, 'a.pdf');
  writeFileSync(file, 'x');
  const both = cli(t.file, ['attach', 'add', '--invoice', String(invId), '--entry', String(entryId), '--file', file], { expectFail: true });
  assert.equal(both.out.error.code, 'REF_REQUIRED');
  const none = cli(t.file, ['attach', 'add', '--file', file], { expectFail: true });
  assert.equal(none.out.error.code, 'REF_REQUIRED');
  const badStore = cli(t.file, ['attach', 'add', '--invoice', String(invId), '--file', file, '--store', 'bogus'], { expectFail: true });
  assert.equal(badStore.out.error.code, 'INVALID_STORE');
});

test('cli: attach dry-run writes nothing', () => {
  const file = path.join(t.dir, 'a.pdf');
  writeFileSync(file, 'x');
  const r = cli(t.file, ['attach', 'add', '--invoice', String(invId), '--file', file, '--dry-run']);
  assert.equal(r.out.data.dryRun, true);
  const db = openDb(t.file);
  try {
    assert.equal(db.prepare('SELECT COUNT(*) c FROM attachments').get().c, 0);
  } finally {
    db.close();
  }
});

test('cli: attach file mode end-to-end', () => {
  const doc = Buffer.from('%PDF-1.4 cli file mode');
  const file = path.join(t.dir, 'F2026-126.pdf');
  writeFileSync(file, doc);
  const r = cli(t.file, ['attach', 'add', '--invoice', String(invId), '--file', file, '--store', 'file']);
  assert.equal(r.out.data.mode, 'file');
  const id = r.out.data.id;
  const outFile = path.join(t.dir, 'extracted2.pdf');
  cli(t.file, ['attach', 'show', '--id', String(id), '--out', outFile]);
  assert.deepEqual(readFileSync(outFile), doc);
});

test('migration 013 applies on fresh init (attachments table exists)', () => {
  const db = openDb(t.file);
  try {
    const cols = db.prepare("PRAGMA table_info('attachments')").all().map((c) => c.name);
    for (const c of ['kind', 'ref_id', 'file_name', 'mime', 'size', 'sha256', 'mode', 'data', 'path', 'note']) {
      assert.ok(cols.includes(c), `missing column ${c}`);
    }
    assert.ok(cols.includes('created_by'));
  } finally {
    db.close();
  }
});

test('attachmentsDir convention: demo.db → demo-attachments/', () => {
  assert.equal(
    attachmentsDir('/tmp/x/demo.db'),
    path.join('/tmp/x', 'demo-attachments'),
  );
});

test('file-mode attachments dir is created under the DB dir (regression)', () => {
  mkdirSync(path.join(t.dir, 'nested'), { recursive: true });
  const nestedDb = path.join(t.dir, 'nested', 'sub.db');
  const doc = Buffer.from('x');
  const file = path.join(t.dir, 'a.pdf');
  writeFileSync(file, doc);
  cli(nestedDb, ['init', '--name', 'Test Coaching', '--kvk', '12345678', '--legal-form', 'eenmanszaak', '--vat', 'off']);
  const db = openDb(nestedDb);
  try {
    const contact = createContact(db, { name: 'Acme BV', actor: 'agent:test' });
    const inv = createInvoice(db, { contactId: contact.id, lines: ['Ding @ 10.00'], date: '2026-08-10', actor: 'agent:test' });
    const a = addAttachment(db, { kind: 'invoice', refId: inv.id, filePath: file, store: 'file', actor: 'agent:test' });
    assert.equal(path.dirname(a.path), path.join(t.dir, 'nested', 'sub-attachments'));
    assert.ok(existsSync(a.path));
  } finally {
    db.close();
  }
});
