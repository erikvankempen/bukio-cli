/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

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
    .option('--dry-run', 'show the plan without writing the file')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          if (!ctx.dryRun) mkdirSync(path.dirname(path.resolve(opts.out)), { recursive: true });
          const result = exportXaf(db, {
            year: opts.year, out: opts.out, actor: ctx.actor, dryRun: ctx.dryRun,
          });
          if (result.dryRun) {
            output(ctx, result, (d) => {
              console.log(`plan: export XAF 4.0 for ${d.company.name} (KVK ${d.company.kvk || '-'}) — fiscal year ${d.year}`);
              console.log(`  ${d.rekeningen} rekeningen, ${d.mutaties} mutaties -> ${d.path}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
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
