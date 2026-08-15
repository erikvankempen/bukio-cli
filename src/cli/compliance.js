/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio compliance — filing deadlines calendar (Phase 5)
import { complianceStatus, markFiled } from '../compliance/index.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

export function make(program) {
  const comp = program.command('compliance').description('filing deadlines (OB/ICP/jaarrekening) — never auto-files');
  comp
    .command('status')
    .description('calendar of obligations for a year')
    .requiredOption('--year <yyyy>', 'calendar year')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const r = complianceStatus(db, { year: opts.year });
          output(ctx, { compliance: r }, (d) => {
            const c = d.compliance;
            console.log(`COMPLIANCE ${c.year} — ${c.company} (as of ${c.as_of})`);
            // table() takes (rows, cols) — the annotation must be applied to
            // the rows BEFORE rendering (a third callback argument was
            // silently ignored and the note never showed)
            const obligations = c.obligations.map((row) => {
              const extra = row.books_closed === false && row.type === 'JAARREKENING' ? ' (books not closed)' : '';
              return extra ? { ...row, status: `${row.status}${extra}` } : row;
            });
            table(obligations, [
              { key: 'type', label: 'type' },
              { key: 'period', label: 'periode' },
              { key: 'deadline', label: 'deadline' },
              { key: 'status', label: 'status' },
            ]);
            console.log(`summary: ${c.summary.filed} filed, ${c.summary.open} open, ${c.summary.overdue} overdue`);
            console.log(c.note);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
  comp
    .command('mark')
    .description('record a manual filing (OB uses "vat readout --mark-filed")')
    .requiredOption('--type <type>', 'obligation type (from the country profile\'s filingTypes, e.g. OB/ICP/JAARREKENING for NL)')
    .requiredOption('--period <p>', '2026-Q3 (OB/ICP) or 2026 (JAARREKENING)')
    .option('--date <yyyy-mm-dd>', 'filing date (default: today)')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const r = markFiled(db, { type: opts.type, period: opts.period, date: opts.date, actor: ctx.actor, dryRun: ctx.dryRun });
          output(ctx, { filed: r }, (d) => {
            if (d.filed.dryRun) {
              console.log(`plan: mark ${d.filed.type} ${d.filed.period} filed on ${d.filed.filed_at}`);
              console.log('(dry run — nothing written)');
              return;
            }
            console.log(`${d.filed.type} ${d.filed.period} filed on ${d.filed.filed_at}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
