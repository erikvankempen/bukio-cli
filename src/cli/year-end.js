/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio year-end + jaarrekening + icp — annual close and statutory reports (Phase 4).
import { formatAmount } from '../core/money.js';
import { yearEndClose, yearEndStatus } from '../year-end/index.js';
import { jaarrekening } from '../report/jaarrekening.js';
import { jaarrekeningToPdf } from '../report/jaarrekening-pdf.js';
import { icpReadout } from '../icp/index.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

export function make(program) {
  const yearEnd = program.command('year-end').description('annual close (afsluiting boekjaar)');
  yearEnd
    .command('close')
    .description('close the fiscal year: result into 9900, then 3000 (source closing, P&L stays visible)')
    .requiredOption('--year <yyyy>', 'fiscal year')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const result = yearEndClose(db, { year: opts.year, actor: ctx.actor, dryRun: ctx.dryRun });
          if (result.dryRun) {
            output(ctx, result, (d) => {
              console.log(`plan: close ${d.year} — resultaat ${formatAmount(d.result_cents)}${d.create_9900 ? ' (creates account 9900)' : ''}`);
              for (const e of d.entries) {
                console.log(`  ${e.description}`);
                for (const p of e.postings) console.log(`    ${p.code}  ${formatAmount(p.amountCents).padStart(12)}`);
              }
              console.log('(dry run — nothing written)');
            });
            return;
          }
          output(ctx, result, (d) => {
            if (!d.closed) { console.log(d.message); return; }
            console.log(`${d.year} closed — resultaat ${formatAmount(d.result_cents)} (entries #${d.entries.map((e) => e.id).join(', #')}, posted)`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
  yearEnd
    .command('status')
    .description('is the year closed? what is the result?')
    .requiredOption('--year <yyyy>', 'fiscal year')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const s = yearEndStatus(db, { year: opts.year });
          output(ctx, { status: s }, (d) => {
            const st = d.status;
            console.log(`${st.year}: ${st.closed ? 'CLOSED' : 'open'} — resultaat ${formatAmount(st.result_cents)}`);
            for (const a of st.accounts) console.log(`  ${a.code}  ${a.name.padEnd(30)} ${formatAmount(a.net_cents)}`);
            for (const e of st.closing_entries) console.log(`  closing entry #${e.id} ${e.date} "${e.description}"`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  const jr = program.command('jaarrekening').description('statutory annual accounts (micro/klein)');
  jr
    .command('report')
    .description('annual accounts in the Dutch statutory layout')
    .requiredOption('--year <yyyy>', 'fiscal year')
    .option('--model <micro|klein>', 'statutory model', 'klein')
    .option('--format <json|pdf|xlsx>', 'output format', 'json')
    .option('--out <path>', 'output path (pdf/xlsx)')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const report = jaarrekening(db, { year: opts.year, model: opts.model });
          if (opts.format === 'json') {
            // --format json is the declared default — it must emit JSON even
            // without the global --json flag (parity with audit --format json)
            console.log(JSON.stringify({ ok: true, data: { jaarrekening: report } }, null, 2));
            return;
          }
          if (opts.format === 'pdf') {
            const outPath = opts.out ?? `jaarrekening-${opts.year}-${opts.model}.pdf`;
            const result = await jaarrekeningToPdf(report, { outPath });
            output(ctx, { path: result.path, bytes: result.bytes }, (d) => console.log(`wrote ${d.path} (${d.bytes} bytes)`));
            return;
          }
          if (opts.format === 'xlsx') {
            const { renderJaarrekeningXlsx } = await import('../report/jaarrekening-xlsx.js');
            const outPath = opts.out ?? `jaarrekening-${opts.year}-${opts.model}.xlsx`;
            const result = await renderJaarrekeningXlsx(report, { outPath });
            output(ctx, { path: result.path, bytes: result.bytes }, (d) => console.log(`wrote ${d.path} (${d.bytes} bytes)`));
            return;
          }
          throw Object.assign(new Error(`unknown format '${opts.format}' (use json|pdf|xlsx)`), { code: 'INVALID_FORMAT' });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  const icp = program.command('icp').description('ICP listing (intracommunautaire prestaties)');
  icp
    .command('readout')
    .description('EU btw-verlegde supplies per customer for the ICP listing (manual filing aid)')
    .requiredOption('--period <yyyy-qn>', 'quarter, e.g. 2026-Q3')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const r = icpReadout(db, { period: opts.period });
          output(ctx, r, (d) => {
            console.log(`ICP ${d.period}: ${d.customers.length} EU customer(s), total ${d.total}`);
            table(d.customers, [
              { key: 'name', label: 'klant' },
              { key: 'vat_id', label: 'btw-id' },
              { key: 'country', label: 'land' },
              { key: 'amount', label: 'bedrag' },
            ]);
            console.log(d.note);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}

