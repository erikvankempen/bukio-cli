/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio item — items catalog: reusable products/services with a quantity
// unit, unit price, default VAT code and optional revenue account.
import { ensureDb, makeCtx, output, fail, table } from './util.js';
import { parseAmount } from '../core/money.js';
import { createItem, getItem, listItems, updateItem } from '../items/index.js';
import { UNIT_CODES } from '../invoice/i18n.js';

function fmtItem(row) {
  return {
    id: row.id, name: row.name, description: row.description, unit: row.unit,
    unit_price: (row.unit_price_cents / 100).toFixed(2),
    vat_code: row.vat_code, gl_account: row.gl_account, active: row.active === 1,
  };
}

export function make(program) {
  const item = program.command('item').description('items catalog (products/services for invoices)');

  item
    .command('add')
    .description('add an item to the catalog')
    .requiredOption('--name <name>', 'item name')
    .option('--description <text>', 'line description printed on invoices (default: the name)')
    .option('--unit <code>', `quantity unit (${UNIT_CODES.join('|')})`, 'unit')
    .requiredOption('--price <amount>', 'unit price (e.g. 150.00)')
    .option('--vat <code>', 'default VAT code (e.g. 21)')
    .option('--gl <code>', 'revenue account (default 8000)')
    .option('--dry-run', 'validate without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const result = createItem(db, {
            name: opts.name, description: opts.description ?? null,
            unit: opts.unit, unitPriceCents: parseAmount(opts.price),
            vatCode: opts.vat ?? null, glAccount: opts.gl ?? null,
            actor: ctx.actor, dryRun: ctx.dryRun,
          });
          if (ctx.dryRun) {
            output(ctx, result, (d) => {
              console.log(`plan: add item '${d.name}' (${d.unit}, ${(d.unit_price_cents / 100).toFixed(2)})${d.vat_code ? ` @${d.vat_code}` : ''}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          output(ctx, { item: fmtItem(result), dryRun: false }, (d) => {
            console.log(`item #${d.item.id}: ${d.item.name} — ${d.item.unit_price} per ${d.item.unit}${d.item.vat_code ? ` @${d.item.vat_code}` : ''}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  item
    .command('list')
    .description('list the catalog')
    .option('--all', 'include deactivated items')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const items = listItems(db, { activeOnly: !opts.all }).map(fmtItem);
          output(ctx, { items }, (d) => {
            table(d.items, [
              { key: 'id', label: '#' },
              { key: 'name', label: 'name' },
              { key: 'unit', label: 'unit' },
              { key: 'unit_price', label: 'price' },
              { key: 'vat_code', label: 'VAT' },
              { key: 'gl_account', label: 'ledger' },
              { key: 'active', label: 'active' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  item
    .command('show')
    .description('show one item')
    .requiredOption('--id <id>', 'item id')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const row = getItem(db, opts.id);
          if (!row) throw Object.assign(new Error(`item ${opts.id} does not exist`), { code: 'ITEM_NOT_FOUND' });
          output(ctx, { item: fmtItem(row) }, (d) => {
            table([d.item], [
              { key: 'id', label: '#' },
              { key: 'name', label: 'name' },
              { key: 'description', label: 'description' },
              { key: 'unit', label: 'unit' },
              { key: 'unit_price', label: 'price' },
              { key: 'vat_code', label: 'VAT' },
              { key: 'gl_account', label: 'ledger' },
              { key: 'active', label: 'active' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  item
    .command('update')
    .description('update an item (price, unit, VAT, GL account) or deactivate it')
    .requiredOption('--id <id>', 'item id')
    .option('--name <name>', 'item name')
    .option('--description <text>', 'line description')
    .option('--unit <code>', `quantity unit (${UNIT_CODES.join('|')})`)
    .option('--price <amount>', 'unit price')
    .option('--vat <code>', 'default VAT code')
    .option('--gl <code>', 'revenue account')
    .option('--deactivate', 'deactivate the item (new invoices blocked, existing keep snapshots)')
    .option('--dry-run', 'validate without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const result = updateItem(db, {
            id: opts.id,
            name: opts.name ?? null,
            description: opts.description !== undefined ? opts.description : null,
            unit: opts.unit ?? null,
            unitPriceCents: opts.price !== undefined ? parseAmount(opts.price) : null,
            vatCode: opts.vat !== undefined ? opts.vat : null,
            glAccount: opts.gl !== undefined ? opts.gl : null,
            deactivate: Boolean(opts.deactivate),
            actor: ctx.actor, dryRun: ctx.dryRun,
          });
          if (ctx.dryRun) {
            output(ctx, result, (d) => {
              console.log(`plan: update item #${d.id}`);
              for (const [k, v] of Object.entries(d.changes)) console.log(`  ${k}: '${String(v)}'`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          output(ctx, { item: fmtItem(result), dryRun: false }, (d) => {
            console.log(`item #${d.item.id} updated: ${d.item.name} — ${d.item.unit_price} per ${d.item.unit}${d.item.active ? '' : ' (deactivated)'}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
