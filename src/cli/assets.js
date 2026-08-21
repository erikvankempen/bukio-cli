/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio assets — fixed assets: depreciation schemes, asset register with
// mid-life adoption, monthly depreciation runs, disposal, asset register.
import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import {
  listSchemes, createScheme, addAsset, listAssets, getAsset,
  runDue, register, disposeAsset,
} from '../assets/index.js';
import { formatAmount } from '../core/money.js';
import { parseImportAmount } from '../import/index.js';
import { ensureDb, makeCtx, output, fail, table, withDb } from './util.js';
import { toCsv, writeXlsx } from '../report/export.js';
import { record } from '../audit/index.js';

const ASSET_COLUMNS = [
  { key: 'id', label: 'id' },
  { key: 'name', label: 'name' },
  { key: 'category', label: 'category' },
  { key: 'status', label: 'status' },
  { key: 'purchase_date', label: 'purchase' },
  { key: 'purchase_price', label: 'purchase price' },
  { key: 'total_cum_dep', label: 'cum. deprec.' },
  { key: 'book_value', label: 'book value' },
];

function fmtAssetRow(r) {
  return {
    id: r.id, name: r.name, category: r.category ?? '', status: r.status,
    purchase_date: r.purchase_date,
    purchase_price: formatAmount(r.purchase_price_cents),
    total_cum_dep: formatAmount(r.total_cum_dep_cents),
    book_value: formatAmount(r.book_value_cents),
  };
}

function sheets(data) {
  // writeXlsx expects [{ name, columns: [{header,key}], rows: [{key: value}] }]
  // — a {header} flat-array shape crashed every `assets register --format xlsx`
  const columns = ASSET_COLUMNS.map((c) => ({ header: c.label, key: c.key }));
  const rows = data.assets.map(fmtAssetRow);
  rows.push({
    id: 'TOTAL', name: '', category: '', status: '', purchase_date: '',
    purchase_price: formatAmount(data.totals.purchase_price_cents),
    total_cum_dep: formatAmount(data.totals.total_cum_dep_cents),
    book_value: formatAmount(data.totals.book_value_cents),
  });
  return [{ name: 'Fixed asset register', columns, rows }];
}

export function make(program) {
  const assets = program.command('assets').description('fixed assets: schemes, register, depreciation runs, disposal');

  const schemeCmd = assets.command('scheme').description('depreciation schemes (lineair | degressief)');

  schemeCmd
    .command('add')
    .description('create a depreciation scheme (default: 5 years lineair, 0% residual)')
    .option('--name <name>', 'scheme name')
    .option('--method <lineair|degressief>', 'depreciation method', 'lineair')
    .option('--life-months <n>', 'useful life in months', '60')
    .option('--residual-bp <bp>', 'residual value in basis points of cost (0-10000)', '0')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const plan = {
          name: opts.name || `Standard ${opts.lifeMonths} months ${opts.method}`, method: opts.method,
          lifeMonths: Number(opts.lifeMonths), residualBp: Number(opts.residualBp),
        };
        if (ctx.dryRun) {
          // validation runs in dry-run too (createScheme checks life/residual bounds)
          const checked = createScheme(db, { ...plan, actor: ctx.actor, dryRun: true });
          output(ctx, { ...checked, dryRun: true }, (d) => console.log(`plan: scheme '${d.name}' — ${d.method}, ${d.life_months} months, residual ${d.residual_bp}bp (dry run)`));
          return;
        }
        const s = createScheme(db, { ...plan, actor: ctx.actor });
        output(ctx, { scheme: s }, (d) => console.log(`scheme #${d.scheme.id} '${d.scheme.name}' — ${d.scheme.method}, ${d.scheme.life_months} months, residual ${d.scheme.residual_bp}bp`));
    }));

  schemeCmd.command('list').description('list depreciation schemes')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const schemes = listSchemes(db);
        output(ctx, { schemes }, (d) => {
          table(d.schemes.map((s) => ({ id: s.id, name: s.name, method: s.method, life_months: s.life_months, residual_bp: s.residual_bp })), [
            { key: 'id', label: 'id' }, { key: 'name', label: 'name' },
            { key: 'method', label: 'method' }, { key: 'life_months', label: 'months' },
            { key: 'residual_bp', label: 'residual value bp' },
          ]);
        });
    }));

  assets
    .command('add')
    .description('register an asset (already booked in the ledger) with its depreciation scheme')
    .requiredOption('--name <name>', 'asset name')
    .option('--category <cat>', 'category (e.g. ICT, Inventaris, Vervoer)')
    .option('--serial <ref>', 'serial number / reference')
    .requiredOption('--purchase-date <yyyy-mm-dd>', 'purchase date')
    .requiredOption('--purchase-price <amount>', 'purchase price (e.g. 1511.74)')
    .option('--residual <amount>', 'residual value override (default: from scheme)')
    .requiredOption('--depreciation-start <yyyy-mm-dd>', 'date depreciation started')
    .requiredOption('--recognition-date <yyyy-mm-dd>', 'date the asset enters this module')
    .option('--cum-dep <amount>', 'cumulative depreciation at recognition (default 0)')
    .option('--scheme <id>', 'depreciation scheme id (default: standard 5y lineair)')
    .option('--method <lineair|degressief>', 'inline scheme method (when not using --scheme)')
    .option('--life-months <n>', 'inline scheme life in months (when not using --scheme)')
    .option('--residual-bp <bp>', 'inline scheme residual basis points')
    .option('--asset-account <code>', 'asset GL account', '1800')
    .option('--cum-dep-account <code>', 'cumulative-depreciation GL account (default: book on the asset account)')
    .option('--expense-account <code>', 'depreciation expense GL account', '4600')
    .option('--entry-id <n>', 'link the purchase booking entry (e.g. from a purchase invoice)')
    .option('--note <text>', 'free-form note')
    .option('--dry-run', 'show the computed plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const result = addAsset(db, {
          name: opts.name, category: opts.category, serial: opts.serial,
          schemeId: opts.scheme ? Number(opts.scheme) : null,
          method: opts.method, lifeMonths: opts.lifeMonths ? Number(opts.lifeMonths) : null,
          residualBp: opts.residualBp !== undefined ? Number(opts.residualBp) : null,
          residualCents: opts.residual !== undefined ? parseImportAmount(opts.residual) : null,
          purchaseDate: opts.purchaseDate,
          purchasePriceCents: parseImportAmount(opts.purchasePrice),
          depreciationStartDate: opts.depreciationStart,
          recognitionDate: opts.recognitionDate,
          cumDepAtRecognitionCents: parseImportAmount(opts.cumDep ?? '0'),
          assetAccount: opts.assetAccount, cumDepAccount: opts.cumDepAccount,
          expenseAccount: opts.expenseAccount,
          entryId: opts.entryId ? Number(opts.entryId) : null,
          note: opts.note, actor: ctx.actor, dryRun: ctx.dryRun,
        });
        if (ctx.dryRun) {
          output(ctx, result, (d) => {
            const a = d.asset;
            console.log(`plan: asset '${a.name}' — ${a.scheme} (${a.method}, ${a.life_months} months)`);
            console.log(`  purchase ${formatAmount(a.purchase_price_cents)} on ${a.purchase_date}, residual ${formatAmount(a.residual_cents)}`);
            console.log(`  recognised ${a.recognition_date} with cum. depreciation ${formatAmount(a.cum_dep_at_recognition_cents)}`);
            console.log(`  ${a.months_left} months left, first run ${a.first_run_period} — ${a.asset_account}/${a.cum_dep_account ?? 'asset'}/${a.expense_account}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        output(ctx, result, (d) => {
          const a = d.asset;
          console.log(`asset #${a.id} '${a.name}' — status ${a.status}`);
          for (const w of d.warnings) console.log(`  warning: ${w}`);
        });
    }));

  assets
    .command('list')
    .description('list assets')
    .option('--status <active|paused|fully_depreciated|disposed>', 'filter by status')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const rows = listAssets(db, { status: opts.status });
        const data = rows.map((r) => ({
          id: r.id, name: r.name, category: r.category, status: r.status,
          purchase_date: r.purchase_date, purchase_price_cents: r.purchase_price_cents,
          scheme: r.scheme.name,
        }));
        output(ctx, { assets: data }, (d) => {
          table(d.assets.map((a) => ({ ...a, purchase_price: formatAmount(a.purchase_price_cents) })), [
            { key: 'id', label: 'id' }, { key: 'name', label: 'name' },
            { key: 'category', label: 'category' }, { key: 'status', label: 'status' },
            { key: 'purchase_date', label: 'purchase' },
            { key: 'purchase_price', label: 'value' }, { key: 'scheme', label: 'scheme' },
          ]);
        });
    }));

  assets
    .command('show')
    .description('one asset with its scheme and run history')
    .requiredOption('--id <n>', 'asset id')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const asset = getAsset(db, Number(opts.id));
        if (!asset) throw Object.assign(new Error(`asset ${opts.id} does not exist`), { code: 'ASSET_NOT_FOUND' });
        const runs = db.prepare('SELECT period, amount_cents, entry_id FROM asset_depreciation_runs WHERE asset_id = ? ORDER BY period').all(asset.id);
        const data = { asset, runs };
        output(ctx, data, (d) => {
          console.log(`asset #${d.asset.id} '${d.asset.name}' (${d.asset.status}) — ${d.asset.scheme.name}`);
          console.log(`  purchase ${formatAmount(d.asset.purchase_price_cents)} on ${d.asset.purchase_date}, residual ${formatAmount(d.asset.residual_cents)}`);
          console.log(`  depreciation ${d.asset.depreciation_start_date} -> recognised ${d.asset.recognition_date} with cum. dep ${formatAmount(d.asset.cum_dep_at_recognition_cents)}`);
          console.log(`  accounts: ${d.asset.asset_account_code} / ${d.asset.cum_dep_account_code ?? 'asset'} / ${d.asset.expense_account_code}`);
          if (d.asset.entry_id) console.log(`  linked purchase entry: #${d.asset.entry_id}`);
          if (d.runs.length) {
            console.log('  runs:');
            for (const r of d.runs) console.log(`    ${r.period}  ${formatAmount(r.amount_cents)}  (entry #${r.entry_id})`);
          }
        });
    }));

  assets
    .command('run')
    .description('book the depreciation runs that are due (idempotent per asset-month)')
    .option('--period <yyyy-mm>', 'book all runs due up to and including this period')
    .option('--as-of <yyyy-mm-dd>', 'book everything due up to this date')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const result = runDue(db, {
          period: opts.period, asOf: opts.asOf, actor: ctx.actor, dryRun: ctx.dryRun,
        });
        if (ctx.dryRun) {
          output(ctx, { plan: result.plan, dryRun: true }, (d) => {
            if (!d.plan.length) { console.log('plan: nothing due (dry run)'); return; }
            for (const p of d.plan) {
              console.log(`  asset #${p.asset_id} ${p.name}: ${p.periods.map((s) => `${s.period} ${formatAmount(s.amountCents)}`).join(', ')}`);
            }
            console.log('(dry run — nothing written)');
          });
          return;
        }
        output(ctx, result, (d) => {
          console.log(`booked ${d.booked.length} depreciation run${d.booked.length === 1 ? '' : 's'}`);
          for (const b of d.booked) console.log(`  ${b.period} asset #${b.asset_id} ${formatAmount(b.amount_cents)} (entry #${b.entry_id})`);
        });
    }));

  assets
    .command('register')
    .description('fixed asset register: cost, cumulative depreciation, book value per asset (as of a date)')
    .option('--as-of <yyyy-mm-dd>', 'cut-off date (default: today)')
    .option('--format <json|csv|xlsx>', 'output format', 'json')
    .option('--out <path>', 'file to write (csv/xlsx)')
    .action((opts, command) => withDb(command, async (ctx, db) => {
        const data = register(db, { asOf: opts.asOf, actor: ctx.actor });
        const format = opts.format;
        if (format === 'csv') {
          const totaalRow = {
            id: '', name: 'TOTAL', category: '', status: '',
            purchase_date: '', purchase_price: formatAmount(data.totals.purchase_price_cents),
            total_cum_dep: formatAmount(data.totals.total_cum_dep_cents),
            book_value: formatAmount(data.totals.book_value_cents),
          };
          const csv = toCsv([...data.assets.map(fmtAssetRow), totaalRow], ASSET_COLUMNS);
          if (opts.out) {
            mkdirSync(path.dirname(path.resolve(opts.out)), { recursive: true });
            writeFileSync(opts.out, csv);
            console.log(`wrote ${opts.out}`);
          } else process.stdout.write(csv);
          return;
        }
        if (format === 'xlsx') {
          if (!opts.out) throw Object.assign(new Error('--out <path> is required for xlsx output'), { code: 'OUT_REQUIRED' });
          mkdirSync(path.dirname(path.resolve(opts.out)), { recursive: true });
          await writeXlsx(opts.out, sheets(data));
          console.log(`wrote ${opts.out}`);
          return;
        }
        if (format === 'json') {
          // --format json is the declared default — it must emit JSON even
          // without the global --json flag (parity with audit --format json)
          console.log(JSON.stringify({ ok: true, data }, null, 2));
          return;
        }
        output(ctx, data, (d) => {
          console.log(`fixed asset register ${d.as_of}`);
          table(d.assets.map(fmtAssetRow), ASSET_COLUMNS);
          console.log(`total: ${formatAmount(d.totals.purchase_price_cents)} / ${formatAmount(d.totals.total_cum_dep_cents)} / ${formatAmount(d.totals.book_value_cents)}`);
        });
    }));

  assets
    .command('dispose')
    .description('dispose of an asset (sale or scrap): proposes the full booking, status -> disposed')
    .requiredOption('--id <n>', 'asset id')
    .requiredOption('--date <yyyy-mm-dd>', 'disposal date')
    .option('--proceeds <amount>', 'sale proceeds (default 0 = scrapped)', '0')
    .option('--bank-account <code>', 'bank account for the proceeds (default 1100)')
    .option('--result-account <code>', 'profit/loss account (default 8100)')
    .option('--note <text>', 'free-form note')
    .option('--dry-run', 'show the proposed entry without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const result = disposeAsset(db, {
          id: Number(opts.id), date: opts.date,
          proceedsCents: parseImportAmount(opts.proceeds),
          bankAccount: opts.bankAccount, resultAccount: opts.resultAccount,
          note: opts.note, actor: ctx.actor, dryRun: ctx.dryRun,
        });
        if (ctx.dryRun) {
          output(ctx, result, (d) => {
            console.log(`plan: dispose '${d.asset.name}' on ${d.date} — proceeds ${formatAmount(d.proceeds_cents)}, book value ${formatAmount(d.book_value_cents)}, result ${formatAmount(d.result_cents)}`);
            for (const p of d.postings) console.log(`  ${p.code} ${formatAmount(p.amountCents)}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        output(ctx, result, (d) => {
          console.log(`disposed '${d.asset.name}' — entry #${d.entry.id}, result ${formatAmount(d.result_cents)}`);
          for (const p of d.postings) console.log(`  ${p.code} ${formatAmount(p.amountCents)}`);
        });
    }));

  assets
    .command('pause')
    .description('pause depreciation for an asset (status -> paused)')
    .requiredOption('--id <n>', 'asset id')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const asset = getAsset(db, Number(opts.id));
        if (!asset) throw Object.assign(new Error(`asset ${opts.id} does not exist`), { code: 'ASSET_NOT_FOUND' });
        if (asset.status !== 'active') throw Object.assign(new Error(`asset ${opts.id} is ${asset.status}, only active assets can be paused`), { code: 'INVALID_STATUS' });
        if (ctx.dryRun) {
          output(ctx, { action: 'assets.pause', asset_id: asset.id, name: asset.name, from: asset.status, to: 'paused', dryRun: true }, (d) => {
            console.log(`plan: pause asset #${d.asset_id} '${d.name}' (${d.from} -> ${d.to})`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        db.prepare("UPDATE assets SET status = 'paused' WHERE id = ?").run(asset.id);
        record(db, { actor: ctx.actor, action: 'assets.pause', command: 'assets pause', args: { asset_id: asset.id }, outcome: 'ok' });
        output(ctx, { asset: { id: asset.id, name: asset.name, status: 'paused' } }, (d) => console.log(`asset #${d.asset.id} '${d.asset.name}' paused`));
    }));

  assets
    .command('resume')
    .description('resume depreciation for an asset (status -> active)')
    .requiredOption('--id <n>', 'asset id')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const asset = getAsset(db, Number(opts.id));
        if (!asset) throw Object.assign(new Error(`asset ${opts.id} does not exist`), { code: 'ASSET_NOT_FOUND' });
        if (asset.status !== 'paused') throw Object.assign(new Error(`asset ${opts.id} is ${asset.status}, only paused assets can be resumed`), { code: 'INVALID_STATUS' });
        if (ctx.dryRun) {
          output(ctx, { action: 'assets.resume', asset_id: asset.id, name: asset.name, from: asset.status, to: 'active', dryRun: true }, (d) => {
            console.log(`plan: resume asset #${d.asset_id} '${d.name}' (${d.from} -> ${d.to})`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        db.prepare("UPDATE assets SET status = 'active' WHERE id = ?").run(asset.id);
        record(db, { actor: ctx.actor, action: 'assets.resume', command: 'assets resume', args: { asset_id: asset.id }, outcome: 'ok' });
        output(ctx, { asset: { id: asset.id, name: asset.name, status: 'active' } }, (d) => console.log(`asset #${d.asset.id} '${d.asset.name}' resumed`));
    }));
}
