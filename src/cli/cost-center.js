/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio cost-center — registry CRUD for the analytical (management-reporting)
// dimension. Cost centers tag postings; they carry no legal/statutory meaning.
import { ensureDb, makeCtx, output, fail, table, withDb, cliError } from './util.js';
import { record } from '../audit/index.js';
import {
  createCostCenter, deactivateCostCenter, getCostCenterByCode, listCostCenters,
  reactivateCostCenter,
} from '../core/cost-centers.js';

function serialize(cc) {
  return { id: cc.id, code: cc.code, name: cc.name, active: Boolean(cc.active) };
}

export function make(program) {
  const cc = program.command('cost-center').description('cost centers (analytical dimension for reporting)');

  cc.command('add')
    .description('add a cost center')
    .requiredOption('--code <code>', 'cost center code (e.g. HQ, SALES)')
    .requiredOption('--name <name>', 'cost center name')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          if (ctx.dryRun) {
            output(ctx, { action: 'add cost center', code: opts.code, name: opts.name, dryRun: true }, (d) => {
              console.log(`plan: add cost center ${d.code} ${d.name}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          const row = createCostCenter(db, { code: opts.code, name: opts.name });
          record(db, {
            actor: ctx.actor, action: 'cost-center.add', command: 'cost-center add',
            args: { code: opts.code, name: opts.name }, outcome: 'ok',
          });
          output(ctx, { ...serialize(row) }, (d) => console.log(`added cost center ${d.code} ${d.name}`));
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  cc.command('list')
    .description('list cost centers')
    .option('--include-inactive', 'include inactive cost centers')
    .action((opts, command) => withDb(command, (ctx, db) => {
      const rows = listCostCenters(db, { includeInactive: Boolean(opts.includeInactive) });
      const data = { cost_centers: rows.map(serialize) };
      output(ctx, data, (d) => {
        table(d.cost_centers, [
          { key: 'code', label: 'code' }, { key: 'name', label: 'name' }, { key: 'active', label: 'active' },
        ]);
      });
    }));

  cc.command('show')
    .description('show one cost center')
    .requiredOption('--code <code>', 'cost center code')
    .action((opts, command) => withDb(command, (ctx, db) => {
      const row = getCostCenterByCode(db, opts.code);
      if (!row) throw cliError('COST_CENTER_NOT_FOUND', `cost center '${opts.code}' does not exist`);
      output(ctx, serialize(row), (d) => console.log(`${d.code}  ${d.name}  (active: ${d.active})`));
    }));

  cc.command('deactivate')
    .description('deactivate a cost center (blocks new taggings; history stays)')
    .requiredOption('--code <code>', 'cost center code')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
      const row = getCostCenterByCode(db, opts.code);
      if (!row) throw cliError('COST_CENTER_NOT_FOUND', `cost center '${opts.code}' does not exist`);
      if (ctx.dryRun) {
        output(ctx, { action: 'deactivate cost center', code: opts.code, dryRun: true }, (d) => {
          console.log(`plan: deactivate cost center ${d.code}`);
          console.log('(dry run — nothing written)');
        });
        return;
      }
      const updated = deactivateCostCenter(db, opts.code);
      record(db, { actor: ctx.actor, action: 'cost-center.deactivate', command: 'cost-center deactivate', args: { code: opts.code }, outcome: 'ok' });
      output(ctx, { cost_center: updated }, (d) => console.log(`deactivated cost center ${d.cost_center.code}`));
    }));

  cc.command('reactivate')
    .description('reactivate a cost center')
    .requiredOption('--code <code>', 'cost center code')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
      const row = getCostCenterByCode(db, opts.code);
      if (!row) throw cliError('COST_CENTER_NOT_FOUND', `cost center '${opts.code}' does not exist`);
      if (ctx.dryRun) {
        output(ctx, { action: 'reactivate cost center', code: opts.code, dryRun: true }, (d) => {
          console.log(`plan: reactivate cost center ${d.code}`);
          console.log('(dry run — nothing written)');
        });
        return;
      }
      const updated = reactivateCostCenter(db, opts.code);
      record(db, { actor: ctx.actor, action: 'cost-center.reactivate', command: 'cost-center reactivate', args: { code: opts.code }, outcome: 'ok' });
      output(ctx, { cost_center: updated }, (d) => console.log(`reactivated cost center ${d.cost_center.code}`));
    }));
}
