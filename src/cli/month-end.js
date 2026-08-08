/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio month-end — month-end close check (read-only, wraps the module).
import { formatAmount } from '../core/money.js';
import { monthEnd } from '../month-end/index.js';
import { ensureDb, makeCtx, output, fail } from './util.js';

export function make(program) {
  program
    .command('month-end')
    .description('month-end close check (read-only): drafts, bank, VAT, invoices, recurring, totals')
    .requiredOption('--period <yyyy-mm>', 'period to check, e.g. 2026-08')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const data = monthEnd(db, { period: opts.period });
          output(ctx, data, (d) => {
            console.log(`MONTH-END ${d.period} (${d.from} .. ${d.to})`);
            console.log(`  entries:  ${d.entries.draft} draft`);
            console.log(`  bank:     ${d.bank.unmatched} unmatched`);
            if (d.vat) console.log(`  vat:      ${d.vat.quarter} → te betalen/ontvangen ${d.vat.to_pay}`);
            console.log(`  invoices: ${d.invoices.draft} draft, ${d.invoices.overdue} overdue (${formatAmount(d.invoices.overdue_total_cents)})`);
            console.log(`  recurring:${d.recurring.due} due by ${d.to}`);
            console.log(`  totals:   debet ${formatAmount(d.totals.debit_cents)} / credit ${formatAmount(d.totals.credit_cents)} ${d.totals.balanced ? 'BALANCED' : 'UNBALANCED!'}`);
            console.log(`  result:   ${d.totals.profit}`);
            for (const w of d.warnings) console.log(`  ! ${w}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
