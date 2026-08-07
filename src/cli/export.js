// bukio export — Auditfile Financieel (XAF) 4.0 export for external advisors.
import { mkdirSync } from 'node:fs';
import path from 'node:path';
import { exportXaf } from '../export/index.js';
import { ensureDb, makeCtx, output, fail } from './util.js';

export function make(program) {
  const exp = program.command('export').description('export data for external advisors: XAF 4.0 audit file');

  exp
    .command('xaf')
    .description('export the fiscal year as an Auditfile Financieel 4.0 XML (for your boekhouder, tax advisor or auditor)')
    .requiredOption('--year <yyyy>', 'fiscal year to export')
    .requiredOption('--out <path>', 'output .xaf file')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          mkdirSync(path.dirname(path.resolve(opts.out)), { recursive: true });
          const result = exportXaf(db, {
            year: opts.year, out: opts.out, actor: ctx.actor,
          });
          output(ctx, result, (d) => {
            console.log(`wrote ${d.path}`);
            console.log(`  ${d.company.name} (KVK ${d.company.kvk || '-'}) — fiscal year ${d.year}`);
            console.log(`  ${d.rekeningen} rekeningen, ${d.mutaties} mutaties`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
