/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Agent-layer tests — FX translation, MCP server (real child process), compliance.
import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { openDb } from '../src/core/db.js';
import { seedDefaultChart } from '../src/core/accounts.js';
import { createEntry, postEntry, getEntry, reverseEntry } from '../src/core/entries.js';
import { enableVatModule, bookVatEntry, parseVatPostingSpecs, obReadout } from '../src/vat/index.js';
import {
  setFxRate, getFxRate, convertFx, parseRate, toEurPostings, listFxRates, resolveRate,
} from '../src/fx/index.js';
import { complianceStatus, markFiled, quarterDeadline, jaarrekeningDeadline } from '../src/compliance/index.js';
import { fetchEcbRate, parseSdmxObservations, setEcbFetcher, clearEcbFetcher } from '../src/fx/ecb.js';
import { addAsset } from '../src/assets/index.js';

const SDMX_USD = `<?xml version="1.0" encoding="UTF-8"?>
<message:GenericData xmlns:message="http://www.sdmx.org/resources/sdmxml/schemas/v2_1/message" xmlns:generic="http://www.sdmx.org/resources/sdmxml/schemas/v2_1/data/generic">
<message:DataSet>
<generic:Series>
<generic:SeriesKey><generic:Value id="FREQ" value="D"/><generic:Value id="CURRENCY" value="USD"/></generic:SeriesKey>
<generic:Obs><generic:ObsDimension value="2026-07-30"/><generic:ObsValue value="1.1476"/></generic:Obs>
<generic:Obs><generic:ObsDimension value="2026-07-31"/><generic:ObsValue value="1.1485"/></generic:Obs>
<generic:Obs><generic:ObsDimension value="2026-08-03"/><generic:ObsValue value="1.1515"/></generic:Obs>
</generic:Series>
</message:DataSet>
</message:GenericData>`;

function okStub(xml) {
  return async () => ({ ok: true, status: 200, text: async () => xml });
}

let db;

function setup({ vat = true } = {}) {
  db = openDb(':memory:');
  seedDefaultChart(db);
  db.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300',
            'Industrieweg 12', '2712 CD', 'Zoetermeer', ?)
  `).run(vat ? 1 : 0);
  if (vat) enableVatModule(db);
}

beforeEach(() => {
  setup();
});

// --- FX rate store + conversion math ---------------------------------------

test('fx: parseRate and convertFx — integer math, round-half-up', () => {
  assert.equal(parseRate('1.0875'), 10875);
  assert.equal(parseRate('1'), 10000);
  assert.equal(parseRate('0.9'), 9000);
  assert.equal(parseRate('1.087'), 10870);
  assert.throws(() => parseRate('abc'), { code: 'INVALID_RATE' });
  assert.throws(() => parseRate('-1.0'), { code: 'INVALID_RATE' });
  assert.throws(() => parseRate('1.08755'), { code: 'INVALID_RATE' }); // >4 decimals

  // 895.00 USD at 1.0875 -> 89500 * 10000 / 10875 = 82298.85 -> 82299
  assert.equal(convertFx(89500, 10875), 82299);
  // 1246.30 USD at 1.0875 -> 114602.29 -> 114602
  assert.equal(convertFx(124630, 10875), 114602);
});

test('fx: setFxRate upsert + audit; getFxRate exact then latest-on/before', () => {
  setFxRate(db, { currency: 'USD', date: '2026-07-01', rate: '1.08' });
  setFxRate(db, { currency: 'USD', date: '2026-07-10', rate: '1.09' });
  // upsert same date
  setFxRate(db, { currency: 'USD', date: '2026-07-01', rate: '1.081' });
  assert.equal(getFxRate(db, { currency: 'USD', date: '2026-07-01' }), 10810);
  assert.equal(getFxRate(db, { currency: 'USD', date: '2026-07-10' }), 10900);
  assert.equal(getFxRate(db, { currency: 'USD', date: '2026-07-05' }), 10810); // latest on/before
  assert.equal(getFxRate(db, { currency: 'USD', date: '2026-06-01' }), null); // before first
  assert.equal(getFxRate(db, { currency: 'GBP', date: '2026-07-05' }), null); // unknown currency
  const rates = listFxRates(db, { currency: 'USD' });
  assert.equal(rates.length, 2);
  // audited
  assert.equal(db.prepare("SELECT COUNT(*) c FROM audit_log WHERE action='fx.set'").get().c, 3);
  // validation
  assert.throws(() => setFxRate(db, { currency: 'usd', date: '2026-07-01', rate: '1.0' }), { code: 'INVALID_CURRENCY' });
  assert.throws(() => setFxRate(db, { currency: 'USD', date: 'bad', rate: '1.0' }), { code: 'INVALID_DATE' });
});

test('fx: toEurPostings attaches the original amounts', () => {
  const specs = [{ code: '4300', amountCents: 89500, vatCode: '21' }, { code: '1100', amountCents: -124630 }];
  const eur = toEurPostings(specs, { currency: 'USD', rateX10000: 10875 });
  assert.equal(eur[0].amountCents, 82299);
  assert.equal(eur[0].fxCurrency, 'USD');
  assert.equal(eur[0].fxAmountCents, 89500);
  assert.equal(eur[1].amountCents, -114602);
  assert.equal(eur[1].fxAmountCents, -124630);
});

// --- FX through the ledger --------------------------------------------------

test('fx: entry add with currency books EUR + keeps the original amounts (reversal too)', () => {
  const e = createEntry(db, {
    date: '2026-07-03', description: 'Factuur in USD',
    postings: toEurPostings(
      [{ code: '4300', amountCents: 89500 }, { code: '1100', amountCents: -89500 }],
      { currency: 'USD', rateX10000: 10875 },
    ),
    actor: 'agent:test',
  });
  const entry = postEntry(db, { id: e.id, actor: 'agent:test' });
  const p = entry.postings.find((x) => x.account_code === '4300');
  assert.equal(p.amount_cents, 82299); // EUR
  assert.equal(p.fx_currency, 'USD');
  assert.equal(p.fx_amount_cents, 89500);
  assert.equal(p.fx_amount, '895.00');
  assert.equal(entry.postings.find((x) => x.account_code === '1100').amount_cents, -82299);

  // reversal negates both the EUR and the FX amount
  const rev = reverseEntry(db, { id: entry.id, actor: 'agent:test' });
  const rp = rev.postings.find((x) => x.account_code === '4300');
  assert.equal(rp.amount_cents, -82299);
  assert.equal(rp.fx_amount_cents, -89500);
  assert.equal(rp.fx_currency, 'USD');
});

test('fx: vat book with currency — VAT legs computed on the EUR amounts', () => {
  setFxRate(db, { currency: 'USD', date: '2026-07-03', rate: '1.0875' });
  const specs = toEurPostings(
    parseVatPostingSpecs(['4300:895.00@21,1100:-1082.95']),
    { currency: 'USD', rateX10000: getFxRate(db, { currency: 'USD', date: '2026-07-03' }) },
  );
  const { entry } = bookVatEntry(db, {
    date: '2026-07-03', description: 'DeKantoor B.V. (USD)', postings: specs,
    actor: 'agent:test', post: true,
  });
  // 895.00 USD -> 822.99 EUR net; vat 21% = 172.83; gross 995.82 EUR
  const net = entry.postings.find((x) => x.account_code === '4300');
  assert.equal(net.amount_cents, 82299);
  assert.equal(net.fx_amount_cents, 89500);
  const vat = entry.postings.find((x) => x.vat_code !== null);
  assert.equal(vat.amount_cents, 82299);
  assert.equal(vat.vat_amount_cents, 17283);
  const bank = entry.postings.find((x) => x.account_code === '1100');
  assert.equal(bank.amount_cents, -99582);
  // the OB readout sees the EUR base
  const r = obReadout(db, { period: '2026-Q3' });
  assert.equal(r.fields['3a'], 82299);
  assert.equal(r.fields['5b'], 17283);
});

test('fx: invalid currency on a posting is rejected', () => {
  assert.throws(() => createEntry(db, {
    date: '2026-07-03', description: 'x',
    postings: [{ code: '4300', amountCents: 100, fxCurrency: 'usd', fxAmountCents: 100 }, { code: '1100', amountCents: -100 }],
    actor: 'a',
  }), { code: 'INVALID_FX_CURRENCY' });
  assert.throws(() => createEntry(db, {
    date: '2026-07-03', description: 'x',
    postings: [{ code: '4300', amountCents: 100, fxCurrency: 'USD', fxAmountCents: '100' }, { code: '1100', amountCents: -100 }],
    actor: 'a',
  }), { code: 'INVALID_FX_AMOUNT' });
});

// --- ECB reference rates ----------------------------------------------------

test('ecb: parses SDMX observations and falls back to the last business day', async () => {
  const obs = parseSdmxObservations(SDMX_USD, 'USD');
  assert.equal(obs.length, 3);
  assert.equal(obs[0].date, '2026-07-30');
  assert.equal(obs[2].rate, 1.1515);

  setEcbFetcher(okStub(SDMX_USD));
  try {
    // Saturday -> Friday rate
    const sat = await fetchEcbRate({ currency: 'USD', date: '2026-08-01' });
    assert.deepEqual(sat, { date: '2026-07-31', rateX10000: 11485 });
    // exact business day
    const mon = await fetchEcbRate({ currency: 'USD', date: '2026-08-03' });
    assert.deepEqual(mon, { date: '2026-08-03', rateX10000: 11515 });
  } finally {
    clearEcbFetcher();
  }
});

test('ecb: 404 (unknown currency) -> null; network failure -> ECB_FETCH_FAILED', async () => {
  setEcbFetcher(async () => ({ ok: false, status: 404, text: async () => '' }));
  try {
    assert.equal(await fetchEcbRate({ currency: 'XYZ', date: '2026-08-04' }), null);
  } finally {
    clearEcbFetcher();
  }
  setEcbFetcher(async () => { throw new Error('ENOTFOUND'); });
  try {
    await assert.rejects(() => fetchEcbRate({ currency: 'USD', date: '2026-08-04' }), { code: 'ECB_FETCH_FAILED' });
  } finally {
    clearEcbFetcher();
  }
});

test('fx: missing rate auto-fetches from ECB, stores it, and reuses it', async () => {
  let fetches = 0;
  setEcbFetcher(async () => { fetches += 1; return { ok: true, status: 200, text: async () => SDMX_USD }; });
  try {
    // no stored rate -> ECB fetch -> stored as source=ECB
    const r = await resolveRate(db, { currency: 'USD', date: '2026-08-01', actor: 'agent:test' });
    assert.equal(r, 11485);
    assert.equal(fetches, 1);
    assert.equal(getFxRate(db, { currency: 'USD', date: '2026-08-01' }), 11485);
    const row = db.prepare("SELECT * FROM fx_rates WHERE currency='USD' AND date='2026-07-31'").get();
    assert.equal(row.source, 'ECB');
    assert.equal(row.created_by, 'agent:test');
    // second booking: stored rate wins, no network
    const r2 = await resolveRate(db, { currency: 'USD', date: '2026-08-01', actor: 'agent:test' });
    assert.equal(r2, 11485);
    assert.equal(fetches, 1);
    // explicit --rate always wins
    const r3 = await resolveRate(db, { currency: 'USD', date: '2026-08-01', rate: '1.09' });
    assert.equal(r3, 10900);
    assert.equal(fetches, 1);
  } finally {
    clearEcbFetcher();
  }
});

test('fx: BUKIO_FX_NO_FETCH blocks the ECB fallback', async () => {
  const prev = process.env.BUKIO_FX_NO_FETCH;
  process.env.BUKIO_FX_NO_FETCH = '1';
  try {
    await assert.rejects(() => resolveRate(db, { currency: 'USD', date: '2026-08-01' }), { code: 'FX_RATE_NOT_FOUND' });
  } finally {
    if (prev === undefined) delete process.env.BUKIO_FX_NO_FETCH;
    else process.env.BUKIO_FX_NO_FETCH = prev;
  }
});

test('fx: ECB has no rate for the currency -> ECB_RATE_NOT_AVAILABLE', async () => {
  setEcbFetcher(okStub('<?xml version="1.0"?><message:GenericData></message:GenericData>'));
  try {
    await assert.rejects(() => resolveRate(db, { currency: 'USD', date: '2026-08-01', actor: 'a' }), { code: 'ECB_RATE_NOT_AVAILABLE' });
  } finally {
    clearEcbFetcher();
  }
});

// --- compliance -------------------------------------------------------------

test('compliance: quarterly deadlines', () => {
  assert.deepEqual(quarterDeadline('2026-Q1'), { period: '2026-Q1', deadline: '2026-04-30' });
  assert.deepEqual(quarterDeadline('2026-Q3'), { period: '2026-Q3', deadline: '2026-10-31' });
  assert.deepEqual(quarterDeadline('2026-Q4'), { period: '2026-Q4', deadline: '2027-01-31' });
  assert.throws(() => quarterDeadline('2026'), { code: 'INVALID_PERIOD' });
});

test('compliance: jaarrekening deadline is 13 months after the fiscal year end', () => {
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  assert.equal(jaarrekeningDeadline(company, 2026), '2028-01-31');
  db.prepare("UPDATE company SET fiscal_year_end = '06-30'").run();
  const c2 = db.prepare('SELECT * FROM company WHERE id = 1').get();
  assert.equal(jaarrekeningDeadline(c2, 2026), '2027-07-31');
  // tolerant parse: a full YYYY-MM-DD fiscal_year_end must not read the year
  // as the month (regression — the old first-part parse computed ~2195)
  db.prepare("UPDATE company SET fiscal_year_end = '2026-06-30'").run();
  const c3 = db.prepare('SELECT * FROM company WHERE id = 1').get();
  assert.equal(jaarrekeningDeadline(c3, 2026), '2027-07-31');
});

test('compliance: calendar shows obligations, statuses flip with filings', () => {
  const r = complianceStatus(db, { year: 2026 });
  const types = r.obligations.map((o) => o.type);
  assert.ok(types.includes('OB') && types.includes('ICP') && types.includes('JAARREKENING'));
  // 2026 calendar: Q1..Q4 OB+ICP (deadlines >= 2026-01-01) + prev-year Q4 OB/ICP + jaarrekening 2026 (+2025 if open)
  assert.ok(r.obligations.length >= 8);
  const obQ3 = r.obligations.find((o) => o.type === 'OB' && o.period === '2026-Q3');
  assert.equal(obQ3.deadline, '2026-10-31');
  assert.equal(obQ3.status, 'open'); // today is 2026-08-04

  // mark OB Q3 as filed -> status flips
  db.prepare("INSERT INTO vat_returns (type, period, status, fields_json, filed_at) VALUES ('OB','2026-Q3','filed','{}','2026-10-31')").run();
  const r2 = complianceStatus(db, { year: 2026 });
  assert.equal(r2.obligations.find((o) => o.type === 'OB' && o.period === '2026-Q3').status, 'filed');

  // mark ICP + jaarrekening via the registry
  markFiled(db, { type: 'ICP', period: '2026-Q3' });
  markFiled(db, { type: 'JAARREKENING', period: '2026' });
  const r3 = complianceStatus(db, { year: 2026 });
  assert.equal(r3.obligations.find((o) => o.type === 'ICP' && o.period === '2026-Q3').status, 'filed');
  assert.equal(r3.obligations.find((o) => o.type === 'JAARREKENING' && o.period === '2026').status, 'filed');
  // OB must use vat readout --mark-filed
  assert.throws(() => markFiled(db, { type: 'OB', period: '2026-Q3' }), { code: 'INVALID_TYPE' });
});

test('compliance: closed books show on the jaarrekening obligation', async () => {
  const e = createEntry(db, {
    date: '2026-03-01', description: 'Omzet',
    postings: [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }],
    actor: 'a',
  });
  postEntry(db, { id: e.id, actor: 'a' });
  const before = complianceStatus(db, { year: 2026 });
  assert.equal(before.obligations.find((o) => o.type === 'JAARREKENING' && o.period === '2026').books_closed, false);
  const { yearEndClose } = await import('../src/year-end/index.js');
  yearEndClose(db, { year: 2026 });
  const after = complianceStatus(db, { year: 2026 });
  assert.equal(after.obligations.find((o) => o.type === 'JAARREKENING' && o.period === '2026').books_closed, true);
});

// --- MCP server (real stdio child process) ----------------------------------

function mcpSession(dbPath) {
  const child = spawn(process.execPath, ['bin/bukio.js', 'mcp', '--db', dbPath], {
    cwd: process.cwd(),
    env: { ...process.env, BUKIO_ACTOR: 'agent:test' },
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
    raw(line) {
      child.stdin.write(`${line}\n`);
      return next();
    },
    close() {
      child.stdin.end();
      return new Promise((res) => child.on('exit', res));
    },
  };
}

test('MCP: initialize + tools/list + read-only calls work end-to-end', async () => {
  const e = createEntry(db, {
    date: '2026-07-01', description: 'Omzet',
    postings: [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }],
    actor: 'agent:test',
  });
  postEntry(db, { id: e.id, actor: 'agent:test' });

  const tmp = await import('node:fs/promises');
  const dir = await tmp.mkdtemp('/tmp/mcp-test-');
  const dbPath = `${dir}/x.db`;
  // fresh file db with the same content
  const fileDb = openDb(dbPath);
  seedDefaultChart(fileDb);
  fileDb.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300',
            'Industrieweg 12', '2712 CD', 'Zoetermeer', 1)
  `).run();
  enableVatModule(fileDb);
  const fe = createEntry(fileDb, {
    date: '2026-07-01', description: 'Omzet',
    postings: [{ code: '1100', amountCents: 12100 }, { code: '8000', amountCents: -10000 }, { code: '2500', amountCents: -2100 }],
    actor: 'agent:test',
  });
  postEntry(fileDb, { id: fe.id, actor: 'agent:test' });
  fileDb.close();

  const mcp = mcpSession(dbPath);
  try {
    const init = await mcp.call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'test', version: '1' } });
    assert.equal(init.result.serverInfo.name, 'bukio-cli');
    assert.equal(typeof init.result.capabilities.tools, 'object');

    const tools = await mcp.call('tools/list');
    const names = tools.result.tools.map((t) => t.name);
    assert.ok(names.includes('trial_balance'));
    assert.ok(names.includes('entry_add'));
    assert.ok(names.includes('vat_book'));
    assert.ok(names.includes('year_end_close'));
    assert.ok(names.includes('fx_set'));
    assert.ok(names.includes('compliance'));

    const tb = await mcp.call('tools/call', { name: 'trial_balance', arguments: {} });
    const data = JSON.parse(tb.result.content[0].text);
    assert.equal(data.balanced, true);
    assert.equal(tb.result.isError, false);

    const ci = await mcp.call('tools/call', { name: 'company_info', arguments: {} });
    assert.equal(JSON.parse(ci.result.content[0].text).company.name, 'Demo BV');

    const unknown = await mcp.call('tools/call', { name: 'nope', arguments: {} });
    assert.equal(unknown.error.code, -32602);

    // year is REQUIRED for pnl and journal — the schema must say so (a
    // client validating against the schema should catch it, not the server)
    const pnlTool = tools.result.tools.find((t) => t.name === 'pnl');
    assert.ok(pnlTool, 'pnl tool exists');
    assert.deepEqual(pnlTool.inputSchema.required, ['year']);
    const journalTool = tools.result.tools.find((t) => t.name === 'journal');
    assert.ok(journalTool, 'journal tool exists');
    assert.deepEqual(journalTool.inputSchema.required, ['year']);
  } finally {
    await mcp.close();
  }
});

test('MCP: non-object JSON-RPC messages get Invalid Request, server survives', async () => {
  const tmp = await import('node:fs/promises');
  const dir = await tmp.mkdtemp('/tmp/mcp-test-');
  const dbPath = `${dir}/x.db`;
  const fileDb = openDb(dbPath);
  seedDefaultChart(fileDb);
  fileDb.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300',
            'Industrieweg 12', '2712 CD', 'Zoetermeer', 1)
  `).run();
  enableVatModule(fileDb);
  fileDb.close();

  const mcp = mcpSession(dbPath);
  try {
    // valid JSON but not JSON-RPC request objects — must not crash the server
    const r1 = await mcp.raw('null');
    assert.equal(r1.error.code, -32600);
    assert.equal(r1.id, null);

    const r2 = await mcp.raw('42');
    assert.equal(r2.error.code, -32600);

    const r3 = await mcp.raw('[1,2]');
    assert.equal(r3.error.code, -32600);

    // the server is still alive and answers normally
    const init = await mcp.call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'test', version: '1' } });
    assert.equal(init.result.serverInfo.name, 'bukio-cli');
  } finally {
    await mcp.close();
  }
});

test('MCP: mutations are plan-only by default; execute books with the actor', async () => {
  const tmp = await import('node:fs/promises');
  const dir = await tmp.mkdtemp('/tmp/mcp-test-');
  const dbPath = `${dir}/x.db`;
  const fileDb = openDb(dbPath);
  seedDefaultChart(fileDb);
  fileDb.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300',
            'Industrieweg 12', '2712 CD', 'Zoetermeer', 1)
  `).run();
  enableVatModule(fileDb);
  fileDb.close();

  const mcp = mcpSession(dbPath);
  try {
    // dry-run: no write
    const plan = await mcp.call('tools/call', { name: 'entry_add', arguments: { date: '2026-07-05', description: 'Plan only', postings: ['1100:5000.00', '3000:-5000.00'] } });
    const planData = JSON.parse(plan.result.content[0].text);
    assert.equal(planData.mode, 'dry-run');
    assert.equal(planData.balanced, true);

    // execute without post: draft created
    const exec = await mcp.call('tools/call', { name: 'entry_add', arguments: { date: '2026-07-05', description: 'Echte boeking', postings: ['1100:5000.00', '3000:-5000.00'], mode: 'execute', actor: 'agent:mcp-test' } });
    const execData = JSON.parse(exec.result.content[0].text);
    assert.equal(execData.mode, 'execute');
    assert.equal(execData.state, 'draft');

    // post it
    const posted = await mcp.call('tools/call', { name: 'entry_post', arguments: { id: 1, mode: 'execute', actor: 'agent:mcp-test' } });
    assert.equal(JSON.parse(posted.result.content[0].text).state, 'posted');

    // audit trail shows the MCP actor
    const audit = await mcp.call('tools/call', { name: 'audit', arguments: { by: 'agent:mcp-test' } });
    const auditData = JSON.parse(audit.result.content[0].text);
    assert.ok(auditData.entries.length >= 2);

    // fx via MCP
    const fx = await mcp.call('tools/call', { name: 'fx_set', arguments: { currency: 'USD', date: '2026-07-10', rate: '1.09', mode: 'execute', actor: 'agent:mcp-test' } });
    assert.equal(JSON.parse(fx.result.content[0].text).rate, '1.0900');

    // invalid call -> isError
    const bad = await mcp.call('tools/call', { name: 'entry_add', arguments: { date: '2026-07-05', description: 'x', postings: ['1100:1.00'], mode: 'execute' } });
    assert.equal(bad.result.isError, true);
  } finally {
    await mcp.close();
  }
});

test('MCP: assets_run books DEPRECIATION, not recurring entries (import-collision regression)', async () => {
  const tmp = await import('node:fs/promises');
  const dir = await tmp.mkdtemp('/tmp/mcp-test-');
  const dbPath = `${dir}/x.db`;
  const fileDb = openDb(dbPath);
  seedDefaultChart(fileDb);
  fileDb.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300',
            'Industrieweg 12', '2712 CD', 'Zoetermeer', 0)
  `).run();
  // asset with depreciation due for 2026-01 (5y linear on 120000 = 2000/mo)
  addAsset(fileDb, {
    name: 'Laptop', purchaseDate: '2025-12-15', purchasePriceCents: 120000,
    depreciationStartDate: '2026-01-01', recognitionDate: '2026-01-01',
    assetAccount: '1800', expenseAccount: '4600', actor: 'agent:test',
  });
  fileDb.close();

  const mcp = mcpSession(dbPath);
  try {
    await mcp.call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'test', version: '1' } });
    const r = await mcp.call('tools/call', {
      name: 'assets_run',
      arguments: { period: '2026-01', mode: 'execute', actor: 'agent:mcp-test' },
    });
    assert.equal(r.result.isError, false);
    const data = JSON.parse(r.result.content[0].text);
    assert.equal(data.mode, 'execute');
    assert.equal(data.booked.length, 1);

    const db2 = openDb(dbPath);
    try {
      const sources = db2.prepare('SELECT DISTINCT source FROM journal_entries').all().map((x) => x.source);
      assert.ok(sources.includes('assets'), 'a depreciation entry (source=assets) must be booked');
      assert.ok(!sources.includes('recurring'), 'assets_run must NOT generate recurring entries');
    } finally {
      db2.close();
    }
  } finally {
    await mcp.close();
  }
});

test('MCP: contact_add preserves postal_code and vat_id (regression)', async () => {
  const tmp = await import('node:fs/promises');
  const dir = await tmp.mkdtemp('/tmp/mcp-test-');
  const dbPath = `${dir}/x.db`;
  const fileDb = openDb(dbPath);
  seedDefaultChart(fileDb);
  fileDb.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300',
            'Industrieweg 12', '2712 CD', 'Zoetermeer', 0)
  `).run();
  fileDb.close();

  const mcp = mcpSession(dbPath);
  try {
    await mcp.call('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'test', version: '1' } });
    const r = await mcp.call('tools/call', {
      name: 'contact_add',
      arguments: {
        name: 'Acme GmbH', address: 'Leverstrasse 1', postal_code: '80331',
        city: 'München', country: 'DE', vat_id: 'DE123456789', mode: 'execute', actor: 'agent:mcp-test',
      },
    });
    assert.equal(r.result.isError, false);
  } finally {
    await mcp.close();
  }

  const check = openDb(dbPath);
  try {
    const row = check.prepare('SELECT name, address, postal_code, city, country, vat_id FROM contacts WHERE name = ?').get('Acme GmbH');
    assert.ok(row, 'contact must exist');
    assert.equal(row.address, 'Leverstrasse 1');
    assert.equal(row.postal_code, '80331');
    assert.equal(row.city, 'München');
    assert.equal(row.country, 'DE');
    assert.equal(row.vat_id, 'DE123456789');
  } finally {
    check.close();
  }
});

test('MCP: BUKIO_MCP_READONLY blocks execution', async () => {
  const tmp = await import('node:fs/promises');
  const dir = await tmp.mkdtemp('/tmp/mcp-test-');
  const dbPath = `${dir}/x.db`;
  const fileDb = openDb(dbPath);
  seedDefaultChart(fileDb);
  fileDb.prepare(`
    INSERT INTO company (name, kvk, legal_form, btw_id, iban, address, postal_code, city, vat_module)
    VALUES ('Demo BV', '12345678', 'bv', 'NL123456789B01', 'NL91ABNA0417164300',
            'Industrieweg 12', '2712 CD', 'Zoetermeer', 1)
  `).run();
  enableVatModule(fileDb);
  fileDb.close();

  const child = spawn(process.execPath, ['bin/bukio.js', 'mcp', '--db', dbPath], {
    cwd: process.cwd(),
    env: { ...process.env, BUKIO_ACTOR: 'agent:test', BUKIO_MCP_READONLY: '1' },
  });
  let buf = '';
  const waiters = [];
  const pending = [];
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
  try {
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'entry_add', arguments: { date: '2026-07-05', description: 'x', postings: ['1100:1.00', '3000:-1.00'], mode: 'execute' } } })}\n`);
    const res = await next();
    assert.equal(res.result.isError, true);
    const data = JSON.parse(res.result.content[0].text);
    assert.equal(data.error.code, 'MCP_READONLY');
  } finally {
    child.stdin.end();
    await new Promise((res) => child.on('exit', res));
  }
});

test('fx resolveRate: a dry-run must not persist the fetched ECB rate', async () => {
  setEcbFetcher(okStub(SDMX_USD));
  try {
    const rate = await resolveRate(db, { currency: 'USD', date: '2026-08-03', dryRun: true });
    assert.equal(rate, 11515);
    // nothing written: no fx_rates row, no audit row
    assert.equal(listFxRates(db).length, 0, 'dry-run must not INSERT an fx_rates row');
    assert.equal(db.prepare("SELECT COUNT(*) c FROM audit_log WHERE action = 'fx.set'").get().c, 0);
    // the execute path still persists
    const rate2 = await resolveRate(db, { currency: 'USD', date: '2026-08-03' });
    assert.equal(rate2, 11515);
    assert.equal(listFxRates(db).length, 1, 'execute persists the fetched rate');
  } finally {
    clearEcbFetcher();
  }
});
