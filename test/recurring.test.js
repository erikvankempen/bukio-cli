import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart, deactivateAccount } from '../src/core/accounts.js';
import { getEntry } from '../src/core/entries.js';
import {
  addPeriod, buildDepreciationTemplate, createTemplate, getTemplate,
  listTemplates, previewDue, runDue, setTemplateStatus,
} from '../src/recurring/index.js';
import { enableVatModule } from '../src/vat/index.js';

let db;

beforeEach(() => {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare("INSERT INTO company (name, legal_form) VALUES ('Test BV', 'bv')").run();
});

function tpl(overrides = {}) {
  return createTemplate(db, {
    name: 'Huur kantoor',
    frequency: 'monthly',
    startDate: '2026-01-01',
    postings: ['4300:1000.00,1100:-1000.00'],
    ...overrides,
  });
}

test('addPeriod: monthly/quarterly/yearly with day preserved', () => {
  assert.equal(addPeriod('2026-01-15', 'monthly', 1), '2026-02-01');
  assert.equal(addPeriod('2026-11-01', 'monthly', 5), '2026-12-05');
  assert.equal(addPeriod('2026-12-01', 'monthly', 1), '2027-01-01');
  assert.equal(addPeriod('2026-01-01', 'quarterly', 1), '2026-04-01');
  assert.equal(addPeriod('2026-10-01', 'quarterly', 1), '2027-01-01');
  assert.equal(addPeriod('2026-01-01', 'yearly', 1), '2027-01-01');
});

test('createTemplate: validates postings, balances, accounts', () => {
  assert.throws(() => tpl({ postings: ['4300:1000.00'] }), { code: 'INVALID_POSTINGS' });
  assert.throws(() => tpl({ postings: ['4300:1000.00,1100:-999.00'] }), { code: 'UNBALANCED' });
  assert.throws(() => tpl({ postings: ['9999:100.00,1100:-100.00'] }), { code: 'ACCOUNT_NOT_FOUND' });
  assert.throws(() => tpl({ frequency: 'weekly' }), { code: 'INVALID_FREQUENCY' });
  assert.throws(() => tpl({ dayOfPeriod: 29 }), { code: 'INVALID_DATE' });
  assert.throws(() => tpl({ startDate: '2026-13-01' }), { code: 'INVALID_DATE' });
  assert.throws(() => tpl({ startDate: '2026-01-01', endDate: '2025-12-01' }), { code: 'INVALID_RANGE' });
});

test('createTemplate: inactive account rejected', () => {
  deactivateAccount(db, '4300');
  assert.throws(() => tpl(), { code: 'ACCOUNT_INACTIVE' });
});

test('createTemplate: first run normalized to day_of_period', () => {
  // start day (15) > day_of_period (1): day 1 of January already passed -> first run Feb 1
  const a = tpl({ startDate: '2026-01-15', dayOfPeriod: 1 });
  assert.equal(a.next_run_date, '2026-02-01');
  const b = tpl({ startDate: '2026-01-15', dayOfPeriod: 20 });
  assert.equal(b.next_run_date, '2026-01-20');
  const c = tpl({ startDate: '2026-01-25', dayOfPeriod: 1 });
  assert.equal(c.next_run_date, '2026-02-01'); // never backwards
});

test('runDue: books one entry per period on schedule', () => {
  tpl();
  const result = runDue(db, { asOf: '2026-03-15', actor: 'agent:test' });
  assert.equal(result.templates.length, 1);
  assert.equal(result.templates[0].runs.length, 3); // Jan, Feb, Mar backfill
  const t = getTemplate(db, 1);
  assert.equal(t.runs_done, 3);
  assert.equal(t.next_run_date, '2026-04-01');
  assert.equal(t.status, 'active');

  const entries = db.prepare("SELECT * FROM journal_entries WHERE source = 'recurring' ORDER BY date").all();
  assert.deepEqual(entries.map((e) => e.date), ['2026-01-01', '2026-02-01', '2026-03-01']);
  assert.equal(entries[0].source_ref, 'tpl:1');
  assert.equal(entries[0].created_by, 'recurring');
});

test('runDue: idempotent — nothing due means nothing generated', () => {
  tpl();
  runDue(db, { asOf: '2026-01-31' });
  const again = runDue(db, { asOf: '2026-01-31' });
  assert.equal(again.templates.length, 0);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='recurring'").get().c, 1);
});

test('runDue: runs limit completes the template', () => {
  tpl({ runs: 2 });
  runDue(db, { asOf: '2026-06-30' });
  const t = getTemplate(db, 1);
  assert.equal(t.runs_done, 2);
  assert.equal(t.status, 'completed');
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='recurring'").get().c, 2);
});

test('runDue: end_date completes the template', () => {
  tpl({ endDate: '2026-02-28' });
  runDue(db, { asOf: '2026-12-31' });
  const t = getTemplate(db, 1);
  assert.equal(t.status, 'completed');
  assert.equal(t.runs_done, 2);
});

test('runDue: paused templates are skipped', () => {
  tpl();
  setTemplateStatus(db, { id: 1, status: 'paused' });
  const result = runDue(db, { asOf: '2026-03-01' });
  assert.equal(result.templates.length, 0);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='recurring'").get().c, 0);
});

test('runDue: --template runs only that template', () => {
  tpl();
  tpl({ name: 'Verzekering', postings: ['4320:100.00,1700:-100.00'] });
  const result = runDue(db, { asOf: '2026-02-01', templateId: 2 });
  assert.equal(result.templates.length, 1);
  assert.equal(result.templates[0].template_id, 2);
  // template B is due twice (Jan 1 + Feb 1) — 2 entries, all from B
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='recurring'").get().c, 2);
});

test('runDue: dry-run writes nothing', () => {
  tpl();
  const result = runDue(db, { asOf: '2026-03-01', dryRun: true });
  assert.equal(result.templates.length, 1);
  assert.equal(result.templates[0].runs.length, 3);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='recurring'").get().c, 0);
});

test('reverse_previous: accrual pattern — each run reverses the prior entry', () => {
  tpl({ name: 'Nog te betalen kosten', reversePrevious: true, postings: ['4310:250.00,2400:-250.00'] });
  runDue(db, { asOf: '2026-01-31' });
  // after month 1: one accrual entry, 2400 = -250
  let rows = db.prepare("SELECT * FROM journal_entries WHERE source='recurring'").all();
  assert.equal(rows.length, 1);
  assert.equal(db.prepare('SELECT SUM(amount_cents) s FROM postings p JOIN journal_entries e ON e.id=p.entry_id WHERE e.state=\'posted\' AND p.account_id=(SELECT id FROM accounts WHERE code=\'2400\')').get().s, -25000);

  runDue(db, { asOf: '2026-02-28' });
  // accrual 1 + reversal (source='reversal') + accrual 2 = 3 journal entries
  rows = db.prepare('SELECT * FROM journal_entries ORDER BY id').all();
  assert.equal(rows.length, 3);
  const reversal = getEntry(db, rows[1].id);
  assert.equal(reversal.source, 'reversal');
  assert.ok(reversal.reversed_from_id); // linked contra-entry
  assert.equal(reversal.state, 'posted');
  // net 2400 after two periods = -250 (one outstanding accrual)
  const net = db.prepare("SELECT SUM(amount_cents) s FROM postings p JOIN journal_entries e ON e.id=p.entry_id WHERE e.state='posted' AND p.account_id=(SELECT id FROM accounts WHERE code='2400')").get().s;
  assert.equal(net, -25000);
});

test('reverse_previous: completed accrual chain nets zero after final run', () => {
  tpl({ name: 'Tijdelijke post', reversePrevious: true, runs: 3, postings: ['4310:100.00,2400:-100.00'] });
  runDue(db, { asOf: '2026-03-31' });
  const net = db.prepare("SELECT SUM(amount_cents) s FROM postings p JOIN journal_entries e ON e.id=p.entry_id WHERE e.state='posted' AND p.account_id=(SELECT id FROM accounts WHERE code='2400')").get().s;
  assert.equal(net, -10000); // last accrual still outstanding (reversed next run — but template completed)
});

test('buildDepreciationTemplate: remainder-adjusted final run, cents-exact total', () => {
  const r = buildDepreciationTemplate(db, {
    name: 'Laptop Dell', costCents: 537000, lifeMonths: 36, startDate: '2026-01-01',
  });
  assert.equal(r.monthly_cents, 14917); // 149.17
  assert.equal(r.final_cents, 14905); // 5370.00 - 149.17*35
  assert.equal(r.total_cents, 537000);
  const t = getTemplate(db, r.template.id);
  assert.equal(t.runs, 36);
  assert.equal(t.frequency, 'monthly');
  assert.equal(t.postings[0].code, '4600');
  assert.equal(t.postings[0].amountCents, 14917);
  assert.equal(t.final_postings[0].amountCents, 14905);

  // run all 36 months -> asset fully depreciated
  runDue(db, { asOf: '2028-12-31' });
  const assetNet = db.prepare("SELECT SUM(amount_cents) s FROM postings p JOIN journal_entries e ON e.id=p.entry_id WHERE e.state='posted' AND p.account_id=(SELECT id FROM accounts WHERE code='1800')").get().s;
  assert.equal(assetNet, -537000);
});

test('buildDepreciationTemplate: validation', () => {
  assert.throws(() => buildDepreciationTemplate(db, { name: 'x', costCents: -5, lifeMonths: 36, startDate: '2026-01-01' }), { code: 'INVALID_COST' });
  assert.throws(() => buildDepreciationTemplate(db, { name: 'x', costCents: 1000, residualCents: 2000, lifeMonths: 36, startDate: '2026-01-01' }), { code: 'INVALID_RESIDUAL' });
  assert.throws(() => buildDepreciationTemplate(db, { name: 'x', costCents: 1000, lifeMonths: 1, startDate: '2026-01-01' }), { code: 'INVALID_LIFE' });
});

test('vat-aware template: expansion stored, generation replays it', () => {
  enableVatModule(db);
  const t = tpl({ name: 'Abonnement', postings: ['4340:100.00@21,1100:-121.00'] });
  assert.equal(t.vat_aware, 1);
  assert.equal(t.postings.length, 3); // net + vat leg (2500) + bank
  runDue(db, { asOf: '2026-01-31' });
  const entry = getEntry(db, 1);
  // the net posting carries the vat fields (same sign as the amount);
  // the VAT leg goes to 1500 (te vorderen)
  const net = entry.postings.find((p) => p.account_code === '4340');
  assert.equal(net.vat_amount_cents, 2100);
  const vatLeg = entry.postings.find((p) => p.account_code === '1500');
  assert.equal(vatLeg.amount_cents, 2100);
});

test('vat-aware template: requires VAT module on', () => {
  assert.throws(() => tpl({ postings: ['4340:100.00@21,1100:-121.00'] }), { code: 'VAT_MODULE_OFF' });
});

test('previewDue: read-only plan matches runDue', () => {
  tpl();
  const plan = previewDue(db, { asOf: '2026-02-15' });
  assert.equal(plan.templates[0].runs.length, 2);
  assert.equal(db.prepare("SELECT COUNT(*) c FROM journal_entries WHERE source='recurring'").get().c, 0);
});

test('listTemplates: status filter', () => {
  tpl();
  tpl({ name: 'B', runs: 1 });
  runDue(db, { asOf: '2026-01-31' });
  assert.equal(listTemplates(db, { status: 'active' }).length, 1);
  assert.equal(listTemplates(db, { status: 'completed' }).length, 1);
  assert.equal(listTemplates(db, { status: 'all' }).length, 2);
});

test('generated entries are immutable + trial balance stays balanced', () => {
  tpl();
  runDue(db, { asOf: '2026-01-31' });
  const entry = getEntry(db, 1);
  assert.equal(entry.state, 'posted');
  const totals = db.prepare('SELECT COALESCE(SUM(amount_cents),0) s FROM postings').get().s;
  assert.equal(totals, 0);
});
