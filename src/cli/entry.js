/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio entry — journal entries: add / post / reverse / list / show.
import { existsSync } from 'node:fs';
import { openDb } from '../core/db.js';
import { formatAmount } from '../core/money.js';
import {
  createEntry, getEntry, listEntries, parsePostingSpecs, postEntry,
  resolvePostings, reverseEntry, validateDate,
} from '../core/entries.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';
import { resolveRate, toEurPostings } from '../fx/index.js';

/** Convert posting specs to EUR when --currency given; auto rate lookup + ECB fallback. */
async function applyFx(db, postings, { currency, rate, date, actor }) {
  if (!currency) return postings;
  if (!db && rate == null) {
    throw Object.assign(new Error(`no database yet — pass --rate or create the company database first`), { code: 'FX_RATE_NOT_FOUND' });
  }
  const rateX10000 = await resolveRate(db, { currency, rate, date, actor });
  return toEurPostings(postings, { currency, rateX10000 });
}

function todayIso() {
  return new Date().toISOString().slice(0, 10);
}

function serializeEntry(entry) {
  return {
    ...entry,
    postings: entry.postings.map((p) => ({
      id: p.id,
      account_code: p.account_code,
      account_name: p.account_name,
      account_type: p.account_type,
      amount_cents: p.amount_cents,
      amount: formatAmount(p.amount_cents),
    })),
  };
}

function renderEntry(e) {
  console.log(`entry #${e.id}  [${e.state}]  ${e.date}  ${e.description}`);
  for (const p of e.postings) {
    console.log(`  ${p.account_code}  ${p.account_name.padEnd(28)} ${p.amount}`);
  }
  console.log(`  ${''.padEnd(31)} --------`);
  const total = e.postings.reduce((s, p) => s + p.amount_cents, 0);
  console.log(`  ${''.padEnd(31)} ${formatAmount(total)}  (sum)`);
}

export function make(program) {
  const entry = program.command('entry').description('journal entries');

  entry
    .command('add')
    .description('create a journal entry (draft; --post to post immediately)')
    .option('--date <yyyy-mm-dd>', 'entry date', todayIso())
    .requiredOption('--desc <description>', 'description')
    .option('--postings <CODE:AMOUNT>', 'posting spec, repeatable or comma-separated (positive=debit, negative=credit)', collect, [])
    .option('--source <source>', 'manual|bank|invoice|agent', 'manual')
    .option('--source-ref <ref>', 'source reference')
    .option('--currency <ISO>', 'postings are in this foreign currency; converted to EUR (needs a rate)')
    .option('--rate <n>', 'FX rate (1 EUR = n units); auto-looked-up on/before the date when omitted')
    .option('--post', 'post the entry immediately (draft -> posted)')
    .option('--dry-run', 'show the plan without writing anything')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        await addAction(ctx, opts);
      } catch (err) {
        fail(ctx, err);
      }
    });

  entry
    .command('post')
    .description('post a draft entry')
    .requiredOption('--id <id>', 'entry id')
    .option('--dry-run', 'show the plan without writing anything')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        postAction(ctx, opts);
      } catch (err) {
        fail(ctx, err);
      }
    });

  entry
    .command('reverse')
    .description('reverse a posted entry (creates a linked contra-entry)')
    .requiredOption('--id <id>', 'entry id')
    .option('--reason <text>', 'reason for the reversal')
    .option('--dry-run', 'show the plan without writing anything')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        reverseAction(ctx, opts);
      } catch (err) {
        fail(ctx, err);
      }
    });

  entry
    .command('list')
    .description('list journal entries')
    .option('--state <state>', 'draft|posted|reversed')
    .option('--date-from <yyyy-mm-dd>', 'earliest date (inclusive)')
    .option('--date-to <yyyy-mm-dd>', 'latest date (inclusive)')
    .option('--limit <n>', 'max rows', '100')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        listAction(ctx, opts);
      } catch (err) {
        fail(ctx, err);
      }
    });

  entry
    .command('show')
    .description('show one entry with its postings')
    .requiredOption('--id <id>', 'entry id')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        showAction(ctx, opts);
      } catch (err) {
        fail(ctx, err);
      }
    });
}

function collect(value, previous) {
  return previous.concat(value);
}

async function addAction(ctx, opts) {
  const postings = parsePostingSpecs(opts.postings);

  if (ctx.dryRun) {
    // same validation as the real run where it needs no DB (createEntry does
    // the full check incl. account resolution when a DB exists) — the old
    // branch echoed garbage dates/unbalanced postings as ok:true
    validateDate(opts.date);
    if (!opts.desc || !String(opts.desc).trim()) {
      throw Object.assign(new Error('description is required'), { code: 'INVALID_DESCRIPTION' });
    }
    if (postings.length < 2) {
      throw Object.assign(new Error('an entry needs at least 2 postings'), { code: 'TOO_FEW_POSTINGS' });
    }
    const db = existsSync(ctx.dbPath) ? openDb(ctx.dbPath) : null;
    let converted = postings;
    let resolved = null;
    try {
      converted = await applyFx(db, postings, { ...opts, actor: ctx.actor });
      if (db) {
        resolved = resolvePostings(db, converted).map((p) => ({
          code: p.code, amount_cents: p.amountCents, amount: formatAmount(p.amountCents),
          fx_currency: p.fxCurrency, fx_amount_cents: p.fxAmountCents,
        }));
      } else {
        resolved = null;
      }
    } finally {
      if (db) db.close();
    }
    // the displayed sum must match the displayed (EUR) postings — for FX
    // entries the raw specs sum in the foreign currency
    const sum = converted.reduce((s, p) => s + p.amountCents, 0);
    if (sum !== 0) {
      throw Object.assign(new Error(`postings do not sum to zero (sum = ${sum} cents)`), { code: 'UNBALANCED' });
    }
    output(ctx, {
      action: 'create journal entry',
      date: opts.date,
      description: opts.desc,
      currency: opts.currency ?? null,
      postings: resolved ?? postings.map((p) => ({ code: p.code, amount_cents: p.amountCents, amount: formatAmount(p.amountCents) })),
      sum_cents: sum,
      sum: formatAmount(sum),
      state: 'draft',
      post: Boolean(opts.post),
      account_validation: resolved ? 'ok' : 'skipped (no database yet)',
      dryRun: true,
    }, (d) => {
      console.log(`plan: create entry ${d.date} "${d.description}"${d.currency ? ` (${d.currency} -> EUR)` : ''}`);
      for (const p of d.postings) {
        const fx = p.fx_currency ? `  [${p.fx_currency} ${p.fx_amount_cents != null ? (p.fx_amount_cents / 100).toFixed(2) : ''}]` : '';
        console.log(`  ${p.code}  ${p.amount}${fx}`);
      }
      console.log(`  sum: ${d.sum}${d.post ? '  -> will post' : ''}`);
      console.log(d.account_validation.startsWith('ok') ? '(accounts validated)' : `(note: ${d.account_validation})`);
      console.log('(dry run — nothing written)');
    });
    return;
  }

  const db = ensureDb(ctx);
  try {
    const converted = await applyFx(db, postings, { ...opts, actor: ctx.actor });
    let entry = createEntry(db, {
      date: opts.date,
      description: opts.desc,
      postings: converted.map((p) => ({ code: p.code, amountCents: p.amountCents, fxCurrency: p.fxCurrency, fxAmountCents: p.fxAmountCents })),
      source: opts.source,
      sourceRef: opts.sourceRef ?? null,
      actor: ctx.actor,
    });
    if (opts.post) {
      entry = postEntry(db, { id: entry.id, actor: ctx.actor });
    }
    output(ctx, serializeEntry(entry), renderEntry);
  } finally {
    db.close();
  }
}

function postAction(ctx, opts) {
  const db = ensureDb(ctx);
  try {
    const entry = getEntry(db, opts.id);
    if (!entry) throw Object.assign(new Error(`entry ${opts.id} does not exist`), { code: 'NOT_FOUND' });
    if (ctx.dryRun) {
      output(ctx, {
        action: 'post entry',
        id: Number(opts.id),
        current_state: entry.state,
        target_state: entry.state === 'draft' ? 'posted' : '(no change)',
        dryRun: true,
      }, (d) => {
        console.log(`plan: post entry #${d.id} (${d.current_state} -> ${d.target_state})`);
        console.log('(dry run — nothing written)');
      });
      return;
    }
    const posted = postEntry(db, { id: opts.id, actor: ctx.actor });
    output(ctx, serializeEntry(posted), renderEntry);
  } finally {
    db.close();
  }
}

function reverseAction(ctx, opts) {
  const db = ensureDb(ctx);
  try {
    const entry = getEntry(db, opts.id);
    if (!entry) throw Object.assign(new Error(`entry ${opts.id} does not exist`), { code: 'NOT_FOUND' });
    if (ctx.dryRun) {
      output(ctx, {
        action: 'reverse entry (create linked contra-entry)',
        id: Number(opts.id),
        current_state: entry.state,
        reversed_postings: entry.postings.map((p) => ({
          account_code: p.account_code,
          amount_cents: -p.amount_cents,
          amount: formatAmount(-p.amount_cents),
        })),
        dryRun: true,
      }, (d) => {
        console.log(`plan: reverse entry #${d.id} (state: ${d.current_state})`);
        for (const p of d.reversed_postings) console.log(`  ${p.account_code}  ${p.amount}`);
        console.log('(dry run — nothing written)');
      });
      return;
    }
    const reversed = reverseEntry(db, { id: opts.id, actor: ctx.actor, reason: opts.reason ?? null });
    output(ctx, serializeEntry(reversed), renderEntry);
  } finally {
    db.close();
  }
}

function listAction(ctx, opts) {
  const db = ensureDb(ctx);
  try {
    const entries = listEntries(db, {
      state: opts.state || null,
      dateFrom: opts.dateFrom || null,
      dateTo: opts.dateTo || null,
      limit: Number(opts.limit),
    });
    const data = entries.map((e) => ({
      id: e.id, date: e.date, description: e.description, state: e.state,
      source: e.source, created_by: e.created_by,
    }));
    output(ctx, { entries: data }, (d) => {
      table(d.entries, [
        { key: 'id', label: '#' },
        { key: 'date', label: 'date' },
        { key: 'state', label: 'state' },
        { key: 'source', label: 'source' },
        { key: 'created_by', label: 'by' },
        { key: 'description', label: 'description' },
      ]);
    });
  } finally {
    db.close();
  }
}

function showAction(ctx, opts) {
  const db = ensureDb(ctx);
  try {
    const entry = getEntry(db, opts.id);
    if (!entry) throw Object.assign(new Error(`entry ${opts.id} does not exist`), { code: 'NOT_FOUND' });
    output(ctx, serializeEntry(entry), renderEntry);
  } finally {
    db.close();
  }
}
