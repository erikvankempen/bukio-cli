/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// MCP server (Phase 5) — Model Context Protocol over stdio (JSON-RPC 2.0,
// newline-delimited). Lets agents (Hermes, Claude, ...) drive bukio natively:
// read-only introspection tools plus mutation tools that default to dry-run
// plans — an agent executes only after a human approves the plan.
// Env: BUKIO_MCP_READONLY=1 turns every mutation into a plan-only call.
import { createInterface } from 'node:readline';
import { existsSync, readFileSync } from 'node:fs';
import { trialBalance } from '../report/trial-balance.js';
import { balans } from '../report/balans.js';
import { pnl } from '../report/pnl.js';
import { journal } from '../report/journal.js';
import { listAccounts } from '../core/accounts.js';
import { dbError, signPayload } from './util.js';
import { setPendingSignature } from '../audit/index.js';
import {
  createEntry, postEntry, reverseEntry, getEntry, resolvePostings,
  parsePostingSpecs, validateDate,
} from '../core/entries.js';
import { bookVatEntry, expandVatPostings, obReadout, parseVatPostingSpecs } from '../vat/index.js';
import {
  createContact, createInvoice, finalizeInvoice, creditInvoice, markPaid, getInvoice, listInvoices,
} from '../invoice/index.js';
import { runDue, previewDue } from '../recurring/index.js';
import { register, addAsset, runDue as assetsRunDue, disposeAsset } from '../assets/index.js';
import { yearEndClose, yearEndStatus, fiscalYearWindow } from '../year-end/index.js';
import { setFxRate, parseRate, toEurPostings, resolveRate } from '../fx/index.js';
import { icpReadout } from '../icp/index.js';
import { list as auditList } from '../audit/index.js';
import { complianceStatus } from '../compliance/index.js';
import { parseImportAmount } from '../import/index.js';
import { openDb } from '../core/db.js';
import { isValidActor } from '../core/actor.js';
import { parseAmount } from '../core/money.js';
import { createItem, listItems, updateItem } from '../items/index.js';
import { addAttachment, listAttachments, removeAttachment } from '../core/attachments.js';
import { aging } from '../report/aging.js';
import { sales } from '../report/sales.js';
import { importUblInvoice } from '../import/ubl-invoice.js';
import { emailInvoice } from '../invoice/email.js';
import { addMandate, listMandates, createPaymentBatch, exportPaymentBatch } from '../payments/index.js';

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

// --- sign gate for tool calls (Tier 0, Task 8) ----------------------------
//
// Every MUTATING tool call is signed by its actor before the handler runs —
// exactly like the CLI's preAction gate, sharing signPayload so the digest
// scheme, nonce cache and ±5 min window are identical. Enforcement state of
// the company DB applies equally: under `actor enforce on` an unsigned or
// unverifiable call is refused with the same error codes as the CLI, before
// any mutation or plan is produced. Record mode (default) still signs when
// key material exists — the audit rows the handler records then carry
// sig_status=verified. The signed payload is the tool call itself:
// cmd = `mcp:<tool_name>`, args = the tool arguments minus the identity
// flag `actor` (the same exclusion as the CLI's --actor). Read-only tools
// are not gated (they record nothing).

/** Tool args minus the identity flag — the exact signed payload. */
function signedToolArgs(args) {
  const out = {};
  for (const [k, v] of Object.entries(args ?? {})) {
    if (k === 'actor') continue;
    out[k] = v;
  }
  return out;
}

/** Sign a mutating tool call; throws a sign-gate error on enforcement refusal. */
function gateToolCall(db, ctx, toolName, args) {
  const bundle = signPayload({ actor: args?.actor ?? ctx.actor }, {
    cmd: `mcp:${toolName}`,
    args: signedToolArgs(args),
    db,
  });
  setPendingSignature(bundle);
}

/**
 * Resolve posting specs to EUR when a currency is given. The rate lookup
 * must NOT store anything on a plan-only call — the dry-run flag derives
 * from the MODE (modeOf), never from `args.dryRun`, which is not declared
 * in any tool schema and can therefore never be set by a client (a dead
 * flag made every dry-run store the ECB-fetched rate + an audit row).
 */
export async function resolveMcpFx(db, specs, args, ctx) {
  if (!args.currency) return specs;
  const rateX10000 = await resolveRate(db, {
    currency: args.currency, rate: args.rate, date: args.date,
    actor: args.actor ?? ctx.actor, dryRun: modeOf(args) === 'dry-run',
  });
  return toEurPostings(specs, { currency: args.currency, rateX10000 });
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
  name: 'balance_sheet', description: 'balance sheet as of a date (balanced must be true)', schema: {
    type: 'object', properties: { as_of: { type: 'string', description: 'YYYY-MM-DD' } },
  },
  handler: (db, args) => balans(db, { asOf: args.as_of }),
});
tool({
  name: 'pnl', description: 'profit & loss for a year (excludes closing entries)', schema: {
    type: 'object', properties: { year: { type: 'string' } }, required: ['year'],
  },
  handler: (db, args) => {
    const year = String(args.year ?? '');
    if (!/^\d{4}$/.test(year)) throw new McpError('INVALID_YEAR', `year '${args.year}' must be YYYY`);
    const [from, to] = fiscalYearWindow(db, year);
    return pnl(db, { from, to });
  },
});
tool({
  name: 'journal', description: 'journal export for a year (complete unless a limit is given; truncated: true when rows were cut)', schema: {
    type: 'object', properties: { year: { type: 'string' }, limit: { type: 'number' } }, required: ['year'],
  },
  handler: (db, args) => {
    const year = String(args.year ?? '');
    if (!/^\d{4}$/.test(year)) throw new McpError('INVALID_YEAR', `year '${args.year}' must be YYYY`);
    const limit = args.limit ?? 500;
    if (!Number.isInteger(limit) || limit < 0) throw new McpError('INVALID_LIMIT', `limit must be a non-negative integer, got '${limit}'`);
    const [from, to] = fiscalYearWindow(db, year);
    const rows = journal(db, { from, to, limit: limit + 1 });
    const truncated = rows.length > limit;
    return { rows: truncated ? rows.slice(0, limit) : rows, truncated, limit };
  },
});
tool({
  name: 'accounts', description: 'chart of accounts', schema: { type: 'object', properties: {} },
  handler: (db) => ({ accounts: listAccounts(db) }),
});
tool({
  name: 'vat_readout', description: 'VAT return fields 1a-5d for manual filing (never auto-files)', schema: {
    type: 'object', properties: { period: { type: 'string', description: 'YYYY-Qn or YYYY-MM' } }, required: ['period'],
  },
  handler: (db, args) => obReadout(db, { period: args.period }),
});
tool({
  name: 'icp_readout', description: 'ICP listing: EU reverse-charge supplies per customer', schema: {
    type: 'object', properties: { period: { type: 'string', description: 'YYYY-Qn' } }, required: ['period'],
  },
  handler: (db, args) => icpReadout(db, { period: args.period }),
});
tool({
  name: 'audit', description: 'append-only audit log', schema: {
    type: 'object', properties: { by: { type: 'string' }, since: { type: 'string' }, limit: { type: 'number' } },
  },
  handler: (db, args) => {
    const limit = args.limit ?? 50;
    if (!Number.isInteger(limit) || limit < 0) throw new McpError('INVALID_LIMIT', `limit must be a non-negative integer, got '${limit}'`);
    return { entries: auditList(db, { since: args.since ?? null, actor: args.by ?? null, limit }) };
  },
});
tool({
  name: 'compliance', description: 'compliance calendar for a year (OB/ICP deadlines, financial statements deposit)', schema: {
    type: 'object', properties: { year: { type: 'string' } }, required: ['year'],
  },
  handler: (db, args) => {
    const year = String(args.year ?? '');
    if (!/^\d{4}$/.test(year)) throw new McpError('INVALID_YEAR', `year '${args.year}' must be YYYY`);
    return complianceStatus(db, { year });
  },
});
tool({
  name: 'invoices',
  description: 'list invoices — optional status filter (draft|sent|paid|overdue|void) and type filter (sales|credit); returns newest first',
  schema: {
    type: 'object', properties: {
      limit: { type: 'number' }, status: { type: 'string' }, type: { type: 'string' },
    },
  },
  handler: (db, args) => {
    const limit = args.limit ?? 50;
    if (!Number.isInteger(limit) || limit < 0) throw new McpError('INVALID_LIMIT', `limit must be a non-negative integer, got '${limit}'`);
    if (args.status && !['draft', 'sent', 'paid', 'overdue', 'void'].includes(args.status)) {
      throw new McpError('INVALID_STATUS', `status must be one of draft|sent|paid|overdue|void, got '${args.status}'`);
    }
    if (args.type && !['sales', 'credit'].includes(args.type)) {
      throw new McpError('INVALID_TYPE', `type must be 'sales' or 'credit', got '${args.type}'`);
    }
    // listInvoices, NOT a raw `status = ?` filter: 'overdue' is a DERIVED
    // status (sent + past due) that is never stored — a raw SQL filter
    // would silently return an empty list for the tool's own documented
    // status value (the CLI path already routes through listInvoices)
    const rows = listInvoices(db, { status: args.status ?? null, type: args.type ?? null });
    return { invoices: rows.slice(0, limit) };
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
    const converted = await resolveMcpFx(db, specs, args, ctx);
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
      action: 'entry.create', date: args.date, description: args.description,
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
      source_ref: { type: 'string', description: 'boekstuk reference (source_ref on the entry)' },
      post: { type: 'boolean' }, actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['date', 'description', 'postings'],
  },
  handler: async (db, args, ctx) => {
    guardExecute(ctx, args);
    const specs = parseVatPostingSpecs(args.postings);
    const converted = await resolveMcpFx(db, specs, args, ctx);
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
    // parity with entry_add: execute rejects an unbalanced entry — a green
    // dry-run must not precede a failing execute
    const sum = expanded.reduce((s, p) => s + p.amountCents, 0);
    if (sum !== 0) {
      throw new McpError('UNBALANCED', `postings do not sum to zero (sum = ${sum})`);
    }
    // parity with entry_add: createEntry also rejects zero-amount postings
    // (INVALID_AMOUNT_CENTS) — validate them in dry-run so the plan never
    // precedes a failing execute
    for (const p of expanded) {
      if (!Number.isInteger(p.amountCents) || p.amountCents === 0) {
        throw new McpError('INVALID_AMOUNT_CENTS', `posting ${p.code} must be a non-zero integer amount in cents`);
      }
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
  description: 'create a draft invoice: contact_id + line specs ("2x Dienst @ 150.00 @21", per-line discount "@-10%" or "@-25.00", fractional qty "1.5x") OR item specs ("1:2@140.00@21@-10%"); discount_pct/discount_amount_cents apply to the total before VAT; language nl|en',
  schema: {
    type: 'object', properties: {
      contact_id: { type: 'number' },
      lines: { type: 'array', items: { type: 'string' } },
      items: { type: 'array', items: { type: 'string' } },
      date: { type: 'string' }, due_days: { type: 'number' },
      discount_pct: { type: 'number' }, discount_amount_cents: { type: 'number' },
      language: { type: 'string' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['contact_id'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const discountType = args.discount_pct !== undefined ? 'pct'
      : args.discount_amount_cents !== undefined ? 'amount' : null;
    const discountValue = discountType === 'pct'
      ? Math.round(Number(args.discount_pct) * 100)
      : discountType === 'amount' ? args.discount_amount_cents : null;
    const base = {
      contactId: args.contact_id,
      lines: args.lines ?? null,
      items: args.items ?? null,
      date: args.date ?? new Date().toISOString().slice(0, 10),
      dueDays: args.due_days ?? 30,
      discountType, discountValue,
      language: args.language ?? 'nl',
      actor: args.actor ?? ctx.actor,
    };
    if (modeOf(args) === 'dry-run') {
      // validate like the real path (createInvoice dryRun) — the old plan
      // echoed ok for garbage dates and nonexistent contacts
      const plan = createInvoice(db, { ...base, dryRun: true });
      return { ...plan, mode: 'dry-run', note: 'plan only — re-run with mode=execute to book' };
    }
    const inv = createInvoice(db, base);
    return {
      action: 'invoice.create', contact_id: args.contact_id,
      lines: args.lines ?? null, items: args.items ?? null, date: inv.date,
      mode: 'execute', invoice_id: inv.id, invoice_number: null, status: 'draft',
      totals: { net: inv.net_cents, vat: inv.vat_cents, gross: inv.gross_cents, discount: inv.discount_cents },
    };
  },
});
tool({
  name: 'item_add', mutating: true,
  description: 'add an item to the catalog (name, quantity unit h|day|month|unit|session|km|kg|project, price, optional default VAT code + revenue account)',
  schema: {
    type: 'object', properties: {
      name: { type: 'string' }, description: { type: 'string' },
      unit: { type: 'string' }, unit_price: { type: 'string' },
      vat_code: { type: 'string' }, gl_account: { type: 'string' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['name', 'unit_price'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const base = {
      name: args.name, description: args.description ?? null,
      unit: args.unit ?? 'unit', unitPriceCents: parseAmount(args.unit_price),
      vatCode: args.vat_code ?? null, glAccount: args.gl_account ?? null,
      actor: args.actor ?? ctx.actor,
    };
    if (modeOf(args) === 'dry-run') {
      return { ...createItem(db, { ...base, dryRun: true }), mode: 'dry-run', note: 'plan only — re-run with mode=execute to write' };
    }
    const item = createItem(db, base);
    return {
      action: 'item.create', mode: 'execute', item_id: item.id, name: item.name,
      unit: item.unit, unit_price_cents: item.unit_price_cents,
      vat_code: item.vat_code, gl_account: item.gl_account,
    };
  },
});
tool({
  name: 'item_list',
  description: 'items catalog (active items; pass include_inactive:true for all)',
  schema: { type: 'object', properties: { include_inactive: { type: 'boolean' } } },
  handler: (db, args) => ({
    items: listItems(db, { activeOnly: !(args?.include_inactive === true) })
      .map((i) => ({ id: i.id, name: i.name, unit: i.unit, unit_price_cents: i.unit_price_cents, vat_code: i.vat_code, gl_account: i.gl_account, active: i.active === 1 })),
  }),
});
tool({
  name: 'item_update', mutating: true,
  description: 'update an item (price, unit, VAT, GL account) or deactivate it (deactivate:true blocks new invoices; existing keep snapshots)',
  schema: {
    type: 'object', properties: {
      id: { type: 'number' }, name: { type: 'string' }, description: { type: 'string' },
      unit: { type: 'string' }, unit_price: { type: 'string' },
      vat_code: { type: 'string' }, gl_account: { type: 'string' },
      deactivate: { type: 'boolean' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['id'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const base = {
      id: args.id,
      name: args.name ?? null,
      description: args.description !== undefined ? args.description : null,
      unit: args.unit ?? null,
      unitPriceCents: args.unit_price !== undefined ? parseAmount(args.unit_price) : null,
      vatCode: args.vat_code !== undefined ? args.vat_code : null,
      glAccount: args.gl_account !== undefined ? args.gl_account : null,
      deactivate: args.deactivate === true,
      actor: args.actor ?? ctx.actor,
    };
    if (modeOf(args) === 'dry-run') {
      return { ...updateItem(db, { ...base, dryRun: true }), mode: 'dry-run', note: 'plan only — re-run with mode=execute to write' };
    }
    const item = updateItem(db, base);
    return { action: 'item.update', mode: 'execute', item_id: item.id, active: item.active === 1 };
  },
});

// --- attachments ------------------------------------------------------------

tool({
  name: 'attachment_add', mutating: true,
  description: 'store a source document (pdf/jpg/png/xml/eml/...) against an invoice or entry; mode db (default) = BLOB in the database, file = path in <db>-attachments/',
  schema: {
    type: 'object', properties: {
      kind: { type: 'string' }, ref_id: { type: 'number' }, file_path: { type: 'string' },
      store: { type: 'string' }, note: { type: 'string' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['kind', 'ref_id', 'file_path'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const base = {
      kind: args.kind, refId: args.ref_id, filePath: args.file_path,
      note: args.note ?? null, store: args.store ?? 'db', actor: args.actor ?? ctx.actor,
    };
    if (modeOf(args) === 'dry-run') {
      // module dryRun validates like execute (kind, ref, file, size, duplicate)
      return { ...addAttachment(db, { ...base, dryRun: true }), mode: 'dry-run', note: 'plan only — re-run with mode=execute to write' };
    }
    const a = addAttachment(db, base);
    return { action: 'attachments.add', mode: 'execute', attachment_id: a.id, kind: a.kind, ref_id: a.ref_id, file_name: a.file_name, size: a.size, store: a.mode };
  },
});
tool({
  name: 'attachment_list',
  description: 'attachments for an invoice or entry (metadata only — never the file bytes)',
  schema: {
    type: 'object', properties: { kind: { type: 'string' }, ref_id: { type: 'number' } },
    required: ['kind', 'ref_id'],
  },
  handler: (db, args) => ({
    attachments: listAttachments(db, { kind: args.kind, refId: args.ref_id }),
  }),
});
tool({
  name: 'attachment_remove', mutating: true,
  description: 'remove an attachment (file-mode copies on disk are deleted too)',
  schema: { type: 'object', properties: { id: { type: 'number' }, actor: { type: 'string' }, mode: { type: 'string' } }, required: ['id'] },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    if (modeOf(args) === 'dry-run') {
      return { ...removeAttachment(db, { id: args.id, actor: args.actor ?? ctx.actor, dryRun: true }), mode: 'dry-run', note: 'plan only — re-run with mode=execute to write' };
    }
    const r = removeAttachment(db, { id: args.id, actor: args.actor ?? ctx.actor });
    return { action: 'attachment.remove', mode: 'execute', attachment_id: r.id, kind: r.kind, ref_id: r.ref_id, file_name: r.file_name };
  },
});
tool({
  name: 'report_aging',
  description: 'open items per contact bucketed by days past due (kind: debtors | creditors | both)',
  schema: {
    type: 'object', properties: { as_of: { type: 'string' }, kind: { type: 'string' } },
  },
  handler: (db, args) => aging(db, { asOf: args?.as_of ?? null, kind: args?.kind ?? 'both' }),
});
tool({
  name: 'report_sales',
  description: 'sales revenue for a year, by contact (net/vat/gross) or by item (net)',
  schema: {
    type: 'object', properties: { year: { type: 'string' }, by: { type: 'string' } },
    required: ['year'],
  },
  handler: (db, args) => sales(db, { year: args.year, by: args?.by ?? 'contact' }),
});

// --- inbound e-invoices ------------------------------------------------------

tool({
  name: 'invoice_import', mutating: true,
  description: 'import an inbound e-invoice (EN 16931 / Peppol BIS 3.0 UBL XML) into the payables register — no journal entry is created; book it via the normal workflow',
  schema: {
    type: 'object', properties: {
      file_path: { type: 'string' }, contact: { type: 'number' },
      create_missing: { type: 'boolean' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['file_path'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    if (!args.file_path || !existsSync(args.file_path)) {
      throw new McpError('FILE_NOT_FOUND', `file '${args.file_path}' does not exist`);
    }
    const base = {
      xmlText: readFileSync(args.file_path, 'utf8'),
      contact: args.contact != null ? Number(args.contact) : null,
      createMissing: args.create_missing === true,
      actor: args.actor ?? ctx.actor,
    };
    if (modeOf(args) === 'dry-run') {
      // module dryRun validates like execute (UBL structure, contact resolution)
      return { ...importUblInvoice(db, { ...base, dryRun: true }), mode: 'dry-run', note: 'plan only — re-run with mode=execute to write' };
    }
    const r = importUblInvoice(db, base);
    return { action: 'invoice.import', mode: 'execute', imported: r.imported, duplicates: r.duplicates, payable_ref: r.invoice_ref, supplier: r.supplier, amount_cents: r.amount_cents };
  },
});
tool({
  name: 'invoice_email', mutating: true,
  description: 'email a finalized invoice (PDF attached) to the contact — SMTP config via BUKIO_SMTP_* env; dry-run renders the PDF but sends nothing',
  schema: {
    type: 'object', properties: {
      id: { type: 'number' }, to: { type: 'string' }, subject: { type: 'string' },
      body: { type: 'string' }, attach_pdf: { type: 'boolean' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['id'],
  },
  handler: async (db, args, ctx) => {
    guardExecute(ctx, args);
    const base = {
      id: args.id, to: args.to ?? null, subject: args.subject ?? null,
      body: args.body ?? null, attachPdf: args.attach_pdf !== false,
      actor: args.actor ?? ctx.actor,
    };
    if (modeOf(args) === 'dry-run') {
      // module dryRun validates like execute (invoice exists/finalized,
      // recipient, SMTP config) and renders the PDF — no network call
      const plan = await emailInvoice(db, { ...base, dryRun: true });
      return { ...plan, mode: 'dry-run', note: 'plan only — re-run with mode=execute to send' };
    }
    const r = await emailInvoice(db, base);
    return { action: 'invoice.email', mode: 'execute', invoice_id: r.id, invoice_number: r.invoice_number, to: r.to, delivered: r.delivered };
  },
});
tool({
  name: 'payments_mandate_add', mutating: true,
  description: 'register a signed SEPA direct-debit mandate for a contact (core = 8-week refund right, b2b = none)',
  schema: {
    type: 'object', properties: {
      contact_id: { type: 'number' }, mandate_ref: { type: 'string' },
      mandate_date: { type: 'string' }, scheme: { type: 'string' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['contact_id', 'mandate_ref'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const base = {
      contactId: args.contact_id, mandateRef: args.mandate_ref, mandateDate: args.mandate_date ?? null,
      scheme: args.scheme ?? 'core', actor: args.actor ?? ctx.actor,
    };
    if (modeOf(args) === 'dry-run') {
      const plan = addMandate(db, { ...base, dryRun: true });
      return { ...plan, mode: 'dry-run', note: 'plan only — re-run with mode=execute to register' };
    }
    const r = addMandate(db, base);
    return { action: 'payments.mandate.add', mode: 'execute', mandate_id: r.id, contact_id: r.contact_id, mandate_ref: r.mandate_ref, scheme: r.scheme };
  },
});
tool({
  name: 'payments_mandate_list',
  description: 'list SEPA direct-debit mandates (optionally per contact)',
  schema: { type: 'object', properties: { contact_id: { type: 'number' } } },
  handler: (db, args) => ({ mandates: listMandates(db, { contactId: args?.contact_id ?? null }) }),
});
tool({
  name: 'payments_batch_create', mutating: true,
  description: 'create a SEPA batch: type transfer (pain.001) from transfer payables, or direct_debit (pain.008, incasso) from direct-debit payables (each needs a contact mandate)',
  schema: {
    type: 'object', properties: {
      payable_ids: { type: 'array', items: { type: 'number' } },
      batch_date: { type: 'string' }, type: { type: 'string' },
      actor: { type: 'string' }, mode: { type: 'string' },
    },
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const base = {
      date: args.batch_date ?? null, debitIban: null, lines: [],
      payableIds: (args.payable_ids ?? []).map(Number),
      kind: args.type === 'direct_debit' ? 'direct_debit' : 'transfer',
      actor: args.actor ?? ctx.actor,
    };
    if (modeOf(args) === 'dry-run') {
      const plan = createPaymentBatch(db, { ...base, dryRun: true });
      return { ...plan, mode: 'dry-run', note: 'plan only — re-run with mode=execute to create' };
    }
    const r = createPaymentBatch(db, base);
    return { action: 'payments.batch.create', mode: 'execute', batch_id: r.id, batch_kind: r.batch_kind, total_cents: r.total_cents, lines: r.lines.length, status: r.status };
  },
});
tool({
  name: 'payments_batch_export', mutating: true,
  description: 'export a draft batch as SEPA XML (pain.001 for transfer, pain.008.001.02 for direct-debit) — one export per batch, marks it exported',
  schema: { type: 'object', properties: { batch_id: { type: 'number' }, actor: { type: 'string' }, mode: { type: 'string' } }, required: ['batch_id'] },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const base = { id: args.batch_id, schema: null, actor: args.actor ?? ctx.actor };
    if (modeOf(args) === 'dry-run') {
      const plan = exportPaymentBatch(db, { ...base, dryRun: true });
      return { ...plan, mode: 'dry-run', note: 'plan only — re-run with mode=execute to export' };
    }
    const r = exportPaymentBatch(db, base);
    return { action: 'payments.batch.export', mode: 'execute', batch_id: r.batch_id, schema: r.schema, msg_id: r.msg_id, status: r.status, xml: r.xml };
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
    const inv = markPaid(db, { id: args.id, date: args.date, amountCents, method: args.method ?? 'bank', actor: args.actor ?? ctx.actor });
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
  description: 'register an invoice counterparty (EU customers need a VAT id for reverse charge/ICP)',
  schema: {
    type: 'object', properties: {
      name: { type: 'string' }, address: { type: 'string' }, postal_code: { type: 'string' },
      city: { type: 'string' }, country: { type: 'string' }, email: { type: 'string' },
      vat_id: { type: 'string' }, kvk: { type: 'string' }, iban: { type: 'string' },
      actor: { type: 'string' }, mode: { type: 'string' },
    }, required: ['name'],
  },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    // same validation in both paths — createContact rejects a missing name
    // AND an invalid IBAN (dry-run included), so a dry-run plan can't lie
    const c = createContact(db, {
      name: args.name, address: args.address, postalCode: args.postal_code, city: args.city,
      country: args.country ?? 'NL', email: args.email, vatId: args.vat_id, kvk: args.kvk,
      iban: args.iban, actor: args.actor ?? ctx.actor,
      dryRun: modeOf(args) === 'dry-run',
    });
    return { action: 'contact.create', id: c.id ?? null, name: c.name, mode: modeOf(args) === 'dry-run' ? 'dry-run' : 'execute' };
  },
});

// --- fixed assets ------------------------------------------------------------

tool({
  name: 'assets_register', description: 'activastaat: cost, cumulative depreciation, book value per asset',
  schema: { type: 'object', properties: { as_of: { type: 'string' } } },
  handler: (db, args) => register(db, { asOf: args.as_of ?? null }),
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
      return { action: 'assets.add', mode: 'dry-run', asset: r.asset };
    }
    const r = addAsset(db, { ...common, actor: args.actor ?? ctx.actor, dryRun: false });
    return { action: 'assets.add', id: r.asset.id, name: r.asset.name, warnings: r.warnings, mode: 'execute' };
  },
});

tool({
  name: 'assets_run', mutating: true,
  description: 'book the depreciation runs that are due (idempotent per asset-month)',
  schema: { type: 'object', properties: { period: { type: 'string' }, as_of: { type: 'string' }, actor: { type: 'string' }, mode: { type: 'string' } } },
  handler: (db, args, ctx) => {
    guardExecute(ctx, args);
    const actor = args.actor ?? ctx.actor;
    // assetsRunDue — the bare `runDue` name collides with the RECURRING
    // module's runDue (same import name); calling that here would book due
    // recurring entries instead of depreciation runs
    if (modeOf(args) === 'dry-run') {
      const r = assetsRunDue(db, { period: args.period, asOf: args.as_of, actor, dryRun: true });
      return { action: 'assets.run', mode: 'dry-run', plan: r.plan };
    }
    const r = assetsRunDue(db, { period: args.period, asOf: args.as_of, actor, dryRun: false });
    return { action: 'assets.run', booked: r.booked, mode: 'execute' };
  },
});

tool({
  name: 'asset_dispose', mutating: true,
  description: 'dispose of an asset (sale or scrap): books the full entry, status -> disposed',
  schema: {
    type: 'object', properties: {
      id: { type: 'number' }, date: { type: 'string' }, proceeds: { type: 'string' },
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
  // `params: null` (some JSON-RPC clients send it for no-argument calls)
  // must not fall through to `params.name` — the destructure default only
  // applies to `undefined`, so null would throw a TypeError here and surface
  // as a -32603 internal error instead of a clean response
  const { id, method } = msg;
  const params = msg.params ?? {};
  switch (method) {
    case 'initialize':
      return Promise.resolve(rpcResponse(id, {
        protocolVersion: PROTOCOL_VERSION,
        capabilities: { tools: {} },
        serverInfo: { name: 'bukio-cli', version: '0.15.1' },
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
        .then(() => {
          // Tier 0: sign mutating calls before the handler runs (enforce
          // refusals throw here, before any mutation or plan)
          if (t.mutating) gateToolCall(db, ctx, t.name, params.arguments ?? {});
        })
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
        // same resolution as the CLI's makeCtx: --actor flag, then
        // BUKIO_ACTOR env, then the generic fallback — without the env
        // honouring, an MCP session under BUKIO_ACTOR signed every tool
        // call as 'agent:mcp' (wrong key → SIGNATURE_REQUIRED under
        // enforce, and wrong audit attribution in record mode)
        actor: command.optsWithGlobals().actor ?? process.env.BUKIO_ACTOR ?? 'agent:mcp',
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
        // JSON-RPC requests are objects — `null`, `42` or an array would
        // throw in the dispatch destructure AND again in `msg.id` below,
        // crashing the whole server instead of answering Invalid Request
        if (typeof msg !== 'object' || msg === null || Array.isArray(msg)) {
          send(rpcError(null, -32600, 'invalid request'));
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
