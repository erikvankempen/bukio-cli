// MCP server (Phase 5) — Model Context Protocol over stdio (JSON-RPC 2.0,
// newline-delimited). Lets agents (Hermes, Claude, ...) drive bukio natively:
// read-only introspection tools plus mutation tools that default to dry-run
// plans — an agent executes only after a human approves the plan.
// Env: BUKIO_MCP_READONLY=1 turns every mutation into a plan-only call.
import { createInterface } from 'node:readline';
import { existsSync } from 'node:fs';
import { trialBalance } from '../report/trial-balance.js';
import { balans } from '../report/balans.js';
import { pnl } from '../report/pnl.js';
import { journal } from '../report/journal.js';
import { listAccounts } from '../core/accounts.js';
import { dbError } from './util.js';
import {
  createEntry, postEntry, reverseEntry, getEntry, resolvePostings,
  parsePostingSpecs, validateDate,
} from '../core/entries.js';
import { bookVatEntry, expandVatPostings, obReadout, parseVatPostingSpecs } from '../vat/index.js';
import {
  createContact, createInvoice, finalizeInvoice, creditInvoice, markPaid, getInvoice,
} from '../invoice/index.js';
import { runDue, previewDue } from '../recurring/index.js';
import { register, addAsset, runDue as assetsRunDue, disposeAsset } from '../assets/index.js';
import { yearEndClose, yearEndStatus } from '../year-end/index.js';
import { setFxRate, getFxRate, parseRate, toEurPostings, resolveRate } from '../fx/index.js';
import { icpReadout } from '../icp/index.js';
import { list as auditList } from '../audit/index.js';
import { complianceStatus } from '../compliance/index.js';
import { parseImportAmount } from '../import/index.js';
import { openDb } from '../core/db.js';
import { isValidActor } from '../core/actor.js';

const PROTOCOL_VERSION = '2024-11-05';

class McpError extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

function json(content) {
  return [{ type: 'text', text: JSON.stringify(content, null, 2) }];
}

/** Shared mutation gate: default plan-only; execute needs approval. */
function modeOf(args) {
  return args?.mode === 'execute' ? 'execute' : 'dry-run';
}

function guardExecute(ctx, args) {
  if (ctx.readonly && modeOf(args) === 'execute') {
    throw new McpError('MCP_READONLY', 'this server is read-only (BUKIO_MCP_READONLY=1) — no mutations allowed');
  }
  if (modeOf(args) === 'execute' && args.actor && !isValidActor(args.actor)) {
    throw new McpError('INVALID_ACTOR', `invalid actor '${args.actor}' — must be '<role>:<name>' (e.g. agent:bartholomeus)`);
  }
}

function fmtMoney(cents) {
  return (cents / 100).toFixed(2);
}

// --- tool registry ---------------------------------------------------------

const TOOLS = [];

function tool({ name, description, schema, handler, mutating = false }) {
  TOOLS.push({ name, description, inputSchema: schema, handler, mutating });
}

// read tools
tool({
  name: 'company_info', description: 'the company behind this database', schema: { type: 'object', properties: {} },
  handler: (db) => ({ company: db.prepare('SELECT * FROM company WHERE id = 1').get() ?? null }),
});
tool({
  name: 'trial_balance', description: 'per-account totals; balanced tells you the books reconcile', schema: {
    type: 'object', properties: { year: { type: 'string' } },
  },
  handler: (db, args) => {
    if (args.year != null && !/^\d{4}$/.test(String(args.year))) throw new McpError('INVALID_YEAR', `year '${args.year}' must be YYYY`);
    return trialBalance(db, { year: args.year ?? null });
  },
});
tool({
  name: 'balans', description: 'balance sheet as of a date (balanced must be true)', schema: {
    type: 'object', properties: { as_of: { type: 'string', description: 'YYYY-MM-DD' } },
  },
  handler: (db, args) => balans(db, { asOf: args.as_of }),
});
tool({
  name: 'pnl', description: 'profit & loss for a year (excludes closing entries)', schema: {
    type: 'object', properties: { year: { type: 'string' } },
  },
  handler: (db, args) => {
    const year = String(args.year ?? '');
    if (!/^\d{4}$/.test(year)) throw new McpError('INVALID_YEAR', `year '${args.year}' must be YYYY`);
    return pnl(db, { from: `${year}-01-01`, to: `${year}-12-31` });
  },
});
tool({
  name: 'journal', description: 'journal export for a year (complete unless a limit is given; truncated: true when rows were cut)', schema: {
    type: 'object', properties: { year: { type: 'string' }, limit: { type: 'number' } },
  },
  handler: (db, args) => {
    const year = String(args.year ?? '');
    if (!/^\d{4}$/.test(year)) throw new McpError('INVALID_YEAR', `year '${args.year}' must be YYYY`);
    const limit = args.limit ?? 500;
    if (!Number.isInteger(limit) || limit < 0) throw new McpError('INVALID_LIMIT', `limit must be a non-negative integer, got '${limit}'`);
    const rows = journal(db, { from: `${year}-01-01`, to: `${year}-12-31`, limit: limit + 1 });
    const truncated = rows.length > limit;
    return { rows: truncated ? rows.slice(0, limit) : rows, truncated, limit };
  },
});
tool({
  name: 'accounts', description: 'chart of accounts', schema: { type: 'object', properties: {} },
  handler: (db) => ({ accounts: listAccounts(db) }),
});
tool({
  name: 'vat_readout', description: 'OB-aangifte fields 1a-5d for manual filing (never auto-files)', schema: {
    type: 'object', properties: { period: { type: 'string', description: 'YYYY-Qn or YYYY-MM' } },
  },
  handler: (db, args) => obReadout(db, { period: args.period }),
});
tool({
  name: 'icp_readout', description: 'ICP listing: EU btw-verlegde supplies per customer', schema: {
    type: 'object', properties: { period: { type: 'string', description: 'YYYY-Qn' } },
  },
  handler: (db, args) => icpReadout(db, { period: args.period }),
});
tool({
  name: 'audit', description: 'append-only audit log', schema: {
    type: 'object', properties: { by: { type: 'string' }, since: { type: 'string' }, limit: { type: 'number' } },
  },
  handler: (db, args) => ({ entries: auditList(db, { since: args.since ?? null, actor: args.by ?? null, limit: args.limit ?? 50 }) }),
});
tool({
  name: 'compliance', description: 'compliance calendar for a year (OB/ICP deadlines, jaarrekening deposit)', schema: {
    type: 'object', properties: { year: { type: 'string' } },
  },
  handler: (db, args) => {
    if (args.year != null && !/^\d{4}$/.test(String(args.year))) throw new McpError('INVALID_YEAR', `year '${args.year}' must be YYYY`);
    return complianceStatus(db, { year: args.year });
  },
});
tool({
  name: 'invoices', description: 'outstanding + recent invoices', schema: {
    type: 'object', properties: { limit: { type: 'number' } },
  },
  handler: (db, args) => {
    const limit = args.limit ?? 50;
    if (!Number.isInteger(limit) || limit < 0) throw new McpError('INVALID_LIMIT', `limit must be a non-negative integer, got '${limit}'`);
    return {
      invoices: db.prepare('SELECT * FROM invoices ORDER BY id DESC LIMIT ?').all(limit),
    };
  },
});

// mutation tools (plan-only unless mode=execute)
tool({
  name: 'entry_add', mutating: true,
  description: 'create a journal entry. mode=dry-run (default) shows the resolved plan; mode=execute posts it (post=true posts immediately). actor: agent:<name>',
  schema: {
    type: 'object', properties: {
      date: { type: 'string' }, description: { type: 'string' },
      postings: { type: 'array', items: { type: 'string' }, description: 'CODE:AMOUNT specs' },
      source: { type: 'string' }, source_ref: { type: 'string' },
      currency: { type: 'string' }, rate: { type: 'string' },
      post: { type: 'boolean' }, actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['date', 'description', 'postings'],
  },
  handler: async (db, args, ctx) => {
    guardExecute(ctx, args);
    const specs = parsePostingSpecs(args.postings);
    const converted = args.currency ? toEurPostings(specs, {
      currency: args.currency,
      rateX10000: await resolveRate(db, { currency: args.currency, rate: args.rate, date: args.date, actor: args.actor ?? ctx.actor }),
    }) : specs;
    const resolved = resolvePostings(db, converted);
    const sum = resolved.reduce((s, p) => s + p.amountCents, 0);
    // validate the DB-free invariants in dry-run too — the old plan echoed
    // garbage dates / single postings / unbalanced sums as a green plan
    // (createEntry caught them only on execute; the agent saw isError:false)
    validateDate(args.date);
    if (!args.description || !String(args.description).trim()) {
      throw new McpError('INVALID_DESCRIPTION', 'description is required');
    }
    if (resolved.length < 2) {
      throw new McpError('TOO_FEW_POSTINGS', 'an entry needs at least 2 postings');
    }
    if (sum !== 0) {
      throw new McpError('UNBALANCED', `postings do not sum to zero (sum = ${sum} cents)`);
    }
    const plan = {
      action: 'entry.add', date: args.date, description: args.description,
      currency: args.currency ?? null,
      postings: resolved.map((p) => ({
        code: p.code, amount_cents: p.amountCents, amount: fmtMoney(p.amountCents),
        fx_currency: p.fxCurrency, fx_amount_cents: p.fxAmountCents,
      })),
      sum_cents: sum, balanced: sum === 0,
    };
    if (modeOf(args) === 'dry-run') return { ...plan, mode: 'dry-run', note: 'plan only — re-run with mode=execute to book' };
    const actor = args.actor ?? ctx.actor;
    let entry = createEntry(db, {
      date: args.date, description: args.description,
      postings: converted.map((p) => ({ code: p.code, amountCents: p.amountCents, fxCurrency: p.fxCurrency, fxAmountCents: p.fxAmountCents })),
      source: args.source ?? 'agent', sourceRef: args.source_ref ?? null, actor,
    });
    if (args.post) entry = postEntry(db, { id: entry.id, actor });
    return { ...plan, mode: 'execute', entry_id: entry.id, state: entry.state };
  },
});
tool({
  name: 'entry_post', mutating: true,
  description: 'post a draft entry (mode: dry-run shows the transition)',
  schema: { type: 'object', properties: { id: { type: 'number' }, actor: { type: 'string' }, mode: { type: 'string' } }, required: ['id'] },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const entry = getEntry(db, args.id);
    if (!entry) throw new McpError('NOT_FOUND', `entry ${args.id} does not exist`);
    if (modeOf(args) === 'dry-run') {
      // validate the transition like the real path (postEntry rejects
      // already-posted/reversed entries)
      if (entry.state !== 'draft') {
        throw new McpError(entry.state === 'posted' ? 'ALREADY_POSTED' : 'ALREADY_REVERSED', `entry ${args.id} is ${entry.state}`);
      }
      return { action: 'entry.post', id: args.id, from: entry.state, to: 'posted', mode: 'dry-run' };
    }
    const posted = postEntry(db, { id: args.id, actor: args.actor ?? ctx.actor });
    return { action: 'entry.post', id: args.id, state: posted.state, mode: 'execute' };
  },
});
tool({
  name: 'entry_reverse', mutating: true,
  description: 'reverse a posted entry with a contra-entry (never deletes)',
  schema: { type: 'object', properties: { id: { type: 'number' }, reason: { type: 'string' }, actor: { type: 'string' }, mode: { type: 'string' } }, required: ['id'] },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    if (modeOf(args) === 'dry-run') {
      // validate like the real path (reverseEntry dryRun) — the old plan
      // echoed ok for nonexistent/draft entries
      return reverseEntry(db, { id: args.id, reason: args.reason ?? null, dryRun: true });
    }
    const reversed = reverseEntry(db, { id: args.id, actor: args.actor ?? ctx.actor, reason: args.reason });
    return { action: 'entry.reverse', id: args.id, reversal_id: reversed.id, state: reversed.state, mode: 'execute' };
  },
});
tool({
  name: 'vat_book', mutating: true,
  description: 'book a VAT-aware entry — tag net postings with @VATCODE. Currency converts specs from a foreign currency into EUR.',
  schema: {
    type: 'object', properties: {
      date: { type: 'string' }, description: { type: 'string' },
      postings: { type: 'array', items: { type: 'string' }, description: 'CODE:AMOUNT[@VATCODE]' },
      currency: { type: 'string' }, rate: { type: 'string' },
      post: { type: 'boolean' }, actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['date', 'description', 'postings'],
  },
  handler: async (db, args, ctx) => {
    guardExecute(ctx, args);
    const specs = parseVatPostingSpecs(args.postings);
    const converted = args.currency ? toEurPostings(specs, {
      currency: args.currency,
      rateX10000: await resolveRate(db, { currency: args.currency, rate: args.rate, date: args.date, actor: args.actor ?? ctx.actor }),
    }) : specs;
    const expanded = expandVatPostings(db, converted);
    // validate the DB-free invariants in dry-run too (parity with entry_add):
    // expandVatPostings validated codes/accounts, but date/description/count
    // only surfaced on execute before
    validateDate(args.date);
    if (!args.description || !String(args.description).trim()) {
      throw new McpError('INVALID_DESCRIPTION', 'description is required');
    }
    if (expanded.length < 2) {
      throw new McpError('TOO_FEW_POSTINGS', 'an entry needs at least 2 postings');
    }
    const plan = {
      action: 'vat.book', date: args.date, description: args.description, currency: args.currency ?? null,
      postings: expanded.map((p) => ({
        code: p.code, amount_cents: p.amountCents, vat_code: p.vatCode ?? null,
        vat_amount_cents: p.vatAmountCents ?? null,
        fx_currency: p.fxCurrency ?? null, fx_amount_cents: p.fxAmountCents ?? null,
      })),
    };
    if (modeOf(args) === 'dry-run') return { ...plan, mode: 'dry-run', note: 'plan only — re-run with mode=execute to book' };
    const { entry } = bookVatEntry(db, {
      date: args.date, description: args.description, postings: converted,
      source: 'agent', sourceRef: args.source_ref ?? null,
      actor: args.actor ?? ctx.actor,
      // same convention as the CLI and entry_add: post only when explicitly asked
      post: args.post === true,
    });
    return { ...plan, mode: 'execute', entry_id: entry.id, state: entry.state };
  },
});
tool({
  name: 'invoice_create', mutating: true,
  description: 'create a draft invoice (contact id + line specs like "2x Dienst @ 150.00 @21")',
  schema: {
    type: 'object', properties: {
      contact_id: { type: 'number' }, lines: { type: 'array', items: { type: 'string' } },
      date: { type: 'string' }, due_days: { type: 'number' }, actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['contact_id', 'lines'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    if (modeOf(args) === 'dry-run') {
      // validate like the real path (createInvoice dryRun) — the old plan
      // echoed ok for garbage dates and nonexistent contacts
      const plan = createInvoice(db, {
        contactId: args.contact_id, lines: args.lines,
        date: args.date ?? new Date().toISOString().slice(0, 10),
        dueDays: args.due_days ?? 30, actor: args.actor ?? ctx.actor, dryRun: true,
      });
      return { ...plan, mode: 'dry-run', note: 'plan only — re-run with mode=execute to book' };
    }
    const inv = createInvoice(db, {
      contactId: args.contact_id, lines: args.lines, date: args.date ?? new Date().toISOString().slice(0, 10),
      dueDays: args.due_days ?? 30, actor: args.actor ?? ctx.actor,
    });
    return {
      action: 'invoice.create', contact_id: args.contact_id, lines: args.lines, date: inv.date,
      mode: 'execute', invoice_id: inv.id, invoice_number: null, status: 'draft',
      totals: { net: inv.net_cents, vat: inv.vat_cents, gross: inv.gross_cents },
    };
  },
});
tool({
  name: 'invoice_finalize', mutating: true,
  description: 'finalize a draft: sequential number + booking entry (validates the 12 factuurvereisten)',
  schema: { type: 'object', properties: { id: { type: 'number' }, actor: { type: 'string' }, mode: { type: 'string' } }, required: ['id'] },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    if (modeOf(args) === 'dry-run') {
      // validate like the real path (finalizeInvoice dryRun: existence,
      // status, the 12 factuurvereisten) — the old plan echoed ok for
      // nonexistent/draft-incomplete invoices
      return finalizeInvoice(db, { id: args.id, actor: args.actor ?? ctx.actor, dryRun: true });
    }
    const inv = finalizeInvoice(db, { id: args.id, actor: args.actor ?? ctx.actor });
    return { action: 'invoice.finalize', id: args.id, invoice_number: inv.invoice_number, entry_id: inv.entry_id, mode: 'execute' };
  },
});
tool({
  name: 'invoice_credit', mutating: true,
  description: 'credit note for a sales invoice (draft credit; finalize separately)',
  schema: { type: 'object', properties: { id: { type: 'number' }, reason: { type: 'string' }, actor: { type: 'string' }, mode: { type: 'string' } }, required: ['id'] },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    if (modeOf(args) === 'dry-run') {
      // validate like the real path (creditInvoice dryRun) — the old plan
      // echoed ok for nonexistent/unfinalized invoices
      return creditInvoice(db, { id: args.id, reason: args.reason ?? null, dryRun: true });
    }
    const credit = creditInvoice(db, { id: args.id, reason: args.reason, actor: args.actor ?? ctx.actor });
    return { action: 'invoice.credit', id: args.id, credit_id: credit.id, mode: 'execute' };
  },
});
tool({
  name: 'invoice_pay', mutating: true,
  description: 'record a payment on an invoice (partial ok; overpayment rejected)',
  schema: {
    type: 'object', properties: {
      id: { type: 'number' }, date: { type: 'string' }, amount_cents: { type: 'number' }, method: { type: 'string' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['id', 'date'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    // amount omitted -> full outstanding, same as the CLI default
    const invoice = getInvoice(db, args.id) ?? null;
    const amountCents = args.amount_cents != null
      ? args.amount_cents
      : (invoice ? invoice.gross_cents - invoice.paid_cents : null);
    if (modeOf(args) === 'dry-run') {
      // validate like the real path (markPaid dryRun: invoice, status, date,
      // amount, overpayment) — the old plan echoed ok for garbage
      return markPaid(db, { id: args.id, date: args.date, amountCents, method: args.method ?? 'bank', dryRun: true });
    }
    const inv = markPaid(db, { id: args.id, date: args.date, amountCents, method: args.method, actor: args.actor ?? ctx.actor });
    return { action: 'invoice.pay', id: args.id, status: inv.status, mode: 'execute' };
  },
});
tool({
  name: 'recurring_run', mutating: true,
  description: 'generate due recurring entries / draft invoices (idempotent, backfills)',
  schema: {
    type: 'object', properties: { as_of: { type: 'string' }, template_id: { type: 'number' }, actor: { type: 'string' }, mode: { type: 'string' } },
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const fn = modeOf(args) === 'dry-run' ? previewDue : runDue;
    return fn(db, { asOf: args.as_of ?? null, templateId: args.template_id ?? null, actor: args.actor ?? ctx.actor });
  },
});
tool({
  name: 'year_end_close', mutating: true,
  description: 'close the fiscal year (result -> 9900 -> 3000); mode=dry-run shows the plan',
  schema: { type: 'object', properties: { year: { type: 'string' }, actor: { type: 'string' }, mode: { type: 'string' } }, required: ['year'] },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    if (modeOf(args) === 'dry-run') return yearEndClose(db, { year: args.year, dryRun: true });
    return yearEndClose(db, { year: args.year, actor: args.actor ?? ctx.actor });
  },
});
tool({
  name: 'year_end_status', description: 'is the year closed? what is the result?',
  schema: { type: 'object', properties: { year: { type: 'string' } }, required: ['year'] },
  handler: (db, args) => yearEndStatus(db, { year: args.year }),
});
tool({
  name: 'fx_set', mutating: true,
  description: 'store an FX rate (1 EUR = n units of currency on a date)',
  schema: {
    type: 'object', properties: { currency: { type: 'string' }, date: { type: 'string' }, rate: { type: 'string' }, actor: { type: 'string' }, mode: { type: 'string' } },
    required: ['currency', 'date', 'rate'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    // setFxRate validates currency/date (incl. calendar round-trip) and
    // returns the dry-run plan itself — the old branch skipped those checks
    return setFxRate(db, {
      currency: args.currency, date: args.date, rate: parseRate(args.rate),
      actor: args.actor ?? ctx.actor, dryRun: modeOf(args) === 'dry-run',
    });
  },
});
tool({
  name: 'contact_add', mutating: true,
  description: 'register an invoice counterparty (EU customers need a btw-id for verlegd/ICP)',
  schema: {
    type: 'object', properties: {
      name: { type: 'string' }, address: { type: 'string' }, postal_code: { type: 'string' },
      city: { type: 'string' }, country: { type: 'string' }, vat_id: { type: 'string' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['name'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    if (modeOf(args) === 'dry-run') {
      // validate like the real path (createContact rejects a missing name)
      if (!args.name || !String(args.name).trim()) {
        throw new McpError('INVALID_NAME', 'contact needs a name');
      }
      return { action: 'contact.add', name: args.name, mode: 'dry-run' };
    }
    const c = createContact(db, { ...args, actor: args.actor ?? ctx.actor });
    return { action: 'contact.add', id: c.id, name: c.name, mode: 'execute' };
  },
});

// --- fixed assets ------------------------------------------------------------

tool({
  name: 'assets_register', description: 'activastaat: cost, cumulative depreciation, book value per asset',
  schema: { type: 'object', properties: { as_of: { type: 'string' } } },
  handler: (db, args) => register(db, { asOf: args.as_of ?? null, actor: 'agent' }),
});

tool({
  name: 'asset_add', mutating: true,
  description: 'register an asset (already booked in the ledger) — mid-life adoption via recognition-date + cum-dep at recognition',
  schema: {
    type: 'object', properties: {
      name: { type: 'string' }, category: { type: 'string' }, serial: { type: 'string' },
      purchase_date: { type: 'string' }, purchase_price: { type: 'string' },
      depreciation_start: { type: 'string' }, recognition_date: { type: 'string' },
      cum_dep: { type: 'string' }, residual: { type: 'string' },
      scheme: { type: 'string' }, method: { type: 'string' }, life_months: { type: 'string' },
      asset_account: { type: 'string' }, cum_dep_account: { type: 'string' },
      expense_account: { type: 'string' }, entry_id: { type: 'string' }, note: { type: 'string' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['name', 'purchase_date', 'purchase_price', 'depreciation_start', 'recognition_date'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    // money as strict integer cents — parseImportAmount accepts '1234.56',
    // '1234,56' and '1.234,56' and rejects garbage (never parseFloat: a
    // Dutch comma would silently book 12.00 for '12,34')
    const toCents = (s) => (s == null || s === '' ? 0 : parseImportAmount(String(s)));
    const common = {
      name: args.name, category: args.category, serial: args.serial,
      purchaseDate: args.purchase_date, purchasePriceCents: toCents(args.purchase_price),
      depreciationStartDate: args.depreciation_start, recognitionDate: args.recognition_date,
      cumDepAtRecognitionCents: toCents(args.cum_dep), residualCents: args.residual !== undefined ? toCents(args.residual) : null,
      schemeId: args.scheme ? Number(args.scheme) : null,
      method: args.method, lifeMonths: args.life_months ? Number(args.life_months) : null,
      assetAccount: args.asset_account ?? '1800', cumDepAccount: args.cum_dep_account,
      expenseAccount: args.expense_account ?? '4600',
      entryId: args.entry_id ? Number(args.entry_id) : null, note: args.note,
    };
    if (modeOf(args) === 'dry-run') {
      const r = addAsset(db, { ...common, actor: args.actor ?? ctx.actor, dryRun: true });
      return { action: 'asset.add', mode: 'dry-run', asset: r.asset };
    }
    const r = addAsset(db, { ...common, actor: args.actor ?? ctx.actor, dryRun: false });
    return { action: 'asset.add', id: r.asset.id, name: r.asset.name, warnings: r.warnings, mode: 'execute' };
  },
});

tool({
  name: 'assets_run', mutating: true,
  description: 'book the depreciation runs that are due (idempotent per asset-month)',
  schema: { type: 'object', properties: { period: { type: 'string' }, as_of: { type: 'string' }, actor: { type: 'string' }, mode: { type: 'string' } } },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const actor = args.actor ?? ctx.actor;
    if (modeOf(args) === 'dry-run') {
      const r = runDue(db, { period: args.period, asOf: args.as_of, actor, dryRun: true });
      return { action: 'assets.run', mode: 'dry-run', plan: r.plan };
    }
    const r = runDue(db, { period: args.period, asOf: args.as_of, actor, dryRun: false });
    return { action: 'assets.run', booked: r.booked, mode: 'execute' };
  },
});

tool({
  name: 'asset_dispose', mutating: true,
  description: 'dispose of an asset (sale or scrap): books the full entry, status -> disposed',
  schema: {
    type: 'object', properties: {
      id: { type: 'string' }, date: { type: 'string' }, proceeds: { type: 'string' },
      bank_account: { type: 'string' }, result_account: { type: 'string' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['id', 'date'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const actor = args.actor ?? ctx.actor;
    const common = {
      id: Number(args.id), date: args.date,
      proceedsCents: (() => {
        const s = args.proceeds;
        return s == null || s === '' ? 0 : parseImportAmount(String(s));
      })(),
      bankAccount: args.bank_account, resultAccount: args.result_account,
    };
    if (modeOf(args) === 'dry-run') {
      const r = disposeAsset(db, { ...common, actor, dryRun: true });
      return { action: 'asset.dispose', mode: 'dry-run', asset: r.asset, postings: r.postings, result_cents: r.result_cents };
    }
    const r = disposeAsset(db, { ...common, actor, dryRun: false });
    return { action: 'asset.dispose', mode: 'execute', asset: r.asset, entry_id: r.entry.id, result_cents: r.result_cents };
  },
});

// --- JSON-RPC plumbing -----------------------------------------------------

function rpcResponse(id, result) {
  return JSON.stringify({ jsonrpc: '2.0', id, result });
}

function rpcError(id, code, message) {
  return JSON.stringify({ jsonrpc: '2.0', id, error: { code, message } });
}

function dispatch(db, ctx, msg) {
  const { id, method, params = {} } = msg;
  switch (method) {
    case 'initialize':
      return Promise.resolve(rpcResponse(id, {
        protocolVersion: PROTOCOL_VERSION,
        capabilities: { tools: {} },
        serverInfo: { name: 'bukio-cli', version: '0.12.0' },
      }));
    case 'notifications/initialized':
    case 'initialized':
      return Promise.resolve(null); // no response
    case 'ping':
      return Promise.resolve(rpcResponse(id, {}));
    case 'tools/list':
      return Promise.resolve(rpcResponse(id, {
        tools: TOOLS.map((t) => ({
          name: t.name,
          description: t.description,
          inputSchema: {
            type: 'object',
            properties: t.inputSchema.properties,
            required: t.inputSchema.required ?? [],
          },
        })),
      }));
    case 'tools/call': {
      const t = TOOLS.find((x) => x.name === params.name);
      if (!t) return Promise.resolve(rpcError(id, -32602, `unknown tool '${params.name}'`));
      return Promise.resolve()
        .then(() => t.handler(db, params.arguments ?? {}, ctx))
        .then((result) => rpcResponse(id, { content: json(result), isError: false }))
        .catch((err) => rpcResponse(id, {
          content: json({ ok: false, error: { code: err.code ?? 'MCP_ERROR', message: err.message } }),
          isError: true,
        }));
    }
    default:
      return Promise.resolve(rpcError(id, -32601, `method not found: ${method}`));
  }
}

export function make(program) {
  program
    .command('mcp')
    .description('MCP server over stdio (JSON-RPC 2.0, newline-delimited). BUKIO_MCP_READONLY=1 -> plan-only mutations')
    .action(async (opts, command) => {
      const ctx = {
        dbPath: command.optsWithGlobals().db ?? process.env.BUKIO_DB,
        actor: command.optsWithGlobals().actor ?? 'agent:mcp',
        readonly: process.env.BUKIO_MCP_READONLY === '1',
      };
      // same guard as the CLI's ensureDb: a missing path must NOT be
      // auto-created — openDb would silently build an empty 24-table company
      // and an agent with a typo'd --db would book into it (balanced:true!)
      if (!existsSync(ctx.dbPath)) {
        throw dbError('NO_DATABASE', `no database at ${ctx.dbPath} — run 'bukio init' first`);
      }
      const db = openDb(ctx.dbPath);
      const rl = createInterface({ input: process.stdin, crlfDelay: Infinity });
      const send = (line) => {
        if (line) process.stdout.write(`${line}\n`);
      };
      for await (const line of rl) {
        if (!line.trim()) continue;
        let msg;
        try {
          msg = JSON.parse(line);
        } catch {
          send(rpcError(null, -32700, 'parse error'));
          continue;
        }
        try {
          send(await dispatch(db, ctx, msg));
        } catch (err) {
          send(rpcError(msg.id ?? null, -32603, `internal error: ${err.message}`));
        }
      }
      db.close();
    });
}
