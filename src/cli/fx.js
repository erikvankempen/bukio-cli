/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio fx — foreign currency rates (Phase 5, FR5.X)
import { setFxRate, listFxRates } from '../fx/index.js';
import { fetchEcbRate } from '../fx/ecb.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

export function make(program) {
  const fx = program.command('fx').description('foreign exchange rates');
  fx
    .command('fetch')
    .description('fetch the ECB reference rate (1 EUR = N) for a currency on/before a date and store it')
    .requiredOption('--currency <ISO>', 'ISO 4217 currency (USD, GBP, ...)')
    .option('--date <yyyy-mm-dd>', 'rate date (default: today; weekends/holidays fall back to the last business day)')
    .option('--dry-run', 'fetch and show the rate without storing it')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const date = opts.date ?? new Date().toISOString().slice(0, 10);
          const r = await fetchEcbRate({ currency: opts.currency, date });
          if (!r) {
            throw Object.assign(new Error(`no ECB reference rate for ${opts.currency} on/before ${date} (not in the ECB set, or before 1999)`), { code: 'ECB_RATE_NOT_AVAILABLE' });
          }
          if (ctx.dryRun) {
            output(ctx, { rate: { currency: opts.currency, date: r.date, rate: (r.rateX10000 / 10000).toFixed(4), source: 'ECB' }, fetched_for: date, dryRun: true }, (d) => {
              console.log(`plan: store 1 EUR = ${d.rate.rate} ${d.rate.currency} on ${d.rate.date} (ECB, fetched for ${d.fetched_for})`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          const stored = setFxRate(db, { currency: opts.currency, date: r.date, rate: r.rateX10000, source: 'ECB', actor: ctx.actor });
          output(ctx, { rate: stored, fetched_for: date }, (d) =>
            console.log(`1 EUR = ${d.rate.rate} ${d.rate.currency} on ${d.rate.date} (ECB, fetched for ${d.fetched_for}) — stored`));
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
  fx
    .command('set')
    .description('store/update a rate: 1 EUR = N units of currency on a date')
    .requiredOption('--currency <ISO>', 'ISO 4217 currency (USD, GBP, ...)')
    .requiredOption('--date <YYYY-MM-DD>', 'rate as of this date')
    .requiredOption('--rate <n>', 'units of foreign currency per EUR (e.g. 1.0875)')
    .option('--source <text>', 'rate source (default: manual)')
    .option('--dry-run', 'validate the rate without storing it')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const r = setFxRate(db, { currency: opts.currency, date: opts.date, rate: opts.rate, source: opts.source, actor: ctx.actor, dryRun: ctx.dryRun });
          output(ctx, { rate: r }, (d) => {
            if (d.rate.dryRun) {
              console.log(`plan: store 1 EUR = ${d.rate.rate} ${d.rate.currency} on ${d.rate.date}`);
              console.log('(dry run — nothing written)');
              return;
            }
            console.log(`1 EUR = ${d.rate.rate} ${d.rate.currency} on ${d.rate.date} (stored)`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
  fx
    .command('show')
    .description('rate history for one currency')
    .requiredOption('--currency <ISO>', 'ISO 4217 currency')
    .option('--limit <n>', 'max rows', '50')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const rates = listFxRates(db, { currency: opts.currency, limit: Number(opts.limit) });
          output(ctx, { currency: opts.currency, rates }, (d) => {
            if (!d.rates.length) { console.log(`no rates stored for ${d.currency}`); return; }
            table(d.rates, [
              { key: 'date', label: 'date' },
              { key: 'rate', label: '1 EUR =' },
              { key: 'source', label: 'source' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
  fx
    .command('list')
    .description('all stored rates, newest first')
    .option('--limit <n>', 'max rows', '50')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const rates = listFxRates(db, { limit: Number(opts.limit) });
          output(ctx, { rates }, (d) => {
            if (!d.rates.length) { console.log('no rates stored'); return; }
            table(d.rates, [
              { key: 'currency', label: 'currency' },
              { key: 'date', label: 'date' },
              { key: 'rate', label: '1 EUR =' },
              { key: 'source', label: 'source' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
