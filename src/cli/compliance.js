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
            table(c.obligations, [
              { key: 'type', label: 'type' },
              { key: 'period', label: 'periode' },
              { key: 'deadline', label: 'deadline' },
              { key: 'status', label: 'status' },
            ], (row) => {
              const extra = row.books_closed === false && row.type === 'JAARREKENING' ? ' (books not closed)' : '';
              row.status = `${row.status}${extra}`;
              return row;
            });
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
    .requiredOption('--type <OB|ICP|JAARREKENING>', 'obligation type')
    .requiredOption('--period <p>', '2026-Q3 (OB/ICP) or 2026 (JAARREKENING)')
    .option('--date <yyyy-mm-dd>', 'filing date (default: today)')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const r = markFiled(db, { type: opts.type, period: opts.period, date: opts.date, actor: ctx.actor });
          output(ctx, { filed: r }, (d) => console.log(`${d.filed.type} ${d.filed.period} filed on ${d.filed.filed_at}`));
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
