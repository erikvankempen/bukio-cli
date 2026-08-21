/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio account — chart of accounts management + CSV import.
import { openDb } from '../core/db.js';
import {
  createAccount, deactivateAccount, getAccountByCode, importChartCsv,
  listAccounts, reactivateAccount, readChartCsvFile, validateAccount,
} from '../core/accounts.js';
import { makeCtx, output, fail, ensureDb, table, withDb } from './util.js';
import { record } from '../audit/index.js';

function serializeAccount(a) {
  return {
    code: a.code, name: a.name, type: a.type, taxonomy_code: a.taxonomy_code,
    normal_balance: a.normal_balance, active: Boolean(a.active),
  };
}

export function make(program) {
  const account = program.command('account').description('chart of accounts');

  account
    .command('add')
    .description('add an account to the chart')
    .requiredOption('--code <code>', 'account code (1-6 digits)')
    .requiredOption('--name <name>', 'account name')
    .requiredOption('--type <type>', 'asset|liability|equity|income|expense')
    .requiredOption('--normal-balance <debit|credit>', 'normal balance')
    .option('--taxonomy-code <code>', 'taxonomy reference code (RGS for NL, e.g. BMVA.02)')
    .option('--rgs-code <code>', '[deprecated] alias for --taxonomy-code')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      const warnings = [];
      try {
        const db = ensureDb(ctx);
        try {
          // deprecated alias: --rgs-code maps to the generic flag (warns);
          // primary + alias together: alias ignored WITH a warning
          if (opts.rgsCode != null && opts.taxonomyCode != null) {
            warnings.push('--rgs-code ignored because --taxonomy-code was also given');
          } else if (opts.rgsCode != null) {
            opts.taxonomyCode = opts.rgsCode;
            warnings.push('--rgs-code is deprecated — use --taxonomy-code');
          }
          if (ctx.dryRun) {
            const plan = { code: opts.code, name: opts.name, type: opts.type, normal_balance: opts.normalBalance, taxonomy_code: opts.taxonomyCode ?? null };
            validateAccount({ code: opts.code, name: opts.name, type: opts.type, normalBalance: opts.normalBalance, taxonomyCode: opts.taxonomyCode });
            const exists = getAccountByCode(db, opts.code);
            output(ctx, { action: 'add account', account: plan, exists: Boolean(exists), ...(warnings.length ? { warnings } : {}), dryRun: true }, (d) => {
              console.log(`plan: add account ${d.account.code} ${d.account.name} (${d.account.type}/${d.account.normal_balance})`);
              console.log(d.exists ? `(note: account ${d.account.code} already exists)` : '');
              for (const w of d.warnings ?? []) console.error(`warning: ${w}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          const accountRow = createAccount(db, {
            code: opts.code, name: opts.name, type: opts.type,
            normalBalance: opts.normalBalance, taxonomyCode: opts.taxonomyCode ?? null,
          });
          record(db, {
            actor: ctx.actor, action: 'account.add', command: 'account add',
            args: { code: opts.code, name: opts.name, type: opts.type, normal_balance: opts.normalBalance, taxonomy_code: opts.taxonomyCode ?? null },
            outcome: 'ok',
          });
          output(ctx, { ...serializeAccount(accountRow), ...(warnings.length ? { warnings } : {}) }, (a) => {
            console.log(`added account ${a.code} ${a.name} (${a.type}/${a.normal_balance})`);
            for (const w of a.warnings ?? []) console.error(`warning: ${w}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  account
    .command('list')
    .description('list accounts')
    .option('--type <type>', 'filter by type')
    .option('--include-inactive', 'include inactive accounts')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const rows = listAccounts(db, { type: opts.type || null, includeInactive: Boolean(opts.includeInactive) });
        const data = { accounts: rows.map(serializeAccount) };
        output(ctx, data, (d) => {
          table(d.accounts, [
            { key: 'code', label: 'code' },
            { key: 'name', label: 'name' },
            { key: 'type', label: 'type' },
            { key: 'normal_balance', label: 'bal' },
            { key: 'taxonomy_code', label: 'rgs' },
            { key: 'active', label: 'active' },
          ]);
        });
    }));

  account
    .command('show')
    .description('show one account')
    .requiredOption('--code <code>', 'account code')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const a = getAccountByCode(db, opts.code);
        if (!a) throw Object.assign(new Error(`account ${opts.code} does not exist`), { code: 'ACCOUNT_NOT_FOUND' });
        output(ctx, serializeAccount(a), (row) => {
          console.log(`${row.code}  ${row.name}`);
          console.log(`type: ${row.type}  normal balance: ${row.normal_balance}  rgs: ${row.taxonomy_code ?? '-'}  active: ${row.active}`);
        });
    }));

  account
    .command('deactivate')
    .description('deactivate an account (blocks new postings; history stays)')
    .requiredOption('--code <code>', 'account code')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const a = getAccountByCode(db, opts.code);
        if (!a) throw Object.assign(new Error(`account ${opts.code} does not exist`), { code: 'ACCOUNT_NOT_FOUND' });
        if (ctx.dryRun) {
          output(ctx, { action: 'deactivate account', code: opts.code, current: a.active ? 'active' : 'inactive', dryRun: true }, (d) => {
            console.log(`plan: deactivate account ${d.code} (${d.current} -> inactive)`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        const updated = deactivateAccount(db, opts.code);
        record(db, {
          actor: ctx.actor, action: 'account.deactivate', command: 'account deactivate',
          args: { code: opts.code }, outcome: 'ok',
        });
        output(ctx, serializeAccount(updated), (d) => console.log(`deactivated account ${d.code} ${d.name}`));
    }));

  account
    .command('reactivate')
    .description('reactivate an account')
    .requiredOption('--code <code>', 'account code')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const updated = reactivateAccount(db, opts.code, { dryRun: ctx.dryRun });
        if (!ctx.dryRun) {
          record(db, {
            actor: ctx.actor, action: 'account.reactivate', command: 'account reactivate',
            args: { code: opts.code }, outcome: 'ok',
          });
        }
        output(ctx, { account: updated }, (d) => {
          if (d.account.dryRun) {
            console.log(`plan: reactivate account ${d.account.code} (${d.account.from} -> ${d.account.to})`);
            console.log('(dry run — nothing written)');
            return;
          }
          console.log(`reactivated account ${d.account.code} ${d.account.name}`);
        });
    }));

  account
    .command('import')
    .description('import a chart from CSV: code,name,type,normal_balance[,taxonomy_code]')
    .requiredOption('--file <path>', 'chart CSV file')
    .option('--dry-run', 'validate the file without importing')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const csvText = readChartCsvFile(opts.file);
        if (ctx.dryRun) {
          const tmp = openDb(':memory:');
          const result = importChartCsv(tmp, csvText);
          tmp.close();
          output(ctx, { action: 'import chart', file: opts.file, ...result, dryRun: true }, (d) => {
            console.log(`plan: import chart from ${d.file} — ${d.created} valid, ${d.skipped} skipped`);
            for (const e of d.errors) console.log(`  line ${e.line}: ${e.error}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        const db = ensureDb(ctx);
        try {
          const result = importChartCsv(db, csvText);
          record(db, {
            actor: ctx.actor, action: 'account.import', command: 'account import',
            args: { file: opts.file, created: result.created, skipped: result.skipped },
            outcome: 'ok',
          });
          output(ctx, { file: opts.file, ...result, dryRun: false }, (d) => {
            console.log(`imported ${d.created} of ${d.total} accounts from ${d.file}`);
            for (const e of d.errors) console.log(`  line ${e.line}: ${e.error}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
