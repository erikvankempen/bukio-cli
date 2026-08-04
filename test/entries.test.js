import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, getAccountByCode, listAccounts } from '../src/core/accounts.js';
import {
  createEntry, postEntry, reverseEntry, getEntry, listEntries, parsePostingSpecs,
} from '../src/core/entries.js';
import { list as listAudit } from '../src/audit/index.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
});

test('default chart is seeded with 14 VAT-free accounts', () => {
  const accounts = listAccounts(db);
  assert.equal(accounts.length, 14);
  assert.equal(accounts.some((a) => a.code === '1100' && a.type === 'asset'), true);
  assert.equal(accounts.some((a) => a.code === '8000' && a.type === 'income'), true);
  // VAT-agnostic: no btw accounts in the core chart
  assert.equal(accounts.some((a) => /btw|omzetbelasting/i.test(a.name)), false);
});

test('createEntry: balanced 2-posting entry lands as draft', () => {
  const e = createEntry(db, {
    date: '2026-08-04', description: 'Startkapitaal',
    postings: [{ code: '1100', amountCents: 1000000 }, { code: '3000', amountCents: -1000000 }],
  });
  assert.equal(e.state, 'draft');
  assert.equal(e.postings.length, 2);
  assert.equal(e.postings.reduce((s, p) => s + p.amount_cents, 0), 0);
  assert.equal(e.created_by, 'human');
});

test('createEntry: agent actor is recorded', () => {
  const e = createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
    actor: 'agent:hermes',
  });
  assert.equal(e.created_by, 'agent:hermes');
});

test('createEntry: rejects unbalanced postings', () => {
  assert.throws(() => createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -99 }],
  }), { code: 'UNBALANCED' });
});

test('createEntry: rejects fewer than 2 postings', () => {
  assert.throws(() => createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '1100', amountCents: 100 }],
  }), { code: 'TOO_FEW_POSTINGS' });
});

test('createEntry: rejects zero-amount postings', () => {
  assert.throws(() => createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '1100', amountCents: 0 }, { code: '3000', amountCents: 0 }],
  }), { code: 'INVALID_AMOUNT_CENTS' });
});

test('createEntry: rejects unknown and inactive accounts', () => {
  assert.throws(() => createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '9999', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  }), { code: 'ACCOUNT_NOT_FOUND' });

  db.prepare('UPDATE accounts SET active = 0 WHERE code = ?').run('4300');
  assert.throws(() => createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '4300', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  }), { code: 'ACCOUNT_INACTIVE' });
});

test('createEntry: rejects invalid date and missing description', () => {
  assert.throws(() => createEntry(db, {
    date: '04-08-2026', description: 'x',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  }), { code: 'INVALID_DATE' });
  assert.throws(() => createEntry(db, {
    date: '2026-08-04', description: '  ',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  }), { code: 'INVALID_DESCRIPTION' });
});

test('postEntry: draft -> posted, idempotence guarded', () => {
  const e = createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  });
  const posted = postEntry(db, { id: e.id });
  assert.equal(posted.state, 'posted');
  assert.ok(posted.posted_at);
  assert.throws(() => postEntry(db, { id: e.id }), { code: 'ALREADY_POSTED' });
});

test('postEntry: DB trigger blocks unbalanced drafts', () => {
  const e = createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  });
  // bypass the engine: make the draft unbalanced via raw SQL (allowed while draft)
  db.prepare('INSERT INTO postings (entry_id, account_id, amount_cents) VALUES (?, (SELECT id FROM accounts WHERE code = ?), ?)')
    .run(e.id, '4300', 50);
  assert.throws(() => postEntry(db, { id: e.id }), /cannot post an unbalanced entry/);
});

test('postEntry: DB trigger requires >= 2 postings', () => {
  const e = createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  });
  db.prepare('DELETE FROM postings WHERE entry_id = ? AND account_id = (SELECT id FROM accounts WHERE code = ?)')
    .run(e.id, '3000');
  assert.throws(() => postEntry(db, { id: e.id }), /at least 2 postings/);
});

test('postings of a posted entry are immutable (triggers)', () => {
  const e = createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  });
  postEntry(db, { id: e.id });
  assert.throws(
    () => db.prepare('UPDATE postings SET amount_cents = 200 WHERE entry_id = ?').run(e.id),
    /non-draft entry/,
  );
  assert.throws(
    () => db.prepare('DELETE FROM postings WHERE entry_id = ?').run(e.id),
    /non-draft entry/,
  );
});

test('reverseEntry: posts linked contra-entry; original stays posted', () => {
  const e = createEntry(db, {
    date: '2026-08-04', description: 'Verkeerde boeking',
    postings: [{ code: '4300', amountCents: 5000 }, { code: '1100', amountCents: -5000 }],
  });
  postEntry(db, { id: e.id });

  const reversal = reverseEntry(db, { id: e.id, reason: 'verkeerde categorie' });
  assert.equal(reversal.state, 'posted');
  assert.equal(reversal.source, 'reversal');
  assert.equal(reversal.reversed_from_id, e.id);
  assert.equal(reversal.postings.length, 2);
  assert.equal(reversal.postings[0].amount_cents, -5000);
  assert.equal(reversal.postings[1].amount_cents, 5000);

  // The original stays posted — the contra-entry cancels it (net effect zero).
  const original = getEntry(db, e.id);
  assert.equal(original.state, 'posted');
  assert.equal(original.reversed_at, null);
});

test('reverseEntry: guards', () => {
  const e = createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  });
  assert.throws(() => reverseEntry(db, { id: e.id }), { code: 'NOT_POSTED' }); // draft
  postEntry(db, { id: e.id });
  reverseEntry(db, { id: e.id });
  assert.throws(() => reverseEntry(db, { id: e.id }), { code: 'ALREADY_REVERSED' });
  assert.throws(() => postEntry(db, { id: e.id }), { code: 'ALREADY_POSTED' });
});

test('parsePostingSpecs: repeatable and comma-separated, negative = credit', () => {
  const specs = parsePostingSpecs(['1100:1000.00,3000:-1000.00']);
  assert.deepEqual(specs, [
    { code: '1100', amountCents: 100000 },
    { code: '3000', amountCents: -100000 },
  ]);
  const multi = parsePostingSpecs(['1100:10.00', '3000:-10.00']);
  assert.equal(multi.length, 2);
  assert.throws(() => parsePostingSpecs(['nonsense']), { code: 'INVALID_POSTING' });
  assert.throws(() => parsePostingSpecs(['1100:1.234']), { code: 'INVALID_AMOUNT' });
});

test('every mutation writes an audit record', () => {
  const e = createEntry(db, {
    date: '2026-08-04', description: 'x',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
    actor: 'agent:test',
  });
  postEntry(db, { id: e.id, actor: 'agent:test' });
  reverseEntry(db, { id: e.id, actor: 'agent:test' });
  // newest first
  const audit = listAudit(db);
  assert.deepEqual(audit.map((a) => a.action), ['entry.reverse', 'entry.post', 'entry.create']);
  assert.ok(audit.every((a) => a.actor === 'agent:test'));
});

test('listEntries: filters', () => {
  const a = createEntry(db, {
    date: '2026-01-15', description: 'jan',
    postings: [{ code: '1100', amountCents: 100 }, { code: '3000', amountCents: -100 }],
  });
  const b = createEntry(db, {
    date: '2026-02-15', description: 'feb',
    postings: [{ code: '1100', amountCents: 200 }, { code: '3000', amountCents: -200 }],
  });
  postEntry(db, { id: a.id });
  assert.equal(listEntries(db).length, 2);
  assert.equal(listEntries(db, { state: 'posted' }).length, 1);
  assert.equal(listEntries(db, { state: 'draft' }).length, 1);
  assert.equal(listEntries(db, { dateFrom: '2026-02-01' }).length, 1);
  assert.equal(listEntries(db, { dateTo: '2026-01-31' }).length, 1);
});
