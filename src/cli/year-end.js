/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio year-end + jaarrekening + icp — annual close and statutory reports (Phase 4).
import { formatAmount } from '../core/money.js';
import { yearEndClose, yearEndStatus } from '../year-end/index.js';
import { jaarrekening } from '../report/jaarrekening.js';
import { jaarrekeningToPdf } from '../report/jaarrekening-pdf.js';
import { icpReadout } from '../icp/index.js';
import { ensureDb, makeCtx, output, fail, table, cliError } from './util.js';
import { t, resolveLocale } from '../i18n/index.js';

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
        const locale = resolveLocale(ctx, db);
        try {
          const result = yearEndClose(db, { year: opts.year, actor: ctx.actor, dryRun: ctx.dryRun });
          if (result.dryRun) {
            output(ctx, result, (d) => {
              console.log(t('yearend.plan', { year: d.year, amount: formatAmount(d.result_cents), extra: d.create_9900 ? ' (creates account 9900)' : '' }, locale));
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
            console.log(t('yearend.closed', { year: d.year, amount: formatAmount(d.result_cents), entries: d.entries.map((e) => e.id).join(', #') }, locale));
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
        const locale = resolveLocale(ctx, db);
        try {
          const s = yearEndStatus(db, { year: opts.year });
          output(ctx, { status: s }, (d) => {
            const st = d.status;
            console.log(t('yearend.status', { year: st.year, state: st.closed ? 'CLOSED' : 'open', amount: formatAmount(st.result_cents) }, locale));
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

  const reportAction = (deprecated) => async (opts, command) => {
    const ctx = makeCtx(command);
    const warnings = deprecated ? ['jaarrekening is deprecated — use `financial-statements report`'] : undefined;
    try {
      const db = ensureDb(ctx);
      try {
        const report = jaarrekening(db, { year: opts.year, model: opts.model });
        if (opts.format === 'json') {
          // --format json is the declared default — it must emit JSON even
          // without the global --json flag (parity with audit --format json)
          console.log(JSON.stringify({
            ok: true,
            data: { financial_statements: report, ...(warnings ? { warnings } : {}) },
          }, null, 2));
          return;
        }
        if (opts.format === 'pdf') {
          const outPath = opts.out ?? `financial-statements-${opts.year}-${report.model}.pdf`;
          const result = await jaarrekeningToPdf(report, { outPath });
          output(ctx, { path: result.path, bytes: result.bytes, ...(warnings ? { warnings } : {}) },
            (d) => {
              console.log(`wrote ${d.path} (${d.bytes} bytes)`);
              for (const w of d.warnings ?? []) console.error(`warning: ${w}`);
            });
          return;
        }
        if (opts.format === 'xlsx') {
          const { renderJaarrekeningXlsx } = await import('../report/jaarrekening-xlsx.js');
          const outPath = opts.out ?? `financial-statements-${opts.year}-${report.model}.xlsx`;
          const result = await renderJaarrekeningXlsx(report, { outPath });
          output(ctx, { path: result.path, bytes: result.bytes, ...(warnings ? { warnings } : {}) },
            (d) => {
              console.log(`wrote ${d.path} (${d.bytes} bytes)`);
              for (const w of d.warnings ?? []) console.error(`warning: ${w}`);
            });
          return;
        }
        throw cliError('INVALID_FORMAT', `unknown format '${opts.format}' (use json|pdf|xlsx)`);
      } finally {
        db.close();
      }
    } catch (err) {
      fail(ctx, err);
    }
  };

  const fsCmd = program.command('financial-statements').description('statutory annual accounts (micro/klein)');
  fsCmd
    .command('report')
    .description('annual accounts in the jurisdiction statutory layout')
    .requiredOption('--year <yyyy>', 'fiscal year')
    .option('--model <model>', 'statutory model (per the country profile: NL micro|klein, LU abrege)')
    .option('--format <json|pdf|xlsx>', 'output format', 'json')
    .option('--out <path>', 'output path (pdf/xlsx)')
    .action(reportAction(false));

  // deprecated alias: the Dutch name is jurisdiction data, the command is not
  const jr = program.command('jaarrekening').description('[deprecated] alias for financial-statements');
  jr
    .command('report')
    .description('[deprecated] alias for financial-statements report')
    .requiredOption('--year <yyyy>', 'fiscal year')
    .option('--model <model>', 'statutory model (per the country profile: NL micro|klein, LU abrege)')
    .option('--format <json|pdf|xlsx>', 'output format', 'json')
    .option('--out <path>', 'output path (pdf/xlsx)')
    .action(reportAction(true));

  const icp = program.command('icp').description('ICP listing (intra-community supplies)');
  icp
    .command('readout')
    .description('EU reverse-charge supplies per customer for the ICP listing (manual filing aid)')
    .requiredOption('--period <yyyy-qn>', 'quarter, e.g. 2026-Q3')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        const locale = resolveLocale(ctx, db);
        try {
          const r = icpReadout(db, { period: opts.period });
          output(ctx, r, (d) => {
            console.log(`ICP ${d.period}: ${d.customers.length} EU customer(s), total ${d.total}`);
            table(d.customers, [
              { key: 'name', label: 'customer' },
              { key: 'vat_id', label: 'tax id' },
              { key: 'country', label: 'country' },
              { key: 'amount', label: 'amount' },
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

