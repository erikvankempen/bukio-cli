// bukio fx — foreign currency rates (Phase 5, FR5.X)
import { setFxRate, listFxRates } from '../fx/index.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

export function make(program) {
  const fx = program.command('fx').description('foreign exchange rates (vreemde valuta)');
  fx
    .command('set')
    .description('store/update a rate: 1 EUR = N units of currency on a date')
    .requiredOption('--currency <ISO>', 'ISO 4217 currency (USD, GBP, ...)')
    .requiredOption('--date <YYYY-MM-DD>', 'rate as of this date')
    .requiredOption('--rate <n>', 'units of foreign currency per EUR (e.g. 1.0875)')
    .option('--source <text>', 'rate source (default: manual)')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const r = setFxRate(db, { currency: opts.currency, date: opts.date, rate: opts.rate, source: opts.source, actor: ctx.actor });
          output(ctx, { rate: r }, (d) => console.log(`1 EUR = ${d.rate.rate} ${d.rate.currency} on ${d.rate.date} (stored)`));
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
              { key: 'date', label: 'datum' },
              { key: 'rate', label: '1 EUR =' },
              { key: 'source', label: 'bron' },
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
              { key: 'currency', label: 'valuta' },
              { key: 'date', label: 'datum' },
              { key: 'rate', label: '1 EUR =' },
              { key: 'source', label: 'bron' },
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
