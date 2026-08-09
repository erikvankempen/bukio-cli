/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Full-year simulation of ONE fictitious company, driven through the real
// CLI (bin/bukio.js) exactly as a bookkeeper/agent would: init -> capital ->
// contacts/items -> sales (line + total discounts, 21%/9%, EU reverse charge,
// credit note) -> purchases (standard, EU verlegd, binnenlands verlegd) ->
// bank statement import + auto-match -> two complete OB filing/settlement
// cycles -> reports -> SEPA payables -> year-end close + jaarrekening + ICP.
// Every stage asserts exact cents; the books must reconcile at every step.
//
// Fictional company: Noordwind Handel BV (all data invented, no personal or
// real-company data — the repo is a fully public Apache-2.0 project).
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, existsSync, readFileSync, rmSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const BIN = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'bin', 'bukio.js');
const IBAN = 'NL91ABNA0417164300';

let dir;
let dbPath;
let pain001Path;
let backupPath;

before(() => {
  dir = mkdtempSync(path.join(os.tmpdir(), 'bukio-sim-'));
  dbPath = path.join(dir, 'noordwind.db');
  pain001Path = path.join(dir, 'betalingen.xml');
  backupPath = path.join(dir, 'backup.db');
});

after(() => {
  rmSync(dir, { recursive: true, force: true });
});

/** Run a CLI command against the simulation DB; parse the JSON document. */
function run(args, { expectFail = false } = {}) {
  const env = { ...process.env, BUKIO_DB: dbPath, BUKIO_ACTOR: 'agent:test' };
  try {
    const stdout = execFileSync(process.execPath, [BIN, ...args], { env, encoding: 'utf8' });
    return { code: 0, out: JSON.parse(stdout) };
  } catch (err) {
    if (expectFail) return { code: err.status, out: JSON.parse(err.stdout), err: err.stderr };
    const detail = err.stdout ? `\nstdout: ${err.stdout.slice(0, 800)}` : '';
    const errDetail = err.stderr ? `\nstderr: ${err.stderr.slice(0, 400)}` : '';
    throw new Error(`command failed (${args.join(' ')})${detail}${errDetail}`);
  }
}

/** Trial-balance net_cents for one account code. */
function tb(code) {
  const { out } = run(['report', 'trial-balance', '--json']);
  assert.equal(out.ok, true, 'trial balance must succeed');
  assert.equal(out.data.balanced, true, 'books must stay balanced');
  const acc = out.data.accounts.find((a) => a.code === code);
  return acc ? acc.net_cents : 0;
}

/** Build a CAMT.053 statement with one Ntry per entry. */
function camt(entries) {
  const ntry = entries.map((e, i) => `    <Ntry>
      <Amt>${(e.cents / 100).toFixed(2)}</Amt><CdtDbtInd>${e.dir}</CdtDbtInd>
      <AcctSvcrRef>${e.ref ?? `REF${i}`}</AcctSvcrRef>
      <BookgDt><Dt>${e.date}</Dt></BookgDt>
      <NtryDtls><TxDtls><RltdPties>${e.dir === 'DBIT' ? `<Cdtr><Nm>${e.party}</Nm></Cdtr>` : `<Dbtr><Nm>${e.party}</Nm></Dbtr>`}</RltdPties>
        <RmtInf><Ustrd>${e.desc}</Ustrd></RmtInf></TxDtls></NtryDtls>
    </Ntry>`).join('\n');
  return `<?xml version="1.0"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
<BkToCstmrStmt><Stmt><Acct><Id><IBAN>${IBAN}</IBAN></Id></Acct>
${ntry}
</Stmt></BkToCstmrStmt></Document>`;
}

function writeStmt(name, entries) {
  const p = path.join(dir, name);
  writeFileSync(p, camt(entries));
  return p;
}

// ---------------------------------------------------------------------------
// Stage 1 — company setup
// ---------------------------------------------------------------------------

test('stage 1: init (dry-run then real), capital, bank account, company profile', () => {
  // dry-run plans, creates nothing
  const plan = run(['init', '--name', 'Noordwind Handel BV', '--kvk', '81234567', '--legal-form', 'bv', '--vat', 'on', '--dry-run', '--json']);
  assert.equal(plan.out.ok, true);
  assert.equal(plan.out.data.dryRun, true);
  assert.equal(existsSync(dbPath), false);

  // real init: company + chart with the VAT accounts
  const init = run(['init', '--name', 'Noordwind Handel BV', '--kvk', '81234567', '--legal-form', 'bv', '--vat', 'on', '--json']);
  assert.equal(init.out.data.company.name, 'Noordwind Handel BV');
  assert.equal(init.out.data.company.vat_module, 1);

  // startkapitaal 25.000,00
  run(['entry', 'add', '--date', '2026-01-02', '--desc', 'Startkapitaal', '--postings', '1100:25000.00,3000:-25000.00', '--post', '--json']);
  // register the bank account (same IBAN the CAMT statements will carry)
  run(['bank', 'add', '--iban', IBAN, '--name', 'Zakelijke rekening', '--json']);
  // complete the supplier profile (12-vereisten before any invoice finalize)
  run(['company', 'update', '--address', 'Industrieweg 12', '--postal-code', '2712 CD', '--city', 'Zoetermeer', '--btw-id', 'NL812345678B01', '--iban', IBAN, '--json']);

  assert.equal(tb('1100'), 2500000);
  assert.equal(tb('3000'), -2500000);
});

// ---------------------------------------------------------------------------
// Stage 2 — contacts + items catalog
// ---------------------------------------------------------------------------

test('stage 2: contacts (NL/EU customers, NL/EU suppliers) + items catalog', () => {
  const acme = run(['contact', 'add', '--name', 'ACME B.V.', '--address', 'Straat 1', '--postal-code', '1000 AA', '--city', 'Amsterdam', '--vat-id', 'NL999999999B01', '--json']);
  const berlin = run(['contact', 'add', '--name', 'Berliner Handel GmbH', '--address', 'Hauptstrasse 5', '--postal-code', '10115', '--city', 'Berlin', '--country', 'DE', '--vat-id', 'DE123456789', '--json']);
  const kantoorx = run(['contact', 'add', '--name', 'KantoorX B.V.', '--address', 'Kantoorlaan 3', '--postal-code', '5611 AA', '--city', 'Eindhoven', '--vat-id', 'NL888888888B01', '--iban', 'NL02ABNA0123456789', '--json']);
  const softhaus = run(['contact', 'add', '--name', 'Softwarehaus GmbH', '--address', 'IT-Park 9', '--postal-code', '80331', '--city', 'Muenchen', '--country', 'DE', '--vat-id', 'DE987654321', '--iban', 'DE89370400440532013000', '--json']);
  assert.equal(acme.out.data.contact.id, 1);
  assert.equal(berlin.out.data.contact.id, 2);
  assert.equal(kantoorx.out.data.contact.id, 3);
  assert.equal(softhaus.out.data.contact.id, 4);

  const c1 = run(['item', 'add', '--name', 'Consultancy', '--unit', 'h', '--price', '150.00', '--vat', '21', '--json']);
  const c2 = run(['item', 'add', '--name', 'Boek', '--unit', 'unit', '--price', '20.00', '--vat', '9', '--json']);
  const c3 = run(['item', 'add', '--name', 'ICT Support', '--unit', 'h', '--price', '90.00', '--vat', '21', '--json']);
  assert.equal(c1.out.data.item.id, 1);
  assert.equal(c2.out.data.item.id, 2);
  assert.equal(c3.out.data.item.id, 3);
});

// ---------------------------------------------------------------------------
// Stage 3 — sales: line + total discounts, 21%/9%, EU reverse charge, credit
// ---------------------------------------------------------------------------

test('stage 3: sales with discounts, mixed rates, verlegde EU levering, credit note', () => {
  // S1 — ACME: line discount 10% on the consultancy, plus a 9% line
  const s1 = run(['invoice', 'create', '--contact', '1', '--date', '2026-01-10', '--lines', '2x Consultancy @ 150.00 @21 @-10%,1x Boek @ 20.00 @9', '--json']);
  assert.equal(s1.out.data.invoice.gross_cents, 34850); // net 290.00 + vat 58.50
  const f1 = run(['invoice', 'finalize', '--id', '1', '--json']);
  assert.equal(f1.out.data.invoice.invoice_number, '2026-0001');

  // S2 — Berliner: invoice-level discount 5% across two VAT rates
  const s2 = run(['invoice', 'create', '--contact', '2', '--date', '2026-01-15', '--lines', '1x Consultancy @ 150.00 @21,3x Boek @ 20.00 @9', '--discount-pct', '5', '--json']);
  assert.equal(s2.out.data.invoice.net_cents, 19950);   // 210.00 - 10.50 discount
  assert.equal(s2.out.data.invoice.vat_cents, 3506);    // 142.50@21% + 57.00@9%
  assert.equal(s2.out.data.invoice.gross_cents, 23456);
  assert.equal(s2.out.data.invoice.discount_cents, 1050);
  const f2 = run(['invoice', 'finalize', '--id', '2', '--json']);
  assert.equal(f2.out.data.invoice.invoice_number, '2026-0002');

  // S3 — Berliner (EU): btw-verlegde levering @RE — no VAT on the books
  const s3 = run(['invoice', 'create', '--contact', '2', '--date', '2026-01-20', '--lines', '2x ICT Support @ 90.00 @RE', '--json']);
  assert.equal(s3.out.data.invoice.net_cents, 18000);
  assert.equal(s3.out.data.invoice.vat_cents, 0);
  assert.equal(s3.out.data.invoice.gross_cents, 18000);
  const f3 = run(['invoice', 'finalize', '--id', '3', '--json']);
  assert.equal(f3.out.data.invoice.invoice_number, '2026-0003');

  // S4 — ACME: line discount 25% on the books
  const s4 = run(['invoice', 'create', '--contact', '1', '--date', '2026-02-05', '--lines', '4x Boek @ 20.00 @9 @-25%', '--json']);
  assert.equal(s4.out.data.invoice.gross_cents, 6540); // net 60.00 + vat 5.40
  const f4 = run(['invoice', 'finalize', '--id', '4', '--json']);
  assert.equal(f4.out.data.invoice.invoice_number, '2026-0004');

  // S5 — ACME: wrong tariff, gets a credit note (2026-0006) that reverses it
  const s5 = run(['invoice', 'create', '--contact', '1', '--date', '2026-02-10', '--lines', '1x Consultancy @ 150.00 @21', '--json']);
  assert.equal(s5.out.data.invoice.gross_cents, 18150);
  const f5 = run(['invoice', 'finalize', '--id', '5', '--json']);
  assert.equal(f5.out.data.invoice.invoice_number, '2026-0005');

  const cr = run(['invoice', 'credit', '--id', '5', '--date', '2026-02-12', '--reason', 'tarief gecorrigeerd', '--json']);
  assert.equal(cr.out.data.invoice.invoice_type, 'credit');
  assert.equal(cr.out.data.invoice.credit_for_invoice_id, 5);
  const f6 = run(['invoice', 'finalize', '--id', '6', '--json']);
  assert.equal(f6.out.data.invoice.invoice_number, '2026-0006');

  // sales net on the books: S1 290.00 + S2 199.50 + S3 180.00 + S4 60.00 + S5 150.00 - credit 150.00
  assert.equal(tb('8000'), -(29000 + 19950 + 18000 + 6000));
  assert.equal(tb('2500'), -(5850 + 3506 + 540)); // S5 + credit net to zero
  assert.equal(tb('1200'), 34850 + 23456 + 18000 + 6540);
});

// ---------------------------------------------------------------------------
// Stage 4 — purchases: standard 21%, EU verlegd, binnenlands verlegd
// ---------------------------------------------------------------------------

test('stage 4: purchases — 21%, EU verlegd (RE), binnenlands verlegd (R)', () => {
  // P1 — kantoorartikelen 21%: 50.00 net + 10.50 voorbelasting
  run(['vat', 'book', '--date', '2026-01-08', '--desc', 'KantoorX - KX-2026-001', '--postings', '4300:50.00@21,1100:-60.50', '--post', '--json']);
  // P2 — EU software, btw verlegd: net 300.00, verschuldigd + aftrekbaar net zero
  run(['vat', 'book', '--date', '2026-01-12', '--desc', 'Softwarehaus - SH-2026-042 (EU verlegd)', '--postings', '4340:300.00@RE,1100:-300.00', '--post', '--json']);
  // P3 — Nederlandse onderaannemer, btw verlegd: net 400.00
  run(['vat', 'book', '--date', '2026-01-18', '--desc', 'Bouwbedrijf De Lier - 2026-013 (verlegd)', '--postings', '4000:400.00@R,1100:-400.00', '--post', '--json']);
  // P4 — marketing 21%
  run(['vat', 'book', '--date', '2026-02-03', '--desc', 'KantoorX - KX-2026-014', '--postings', '4300:100.00@21,1100:-121.00', '--post', '--json']);
  // P5 — verzekering 21%
  run(['vat', 'book', '--date', '2026-02-15', '--desc', 'Verzekeraar - polis 2026', '--postings', '4320:40.00@21,1100:-48.40', '--post', '--json']);

  assert.equal(tb('1500'), 1050 + 2100 + 840); // only standard input VAT lands here
  assert.equal(tb('1100'), 2500000 - (6050 + 30000 + 40000 + 12100 + 4840));
});

// ---------------------------------------------------------------------------
// Stage 5 — bank statement import + auto-match (customers pay, suppliers paid)
// ---------------------------------------------------------------------------

test('stage 5: bank import + auto-match — 9 transactions reconcile', () => {
  const stmt = writeStmt('q1.xml', [
    // outgoing: the five booked purchases (unique amounts -> deterministic)
    { dir: 'DBIT', cents: 6050, date: '2026-01-08', party: 'KantoorX B.V.', desc: 'KX-2026-001', ref: 'NTRY001' },
    { dir: 'DBIT', cents: 30000, date: '2026-01-12', party: 'Softwarehaus GmbH', desc: 'SH-2026-042', ref: 'NTRY002' },
    { dir: 'DBIT', cents: 40000, date: '2026-01-18', party: 'Bouwbedrijf De Lier', desc: '2026-013', ref: 'NTRY003' },
    { dir: 'DBIT', cents: 12100, date: '2026-02-03', party: 'KantoorX B.V.', desc: 'KX-2026-014', ref: 'NTRY004' },
    { dir: 'DBIT', cents: 4840, date: '2026-02-15', party: 'Verzekeraar', desc: 'polis 2026', ref: 'NTRY005' },
    // incoming: customer payments against the four open invoices
    { dir: 'CRDT', cents: 34850, date: '2026-02-01', party: 'ACME B.V.', desc: '2026-0001', ref: 'NTRY006' },
    { dir: 'CRDT', cents: 23456, date: '2026-02-15', party: 'Berliner Handel GmbH', desc: '2026-0002', ref: 'NTRY007' },
    { dir: 'CRDT', cents: 18000, date: '2026-02-15', party: 'Berliner Handel GmbH', desc: '2026-0003', ref: 'NTRY008' },
    { dir: 'CRDT', cents: 6540, date: '2026-03-01', party: 'ACME B.V.', desc: '2026-0004', ref: 'NTRY009' },
  ]);
  const imp = run(['bank', 'import', '--file', stmt, '--iban', IBAN, '--json']);
  assert.equal(imp.out.ok, true);

  const match = run(['bank', 'match', 'auto', '--json']);
  assert.equal(match.out.data.matched.length, 9);
  assert.equal(match.out.data.unmatched_remaining, 0);

  // invoices 1-4 are paid; the books reconcile
  const list = run(['invoice', 'list', '--json']);
  for (const id of [1, 2, 3, 4]) {
    const inv = list.out.data.invoices.find((i) => i.id === id);
    assert.equal(inv.status, 'paid', `invoice ${id} must be paid`);
    assert.equal(inv.outstanding_cents, 0);
  }
  assert.equal(tb('1200'), 18150 - 18150); // only S5 + its credit note remain (net zero)
  assert.equal(tb('1100'), 2500000 - (6050 + 30000 + 40000 + 12100 + 4840) + (34850 + 23456 + 18000 + 6540));
});

// ---------------------------------------------------------------------------
// Stage 6 — Q1 OB readout (fields 1a-5d, exact cents)
// ---------------------------------------------------------------------------

test('stage 6: Q1 OB readout — 1a/1b/2a/3a/3b/4a/4b/5a/5b/5d', () => {
  const r = run(['vat', 'readout', '--period', '2026-Q1', '--json']);
  const f = r.out.data.fields;
  assert.equal(f['1a'].cents, 41250);  // omzet 21%: S1 27000 + S2 14250 + S5 15000 - credit 15000
  assert.equal(f['1b'].cents, 13700);  // omzet 9%: S1 2000 + S2 5700 + S4 6000
  assert.equal(f['1c'].cents, 0);
  assert.equal(f['2a'].cents, 18000);  // verlegde EU levering (S3)
  assert.equal(f['3a'].cents, 59000);  // inkopen 21% + binnenlands verlegd
  assert.equal(f['3b'].cents, 30000);  // EU inkopen verlegd (P2)
  assert.equal(f['4a'].cents, 8400);   // verschuldigd binnenlands verlegd (P3)
  assert.equal(f['4b'].cents, 6300);   // verschuldigd EU verlegd (P2)
  assert.equal(f['5a'].cents, 9896);   // omzetbelasting
  assert.equal(f['5b'].cents, 18690);  // voorbelasting incl. verlegde inkopen
  assert.equal(f['5d'].cents, 5906);   // te betalen: 9896 + 8400 + 6300 - 18690
  assert.equal(r.out.data.to_pay, '59.06');
});

// ---------------------------------------------------------------------------
// Stage 7 — Q1 filing + settlement (af te dragen -> bank payment)
// ---------------------------------------------------------------------------

test('stage 7: Q1 vat file + settle — position 2510, whole-euro payment, rounding to 4700', () => {
  // filing reclassifies the net position (5906) to 'Af te dragen omzetbelasting'
  const plan = run(['vat', 'file', '--period', '2026-Q1', '--dry-run', '--json']);
  assert.equal(plan.out.data.dryRun, true);
  assert.equal(plan.out.data.liability_cents, 5906);

  const file = run(['vat', 'file', '--period', '2026-Q1', '--json']);
  assert.equal(file.out.data.owe, true);
  assert.equal(file.out.data.liability_cents, 5906);
  assert.equal(file.out.data.account, '2510');
  assert.equal(tb('2510'), -5906);
  assert.equal(tb('1500'), 0);
  assert.equal(tb('2500'), 0);

  // the filed return is paid in whole euros (59,00) — the 6-cent difference is ours
  const stmt = writeStmt('q1-vat.xml', [
    { dir: 'DBIT', cents: 5900, date: '2026-04-10', party: 'Belastingdienst', desc: 'OB-aangifte 2026-Q1', ref: 'NTRY010' },
  ]);
  run(['bank', 'import', '--file', stmt, '--iban', IBAN, '--json']);
  const txs = run(['bank', 'transactions', '--state', 'unmatched', '--json']);
  const vatTx = txs.out.data.transactions.find((t) => t.counterparty === 'Belastingdienst');
  assert.ok(vatTx, 'Belastingdienst payment must be unmatched');

  const sPlan = run(['vat', 'settle', '--tx', String(vatTx.id), '--period', '2026-Q1', '--dry-run', '--json']);
  assert.equal(sPlan.out.data.difference_cents, -6);

  const settle = run(['vat', 'settle', '--tx', String(vatTx.id), '--period', '2026-Q1', '--json']);
  assert.equal(settle.out.data.difference_cents, -6);
  assert.equal(tb('2510'), 0);
  assert.equal(tb('4700'), -6); // afrondingsverschil (credit = gain)
  assert.equal(tb('1100'), 2500000 - (6050 + 30000 + 40000 + 12100 + 4840) + (34850 + 23456 + 18000 + 6540) - 5900);
});

// ---------------------------------------------------------------------------
// Stage 8 — reports at Q1 close: P&L, sales by contact, aging, statement, month-end
// ---------------------------------------------------------------------------

test('stage 8: P&L, sales by contact, aging, contact statement, month-end check', () => {
  // P&L after Q1 only (S6/P6 arrive in stage 9): omzet 729.50 (credit note
  // nets S5), kosten 889.94 (incl. -0.06 rounding gain) -> result -160.44
  const pnl = run(['report', 'pnl', '--year', '2026', '--json']);
  assert.equal(pnl.out.data.revenue_cents, 72950);
  assert.equal(pnl.out.data.costs_cents, 88994);
  assert.equal(pnl.out.data.result_cents, -16044);

  // sales report: per contact, gross (credit notes excluded)
  const sales = run(['report', 'sales', '--year', '2026', '--by', 'contact', '--json']);
  const acme = sales.out.data.groups.find((r) => r.name === 'ACME B.V.');
  const berlin = sales.out.data.groups.find((r) => r.name === 'Berliner Handel GmbH');
  assert.equal(acme.gross_cents, 59540);    // S1 + S4 + S5
  assert.equal(berlin.gross_cents, 41456);  // S2 + S3
  assert.equal(acme.gross_cents + berlin.gross_cents, 100996);

  // aging: the finalized credit note (2026-0006) nets the open S5 (181.50)
  // FIFO against the oldest debt — ACME's dunning total is 0, exactly like
  // the ledger (Debiteuren netted by the credit booking)
  const aging = run(['report', 'aging', '--as-of', '2026-12-31', '--kind', 'debtors', '--json']);
  const agingAcme = aging.out.data.debtors.contacts.find((c) => c.name === 'ACME B.V.');
  assert.equal(agingAcme.total_cents, 0);
  assert.equal(agingAcme.items[0].ref, '2026-0005');
  assert.equal(agingAcme.items[0].outstanding_cents, 0);

  // statement: S5 + credit note net to zero — ACME owes nothing
  const stmt = run(['contact', 'statement', '--id', '1', '--as-of', '2026-12-31', '--json']);
  assert.equal(stmt.out.data.balance_cents, 0);

  // month-end close check: read-only, no warnings of substance
  const me = run(['month-end', '--period', '2026-03', '--json']);
  assert.equal(me.out.data.period, '2026-03');
  assert.ok(Array.isArray(me.out.data.warnings));
});

// ---------------------------------------------------------------------------
// Stage 9 — Q2: continuity, second complete filing cycle
// ---------------------------------------------------------------------------

test('stage 9: Q2 — sale + purchase, second readout/file/settle cycle', () => {
  // P6 — hosting 21%
  run(['vat', 'book', '--date', '2026-04-08', '--desc', 'MijnHosting - hosting Q2', '--postings', '4340:80.00@21,1100:-96.80', '--post', '--json']);
  // S6 — ACME: 2x consultancy
  const s6 = run(['invoice', 'create', '--contact', '1', '--date', '2026-04-05', '--lines', '2x Consultancy @ 150.00 @21', '--json']);
  assert.equal(s6.out.data.invoice.gross_cents, 36300);
  const f7 = run(['invoice', 'finalize', '--id', '7', '--json']);
  assert.equal(f7.out.data.invoice.invoice_number, '2026-0007');

  const stmt = writeStmt('q2.xml', [
    { dir: 'DBIT', cents: 9680, date: '2026-04-08', party: 'MijnHosting', desc: 'hosting Q2', ref: 'NTRY011' },
    { dir: 'CRDT', cents: 36300, date: '2026-04-20', party: 'ACME B.V.', desc: '2026-0007', ref: 'NTRY012' },
  ]);
  run(['bank', 'import', '--file', stmt, '--iban', IBAN, '--json']);
  const match = run(['bank', 'match', 'auto', '--json']);
  assert.equal(match.out.data.matched.length, 2);
  assert.equal(match.out.data.unmatched_remaining, 0);

  // Q2 readout: only Q2 activity — Q1 stays isolated
  const r = run(['vat', 'readout', '--period', '2026-Q2', '--json']);
  const f = r.out.data.fields;
  assert.equal(f['1a'].cents, 30000);
  assert.equal(f['2a'].cents, 0);
  assert.equal(f['3a'].cents, 8000);
  assert.equal(f['5a'].cents, 6300);
  assert.equal(f['5b'].cents, 1680);
  assert.equal(f['5d'].cents, 4620);

  // file + settle Q2: owe 46.20, pay 46.00 -> 20-cent rounding gain
  const file = run(['vat', 'file', '--period', '2026-Q2', '--json']);
  assert.equal(file.out.data.liability_cents, 4620);
  assert.equal(tb('2510'), -4620);

  const stmt2 = writeStmt('q2-vat.xml', [
    { dir: 'DBIT', cents: 4600, date: '2026-04-25', party: 'Belastingdienst', desc: 'OB-aangifte 2026-Q2', ref: 'NTRY013' },
  ]);
  run(['bank', 'import', '--file', stmt2, '--iban', IBAN, '--json']);
  const txs = run(['bank', 'transactions', '--state', 'unmatched', '--json']);
  const vatTx = txs.out.data.transactions.find((t) => t.counterparty === 'Belastingdienst');
  const settle = run(['vat', 'settle', '--tx', String(vatTx.id), '--period', '2026-Q2', '--json']);
  assert.equal(settle.out.data.difference_cents, -20);
  assert.equal(tb('2510'), 0);
  assert.equal(tb('4700'), -26);
});

// ---------------------------------------------------------------------------
// Stage 10 — payables register + SEPA batch (pain.001)
// ---------------------------------------------------------------------------

test('stage 10: payables + SEPA batch — two suppliers in one pain.001', () => {
  run(['payments', 'payables', 'add', '--contact', '3', '--ref', 'KX-2026-014', '--date', '2026-02-03', '--due', '2026-03-03', '--amount', '121.00', '--json']);
  run(['payments', 'payables', 'add', '--contact', '4', '--ref', 'SH-2026-042', '--date', '2026-01-12', '--due', '2026-02-12', '--amount', '300.00', '--json']);

  const batch = run(['payments', 'batch', 'create', '--from-invoices', '--json']);
  assert.equal(batch.out.data.id, 1);
  assert.equal(batch.out.data.lines.length, 2);

  const exp = run(['payments', 'batch', 'export', '--id', '1', '--schema', '001.03', '--out', pain001Path, '--json']);
  assert.equal(exp.out.data.batch_id, 1);
  assert.ok(exp.out.data.msg_id);
  assert.ok(existsSync(pain001Path), 'pain.001 file must be written');
  const xml = readFileSync(pain001Path, 'utf8');
  assert.match(xml, /PmtInf/);

  // after the bank executed the file: confirm both payables paid
  run(['payments', 'payables', 'pay', '--id', '1', '--json']);
  run(['payments', 'payables', 'pay', '--id', '2', '--json']);
  const paid = run(['payments', 'payables', 'list', '--status', 'paid', '--json']);
  assert.equal(paid.out.data.payables.length, 2);
});

// ---------------------------------------------------------------------------
// Stage 11 — year-end: close, jaarrekening, ICP listing
// ---------------------------------------------------------------------------

test('stage 11: year-end close, jaarrekening micro, ICP readout', () => {
  const status = run(['year-end', 'status', '--year', '2026', '--json']);
  assert.equal(status.out.data.status.closed, false);
  assert.equal(status.out.data.status.result_cents, 5976);

  const plan = run(['year-end', 'close', '--year', '2026', '--dry-run', '--json']);
  assert.equal(plan.out.data.dryRun, true);
  const close = run(['year-end', 'close', '--year', '2026', '--json']);
  assert.equal(close.out.data.closed, true);
  // result 59.76 moves to equity: 25000.00 + 59.76
  assert.equal(tb('3000'), -(2500000 + 5976));
  assert.equal(tb('9900'), 0);

  // statutory micro accounts carry the same result
  const jk = run(['jaarrekening', 'report', '--year', '2026', '--model', 'micro', '--format', 'json', '--json']);
  assert.equal(jk.out.data.jaarrekening.balans.balanced, true);
  assert.equal(jk.out.data.jaarrekening.balans.total_activa_cents, 2505976);
  assert.equal(jk.out.data.jaarrekening.balans.total_passiva_cents, 2505976);

  // ICP: the verlegde EU levering to Berliner (2026-0003, 180.00)
  const icp = run(['icp', 'readout', '--period', '2026-Q1', '--json']);
  assert.equal(icp.out.data.total_cents, 18000);
  assert.equal(icp.out.data.customers[0].vat_id, 'DE123456789');
  assert.equal(icp.out.data.customers[0].amount_cents, 18000);
});

// ---------------------------------------------------------------------------
// Stage 12 — final books: everything reconciles, audit trail, backup
// ---------------------------------------------------------------------------

test('stage 12: final verification — balanced books, bank, audit, backup', () => {
  assert.equal(tb('1100'), 2505976); // 25000.00 + 1191.46 in - 1026.70 out - 105.00 VAT
  assert.equal(tb('1200'), 0);       // all customers settled (S5 netted by the credit note)
  assert.equal(tb('1500'), 0);
  assert.equal(tb('2500'), 0);
  assert.equal(tb('2510'), 0);

  // the audit trail names the actor for every mutation
  const audit = run(['audit', '--json']);
  assert.ok(audit.out.data.entries.length > 20);
  assert.ok(audit.out.data.entries.every((e) => typeof e.actor === 'string' && e.actor.length > 0));

  // consistent backup + restore target exists
  const backup = run(['backup', '--out', backupPath, '--json']);
  assert.equal(backup.out.ok, true);
  assert.ok(existsSync(backupPath));
});
