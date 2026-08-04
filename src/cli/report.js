// bukio report — reporting commands (Phase 0: trial balance).
import { formatAmount } from '../core/money.js';
import { trialBalance } from '../report/trial-balance.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

export function make(program) {
  const report = program.command('report').description('reports');

  report
    .command('trial-balance')
    .description('per-account debit/credit/net from posted entries')
    .option('--year <yyyy>', 'filter by year')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const tb = trialBalance(db, { year: opts.year || null });
          const data = {
            year: opts.year || null,
            accounts: tb.accounts.map((a) => ({
              code: a.code, name: a.name, type: a.type,
              debit_cents: a.debit_cents, credit_cents: a.credit_cents, net_cents: a.net_cents,
              debit: formatAmount(a.debit_cents), credit: formatAmount(a.credit_cents), net: formatAmount(a.net_cents),
            })),
            total_debit_cents: tb.total_debit_cents,
            total_credit_cents: tb.total_credit_cents,
            total_debit: formatAmount(tb.total_debit_cents),
            total_credit: formatAmount(tb.total_credit_cents),
            balanced: tb.balanced,
          };
          output(ctx, data, (d) => {
            table(d.accounts, [
              { key: 'code', label: 'code' },
              { key: 'name', label: 'account' },
              { key: 'type', label: 'type' },
              { key: 'debit', label: 'debit' },
              { key: 'credit', label: 'credit' },
              { key: 'net', label: 'net' },
            ]);
            console.log(`totals:  debit ${d.total_debit}  credit ${d.total_credit}  -> ${d.balanced ? 'BALANCED' : 'UNBALANCED!'}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
