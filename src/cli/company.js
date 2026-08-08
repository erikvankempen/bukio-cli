/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio company — show the company record, update company details.
import { ensureDb, makeCtx, output, fail, table } from './util.js';
import { isValidIban } from '../core/iban.js';
import { record } from '../audit/index.js';

const COMPANY_FIELDS = [
  ['name', 'name', 'company name'],
  ['kvk', 'kvk', 'KVK number'],
  ['btwId', 'btw_id', 'BTW identification number'],
  ['iban', 'iban', 'bank account (IBAN)'],
  ['address', 'address', 'street address (for compliant invoices)'],
  ['postalCode', 'postal_code', 'postal code'],
  ['city', 'city', 'city'],
];

function serializeCompany(row) {
  return {
    id: row.id, name: row.name, kvk: row.kvk, legal_form: row.legal_form,
    btw_id: row.btw_id, iban: row.iban, address: row.address,
    postal_code: row.postal_code, city: row.city, vat_module: row.vat_module,
    kor_flag: row.kor_flag, fiscal_year_end: row.fiscal_year_end,
  };
}

function getCompany(db) {
  return db.prepare('SELECT * FROM company WHERE id = 1').get() ?? null;
}

export function make(program) {
  const company = program.command('company').description('company record: show, update');

  company
    .command('show')
    .description('show the company record')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const row = getCompany(db);
          if (!row) throw new Error('no company — run bukio init first');
          output(ctx, { company: serializeCompany(row) }, (d) => {
            table([d.company], [
              { key: 'name', label: 'naam' },
              { key: 'kvk', label: 'kvk' },
              { key: 'legal_form', label: 'rechtsvorm' },
              { key: 'btw_id', label: 'btw-id' },
              { key: 'iban', label: 'iban' },
              { key: 'address', label: 'adres' },
              { key: 'postal_code', label: 'postcode' },
              { key: 'city', label: 'plaats' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  company
    .command('update')
    .description('update company details (address, IBAN, btw-id, name, kvk)')
    .option('--name <name>', 'company name')
    .option('--kvk <kvk>', 'KVK number')
    .option('--btw-id <id>', 'BTW identification number')
    .option('--iban <iban>', 'bank account (IBAN)')
    .option('--address <address>', 'street address (for compliant invoices)')
    .option('--postal-code <code>', 'postal code')
    .option('--city <city>', 'city')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const row = getCompany(db);
          if (!row) throw new Error('no company — run bukio init first');

          const changes = {};
          for (const [opt, col, label] of COMPANY_FIELDS) {
            if (opts[opt] !== undefined) changes[col] = String(opts[opt]).trim();
          }
          if (Object.keys(changes).length === 0) {
            throw Object.assign(new Error('nothing to update — pass at least one of --name/--kvk/--btw-id/--iban/--address/--postal-code/--city'), { code: 'NOTHING_TO_UPDATE' });
          }
          for (const [opt, col, label] of COMPANY_FIELDS) {
            if (changes[col] === '' && col !== 'btw_id') {
              throw Object.assign(new Error(`${label} cannot be empty`), { code: 'INVALID_VALUE' });
            }
          }
          if (changes.iban && !isValidIban(changes.iban)) {
            throw Object.assign(new Error(`invalid IBAN '${changes.iban}'`), { code: 'INVALID_IBAN' });
          }

          const plan = { company: { ...serializeCompany(row), ...changes }, changes, dryRun: true };
          if (ctx.dryRun) {
            output(ctx, plan, (d) => {
              console.log('plan: company update');
              for (const [k, v] of Object.entries(d.changes)) console.log(`  ${k}: '${row[k]}' -> '${v}'`);
              console.log('(dry run — nothing written)');
            });
            return;
          }

          const sets = Object.keys(changes).map((c) => `${c} = ?`).join(', ');
          db.prepare(`UPDATE company SET ${sets} WHERE id = 1`).run(...Object.values(changes));
          const updated = getCompany(db);
          record(db, {
            actor: ctx.actor, action: 'company.update', command: 'company update',
            args: changes, outcome: 'ok',
          });
          output(ctx, { company: serializeCompany(updated), changes }, (d) => {
            console.log('company updated:');
            for (const [k, v] of Object.entries(d.changes)) console.log(`  ${k}: '${row[k]}' -> '${v}'`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
