/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio report — trial balance, balance sheet, pnl, journal; CSV/XLSX export.
import { writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { formatAmount } from '../core/money.js';
import { trialBalance } from '../report/trial-balance.js';
import { balans } from '../report/balans.js';
import { pnl } from '../report/pnl.js';
import { journal } from '../report/journal.js';
import { aging } from '../report/aging.js';
import { sales } from '../report/sales.js';
import { toCsv, writeXlsx } from '../report/export.js';
import { fiscalYearWindow } from '../year-end/index.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

function todayIso() {
  return new Date().toISOString().slice(0, 10);
}
function currentYear() {
  return todayIso().slice(0, 4);
}

/** Shared emit: json | csv | xlsx | human. */
export async function emitReport(ctx, opts, data, { csvColumns, csvRows, sheets, render }) {
  const format = opts.format || (ctx.json ? 'json' : 'human');
  if (format === 'json') {
    console.log(JSON.stringify({ ok: true, data }, null, 2));
    return;
  }
  if (format === 'csv') {
    const csv = toCsv(csvRows(data), csvColumns);
    if (opts.out) {
      mkdirSync(path.dirname(path.resolve(opts.out)), { recursive: true });
      writeFileSync(opts.out, csv);
      console.log(`wrote ${opts.out}`);
    } else {
      process.stdout.write(csv);
    }
    return;
  }
  if (format === 'xlsx') {
    if (!opts.out) throw Object.assign(new Error('--out <path> is required for xlsx output'), { code: 'OUT_REQUIRED' });
    mkdirSync(path.dirname(path.resolve(opts.out)), { recursive: true });
    await writeXlsx(opts.out, sheets(data));
    console.log(`wrote ${opts.out}`);
    return;
  }
  render(data);
}

export function make(program) {
  const report = program.command('report').description('reports');

  report
    .command('trial-balance')
    .description('per-account debit/credit/net from posted entries')
    .option('--year <yyyy>', 'filter by year')
    .option('--format <format>', 'json|csv|xlsx|human')
    .option('--out <path>', 'output file (csv/xlsx)')
    .action(async (opts, command) => {
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
          await emitReport(ctx, opts, data, {
            csvColumns: [
              { key: 'code', label: 'code' }, { key: 'name', label: 'account' },
              { key: 'type', label: 'type' }, { key: 'debit', label: 'debit' },
              { key: 'credit', label: 'credit' }, { key: 'net', label: 'net' },
            ],
            csvRows: (d) => [
              ...d.accounts.map((a) => ({ ...a })),
              { code: '', name: 'TOTAAL', type: '', debit: d.total_debit, credit: d.total_credit, net: formatAmount(d.total_debit_cents - d.total_credit_cents) },
            ],
            sheets: (d) => [{
              name: 'Trial balance',
              columns: [
                { header: 'code', key: 'code' }, { header: 'account', key: 'name' },
                { header: 'type', key: 'type' }, { header: 'debit', key: 'debit' },
                { header: 'credit', key: 'credit' }, { header: 'net', key: 'net' },
              ],
              rows: d.accounts,
            }],
            render: (d) => {
              table(d.accounts, [
                { key: 'code', label: 'code' }, { key: 'name', label: 'account' },
                { key: 'type', label: 'type' }, { key: 'debit', label: 'debit' },
                { key: 'credit', label: 'credit' }, { key: 'net', label: 'net' },
              ]);
              console.log(`totals:  debit ${d.total_debit}  credit ${d.total_credit}  -> ${d.balanced ? 'BALANCED' : 'UNBALANCED!'}`);
            },
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  report
    .command('balance-sheet')
    .alias('balans') // deprecated alias, kept for compatibility
    .description('balance sheet as of a date (assets = liabilities + equity + result)')
    .option('--as-of <yyyy-mm-dd>', 'balance date')
    .option('--format <format>', 'json|csv|xlsx|human')
    .option('--out <path>', 'output file (csv/xlsx)')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const asOf = opts.asOf || todayIso();
          const b = balans(db, { asOf });
          const fmt = (c) => formatAmount(c);
          const data = {
            as_of: b.as_of,
            assets: { total_cents: b.assets.total_cents, total: fmt(b.assets.total_cents), sections: b.assets.sections },
            liabilities_and_equity: {
              total_cents: b.liabilities_and_equity.total_cents,
              total: fmt(b.liabilities_and_equity.total_cents),
              sections: b.liabilities_and_equity.sections,
              result_cents: b.liabilities_and_equity.result_cents,
              result: fmt(b.liabilities_and_equity.result_cents),
            },
            balanced: b.balanced,
          };
          const flatRows = (d) => [
            ...d.assets.sections.flatMap((s) => s.accounts.map((a) => ({
              side: 'activa', rgs: s.taxonomy_code, group: s.label, code: a.code, name: a.name, amount: fmt(a.balance_cents),
            }))),
            ...d.liabilities_and_equity.sections.flatMap((s) => s.accounts.map((a) => ({
              side: 'passiva', rgs: s.taxonomy_code, group: s.label, code: a.code, name: a.name, amount: fmt(a.balance_cents),
            }))),
            { side: 'passiva', rgs: '', group: 'Nog te verdelen resultaat', code: '', name: '', amount: d.liabilities_and_equity.result },
          ];
          await emitReport(ctx, opts, data, {
            csvColumns: [
              { key: 'side', label: 'side' }, { key: 'rgs', label: 'rgs' }, { key: 'group', label: 'group' },
              { key: 'code', label: 'code' }, { key: 'name', label: 'name' }, { key: 'amount', label: 'amount' },
            ],
            csvRows: flatRows,
            sheets: (d) => [{
              name: 'Balance Sheet',
              columns: [
                { header: 'side', key: 'side' }, { header: 'rgs', key: 'rgs' }, { header: 'group', key: 'group' },
                { header: 'code', key: 'code' }, { header: 'name', key: 'name' }, { header: 'amount', key: 'amount' },
              ],
              rows: flatRows(d),
            }],
            render: (d) => {
              console.log(`BALANCE SHEET as of ${d.as_of}`);
              console.log('ACTIVA');
              for (const s of d.assets.sections) {
                console.log(`  ${s.label} (${s.taxonomy_code})`);
                for (const a of s.accounts) console.log(`    ${a.code}  ${a.name.padEnd(30)} ${fmt(a.balance_cents)}`);
                console.log(`    ${''.padEnd(32)} ${fmt(s.total_cents)}`);
              }
              console.log(`  totaal activa: ${d.assets.total}`);
              console.log('PASSIVA');
              for (const s of d.liabilities_and_equity.sections) {
                console.log(`  ${s.label} (${s.taxonomy_code})`);
                for (const a of s.accounts) console.log(`    ${a.code}  ${a.name.padEnd(30)} ${fmt(a.balance_cents)}`);
                console.log(`    ${''.padEnd(32)} ${fmt(s.total_cents)}`);
              }
              console.log(`  Nog te verdelen resultaat  ${d.liabilities_and_equity.result}`);
              console.log(`  totaal passiva: ${d.liabilities_and_equity.total}`);
              console.log(d.balanced ? 'BALANCED' : 'UNBALANCED!');
            },
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  report
    .command('pnl')
    .description('winst- en verliesrekening for a period')
    .option('--year <yyyy>', 'fiscal year (overrides --from/--to)')
    .option('--from <yyyy-mm-dd>', 'period start (inclusive)')
    .option('--to <yyyy-mm-dd>', 'period end (inclusive)')
    .option('--format <format>', 'json|csv|xlsx|human')
    .option('--out <path>', 'output file (csv/xlsx)')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const year = opts.year || currentYear();
          const [fyFrom, fyTo] = fiscalYearWindow(db, year);
          const from = opts.from || fyFrom;
          const to = opts.to || fyTo;
          const p = pnl(db, { from, to });
          const fmt = (c) => formatAmount(c);
          const data = {
            from: p.from, to: p.to,
            sections: p.sections,
            revenue_cents: p.revenue_cents, revenue: fmt(p.revenue_cents),
            costs_cents: p.costs_cents, costs: fmt(p.costs_cents),
            result_cents: p.result_cents, result: fmt(p.result_cents),
          };
          const flatRows = (d) => [
            ...d.sections.flatMap((s) => s.accounts.map((a) => ({
              rgs: s.taxonomy_code, group: s.label, code: a.code, name: a.name, amount: fmt(a.amount_cents),
            }))),
            { rgs: '', group: 'Netto resultaat', code: '', name: '', amount: d.result },
          ];
          await emitReport(ctx, opts, data, {
            csvColumns: [
              { key: 'rgs', label: 'rgs' }, { key: 'group', label: 'group' },
              { key: 'code', label: 'code' }, { key: 'name', label: 'name' }, { key: 'amount', label: 'amount' },
            ],
            csvRows: flatRows,
            sheets: (d) => [{
              name: 'Winst en verlies',
              columns: [
                { header: 'rgs', key: 'rgs' }, { header: 'group', key: 'group' },
                { header: 'code', key: 'code' }, { header: 'name', key: 'name' }, { header: 'amount', key: 'amount' },
              ],
              rows: flatRows(d),
            }],
            render: (d) => {
              console.log(`WINST- EN VERLIESREKENING ${d.from} .. ${d.to}`);
              for (const s of d.sections) {
                console.log(`  ${s.label} (${s.taxonomy_code})`);
                for (const a of s.accounts) console.log(`    ${a.code}  ${a.name.padEnd(30)} ${fmt(a.amount_cents)}`);
                console.log(`    ${''.padEnd(32)} ${fmt(s.total_cents)}`);
              }
              console.log(`  opbrengsten: ${d.revenue}`);
              console.log(`  kosten:      ${d.costs}`);
              console.log(`  resultaat:   ${d.result}`);
            },
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  report
    .command('journal')
    .description('journal export (one row per posting) for a period')
    .option('--year <yyyy>', 'fiscal year (overrides --from/--to)')
    .option('--from <yyyy-mm-dd>', 'period start (inclusive)')
    .option('--to <yyyy-mm-dd>', 'period end (inclusive)')
    .option('--format <format>', 'json|csv|xlsx|human')
    .option('--out <path>', 'output file (csv/xlsx)')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const year = opts.year || currentYear();
          const [fyFrom, fyTo] = fiscalYearWindow(db, year);
          const from = opts.from || fyFrom;
          const to = opts.to || fyTo;
          const rows = journal(db, { from, to });
          const data = {
            from, to,
            rows: rows.map((r) => ({
              ...r,
              amount: r.amount_cents == null ? '' : formatAmount(r.amount_cents),
            })),
          };
          await emitReport(ctx, opts, data, {
            csvColumns: [
              { key: 'date', label: 'date' }, { key: 'entry_id', label: 'entry' },
              { key: 'description', label: 'description' }, { key: 'source', label: 'source' },
              { key: 'state', label: 'state' }, { key: 'account_code', label: 'code' },
              { key: 'account_name', label: 'account' }, { key: 'amount', label: 'amount' },
            ],
            csvRows: (d) => d.rows,
            sheets: (d) => [{
              name: 'Journal',
              columns: [
                { header: 'date', key: 'date' }, { header: 'entry', key: 'entry_id' },
                { header: 'description', key: 'description' }, { header: 'source', key: 'source' },
                { header: 'state', key: 'state' }, { header: 'code', key: 'account_code' },
                { header: 'account', key: 'account_name' }, { header: 'amount', key: 'amount' },
              ],
              rows: d.rows,
            }],
            render: (d) => {
              table(d.rows, [
                { key: 'date', label: 'date' }, { key: 'entry_id', label: '#' },
                { key: 'state', label: 'state' }, { key: 'account_code', label: 'code' },
                { key: 'account_name', label: 'account' }, { key: 'amount', label: 'amount' },
                { key: 'description', label: 'description' },
              ]);
            },
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  report
    .command('aging')
    .description('open items per contact, bucketed by days past due (30/60/90+)')
    .option('--as-of <yyyy-mm-dd>', 'aging date (default: today)')
    .option('--kind <kind>', 'debtors | creditors | both (default: both)')
    .option('--format <format>', 'json|csv|xlsx|human')
    .option('--out <path>', 'output file (csv/xlsx)')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const data = aging(db, { asOf: opts.asOf || null, kind: opts.kind || 'both' });
          const flat = (d) => {
            const rows = [];
            for (const side of ['debtors', 'creditors']) {
              if (!d[side]) continue;
              for (const c of d[side].contacts) {
                rows.push({
                  side,
                  contact_id: c.contact_id,
                  name: c.name ?? '',
                  current: formatAmount(c.buckets.current),
                  d30: formatAmount(c.buckets.d30),
                  d60: formatAmount(c.buckets.d60),
                  d90: formatAmount(c.buckets.d90),
                  d90plus: formatAmount(c.buckets.d90plus),
                  in_batch: formatAmount(c.in_batch_cents),
                  total: formatAmount(c.total_cents),
                });
              }
            }
            return rows;
          };
          const csvColumns = [
            { key: 'side', label: 'side' }, { key: 'contact_id', label: 'contact_id' },
            { key: 'name', label: 'name' }, { key: 'current', label: 'current' },
            { key: 'd30', label: '30d' }, { key: 'd60', label: '60d' },
            { key: 'd90', label: '90d' }, { key: 'd90plus', label: '90d+' },
            { key: 'in_batch', label: 'in_batch' }, { key: 'total', label: 'total' },
          ];
          await emitReport(ctx, opts, data, {
            csvColumns,
            csvRows: flat,
            sheets: (d) => [{ name: 'Aging', columns: csvColumns.map((c) => ({ header: c.label, key: c.key })), rows: flat(d) }],
            render: (d) => {
              for (const side of ['debtors', 'creditors']) {
                if (!d[side]) continue;
                console.log(`--- ${side} (as of ${d.as_of}) ---`);
                if (!d[side].contacts.length) {
                  console.log('(none)');
                  continue;
                }
                table(d[side].contacts.map((c) => ({
                  contact: `${c.contact_id} ${c.name ?? ''}`.trim(),
                  current: formatAmount(c.buckets.current),
                  d30: formatAmount(c.buckets.d30),
                  d60: formatAmount(c.buckets.d60),
                  d90: formatAmount(c.buckets.d90),
                  d90plus: formatAmount(c.buckets.d90plus),
                  in_batch: formatAmount(c.in_batch_cents),
                  total: formatAmount(c.total_cents),
                })), [
                  { key: 'contact', label: 'Contact' },
                  { key: 'current', label: 'Current' },
                  { key: 'd30', label: '30d' },
                  { key: 'd60', label: '60d' },
                  { key: 'd90', label: '90d' },
                  { key: 'd90plus', label: '90d+' },
                  { key: 'in_batch', label: 'In batch' },
                  { key: 'total', label: 'Total' },
                ]);
                console.log(`total ${side}: ${formatAmount(d[side].totals.total_cents)}`);
              }
            },
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  report
    .command('sales')
    .description('sales revenue for a year, by contact or by item')
    .option('--year <yyyy>', 'fiscal year (default: current)')
    .option('--by <dim>', 'contact | item (default: contact)')
    .option('--format <format>', 'json|csv|xlsx|human')
    .option('--out <path>', 'output file (csv/xlsx)')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const year = opts.year || currentYear();
          const by = opts.by || 'contact';
          const data = sales(db, { year, by });
          const fmt = (g) => (by === 'contact'
            ? {
              key: g.contact_id, name: g.name ?? '', invoice_count: g.invoice_count,
              net: formatAmount(g.net_cents), vat: formatAmount(g.vat_cents), gross: formatAmount(g.gross_cents),
            }
            : { key: g.key, name: g.name, line_count: g.line_count, net: formatAmount(g.net_cents) });
          const csvColumns = by === 'contact'
            ? [
              { key: 'key', label: 'contact_id' }, { key: 'name', label: 'name' },
              { key: 'invoice_count', label: 'invoices' }, { key: 'net', label: 'net' },
              { key: 'vat', label: 'vat' }, { key: 'gross', label: 'gross' },
            ]
            : [
              { key: 'key', label: 'key' }, { key: 'name', label: 'name' },
              { key: 'line_count', label: 'lines' }, { key: 'net', label: 'net' },
            ];
          const rows = (d) => d.groups.map(fmt);
          await emitReport(ctx, opts, data, {
            csvColumns,
            csvRows: rows,
            sheets: (d) => [{ name: 'Sales', columns: csvColumns.map((c) => ({ header: c.label, key: c.key })), rows: rows(d) }],
            render: (d) => {
              console.log(`--- sales ${d.year} by ${d.by} ---`);
              if (!d.groups.length) {
                console.log('(no sales)');
                return;
              }
              table(rows(d), csvColumns.map((c) => ({ key: c.key, label: c.label })));
              if (by === 'contact') {
                console.log(`totals: ${d.totals.invoice_count} invoices, net ${formatAmount(d.totals.net_cents)}, vat ${formatAmount(d.totals.vat_cents)}, gross ${formatAmount(d.totals.gross_cents)}`);
              } else {
                console.log(`totals: ${d.totals.line_count} lines, net ${formatAmount(d.totals.net_cents)}`);
              }
            },
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
