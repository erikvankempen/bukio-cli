import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { record, list } from '../src/audit/index.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
});

test('record + list with filters', () => {
  record(db, { actor: 'human', action: 'company.init', command: 'init', args: { name: 'X' }, outcome: 'ok' });
  record(db, { actor: 'agent:hermes', action: 'entry.create', command: 'entry add', args: { n: 1 }, outcome: 'ok', entryIds: [7] });

  const all = list(db);
  assert.equal(all.length, 2);
  assert.equal(all[0].action, 'entry.create');
  assert.deepEqual(all[0].entry_ids, [7]);
  assert.deepEqual(all[0].args, { n: 1 });

  const byActor = list(db, { actor: 'agent:hermes' });
  assert.equal(byActor.length, 1);
  const bySince = list(db, { since: '2999-01-01T00:00:00.000Z' });
  assert.equal(bySince.length, 0);
});

test('audit log is append-only: UPDATE and DELETE are blocked', () => {
  record(db, { actor: 'human', action: 'company.init', outcome: 'ok' });
  assert.throws(() => db.prepare('UPDATE audit_log SET outcome = ? WHERE action = ?').run('hacked', 'company.init'),
    /append-only/);
  assert.throws(() => db.prepare('DELETE FROM audit_log').run(), /append-only/);
  assert.equal(list(db).length, 1);
});

test('args null is stored and read back as null', () => {
  record(db, { actor: 'human', action: 'plain', outcome: 'ok' });
  const row = list(db)[0];
  assert.equal(row.args, null);
});
