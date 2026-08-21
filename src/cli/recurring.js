/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio recurring + depreciation — scheduled entries (FR3A).
import { formatAmount } from '../core/money.js';
import { parseImportAmount } from '../import/index.js';
import {
  buildDepreciationTemplate, createTemplate, listTemplates, getTemplate,
  previewDue, runDue, setTemplateStatus,
} from '../recurring/index.js';
import { ensureDb, makeCtx, output, fail, table, withDb, cliError } from './util.js';

const FREQUENCIES = ['monthly', 'quarterly', 'yearly'];

function fmtPostings(postings) {
  return postings.map((p) => ({
    code: p.code, amount_cents: p.amountCents, amount: formatAmount(p.amountCents),
    vat_code: p.vatCode ?? null,
  }));
}

function fmtTemplate(t) {
  return {
    ...(t.action ? { action: t.action } : {}),
    ...(t.from ? { from: t.from } : {}),
    ...(t.to ? { to: t.to } : {}),
    ...(t.dryRun !== undefined ? { dryRun: t.dryRun } : {}),
    id: t.id, name: t.name, description: t.description,
    kind: t.kind ?? 'entry', contact_id: t.contact_id ?? null,
    invoice_lines: t.invoice_lines ?? null,
    frequency: t.frequency, day_of_period: t.day_of_period,
    start_date: t.start_date, end_date: t.end_date, runs: t.runs,
    status: t.status, next_run_date: t.next_run_date, last_run_date: t.last_run_date,
    runs_done: t.runs_done, reverse_previous: Boolean(t.reverse_previous),
    vat_aware: Boolean(t.vat_aware),
    // a dry-run plan ({action,id,from,to,dryRun}) has no template body —
    // render what exists instead of crashing on undefined postings
    postings: t.postings ? fmtPostings(t.postings) : null,
    final_postings: t.final_postings ? fmtPostings(t.final_postings) : null,
  };
}

export function make(program) {
  const recurring = program.command('recurring').description('recurring entries: templates, schedule, generation');

  recurring
    .command('add')
    .description('create a recurring template (entry postings or subscription invoices)')
    .requiredOption('--name <name>', 'template name (also the entry description prefix)')
    .option('--kind <entry|invoice>', 'template kind', 'entry')
    .option('--contact <id>', 'contact id (required for --kind invoice)')
    .option('--lines <spec>', 'invoice line specs "[QTYx] DESC @ PRICE [@ VATCODE] [@ -DISCOUNT]" (for --kind invoice)', (v, acc) => [...acc, v], [])
    .option('--items <spec>', 'item specs "ID[:QTY][@PRICE][@VATCODE][@-DISCOUNT]" (for --kind invoice; prices snapshotted per run)', (v, acc) => [...acc, v], [])
    .option('--postings <CODE:AMOUNT[@VAT]>', 'posting specs, comma-separated or repeatable (required for --kind entry)', (v, acc) => [...acc, v], [])
    .requiredOption('--frequency <frequency>', `one of ${FREQUENCIES.join(', ')}`)
    .requiredOption('--start <yyyy-mm-dd>', 'first run date')
    .option('--day <n>', 'day of period to book on (1-28)', '1')
    .option('--end <yyyy-mm-dd>', 'last run date')
    .option('--runs <n>', 'maximum number of runs')
    .option('--due-days <n>', 'payment term for invoice templates (days)', '30')
    .option('--desc <description>', 'template description')
    .option('--reverse-previous', 'accrual pattern: reverse the previous generated entry on each run')
    .option('--dry-run', 'validate without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        if (ctx.dryRun) {
          // same validation as the real path (createTemplate dryRun): the
          // old hand-built plan echoed garbage inputs as ok:true
          const plan = createTemplate(db, {
            name: opts.name, description: opts.desc ?? null, frequency: opts.frequency,
            dayOfPeriod: Number(opts.day), startDate: opts.start, endDate: opts.end ?? null,
            runs: opts.runs ? Number(opts.runs) : null,
            postings: opts.postings ?? [],
            reversePrevious: Boolean(opts.reversePrevious),
            kind: opts.kind, contactId: opts.contact ? Number(opts.contact) : null,
            invoiceLines: opts.lines?.length ? opts.lines : null,
            invoiceItems: opts.items?.length ? opts.items : null,
            dueDays: Number(opts.dueDays),
            actor: ctx.actor, dryRun: true,
          });
          output(ctx, plan, (d) => {
            console.log(`plan: ${d.kind} template "${d.name}" (${d.frequency}, day ${d.day_of_period}) from ${d.start_date}`);
            if (d.kind === 'invoice') console.log(`  contact #${d.contact_id}: ${d.lines}`);
            else console.log(`  postings: ${d.postings}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        const tpl = createTemplate(db, {
          name: opts.name, description: opts.desc ?? null, frequency: opts.frequency,
          dayOfPeriod: Number(opts.day), startDate: opts.start, endDate: opts.end ?? null,
          runs: opts.runs ? Number(opts.runs) : null,
          postings: opts.postings ?? [],
          reversePrevious: Boolean(opts.reversePrevious),
          kind: opts.kind, contactId: opts.contact ? Number(opts.contact) : null,
          invoiceLines: opts.lines?.length ? opts.lines : null,
          invoiceItems: opts.items?.length ? opts.items : null,
          dueDays: Number(opts.dueDays),
          actor: ctx.actor,
        });
        output(ctx, { template: fmtTemplate(tpl), dryRun: false }, (d) => {
          const t = d.template;
          console.log(`template #${t.id} "${t.name}" [${t.kind}] — next run ${t.next_run_date} (${t.frequency})`);
          if (t.kind === 'invoice') {
            console.log(`  contact #${t.contact_id}: ${(t.invoice_items ?? t.invoice_lines)?.map((l) => (typeof l === 'string' ? l : `${l.quantity / 1000}x ${l.description}`)).join(' + ')}`);
          }
        });
    }));

  recurring
    .command('list')
    .description('list recurring templates')
    .option('--status <status>', 'active|paused|completed|all', 'active')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const rows = listTemplates(db, { status: opts.status }).map(fmtTemplate);
        output(ctx, { templates: rows }, (d) => {
          table(d.templates, [
            { key: 'id', label: '#' },
            { key: 'name', label: 'name' },
            { key: 'kind', label: 'kind' },
            { key: 'frequency', label: 'freq' },
            { key: 'next_run_date', label: 'next' },
            { key: 'runs_done', label: 'done' },
            { key: 'status', label: 'status' },
          ]);
        });
    }));

  recurring
    .command('show')
    .description('show one template with its postings')
    .requiredOption('--id <id>', 'template id')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const tpl = getTemplate(db, opts.id);
        if (!tpl) throw cliError('NOT_FOUND', `recurring template ${opts.id} does not exist`);
        output(ctx, { template: fmtTemplate(tpl) }, (d) => {
          console.log(`#${d.template.id} ${d.template.name} [${d.template.status}] — ${d.template.frequency}, day ${d.template.day_of_period}`);
          console.log(`  start ${d.template.start_date}  next ${d.template.next_run_date}  runs ${d.template.runs_done}/${d.template.runs ?? '∞'}`);
          for (const p of d.template.postings) console.log(`  ${p.code}  ${p.amount.padStart(12)}${p.vat_code ? ` @${p.vat_code}` : ''}`);
          if (d.template.final_postings) console.log('  (final run:)');
          for (const p of d.template.final_postings ?? []) console.log(`  ${p.code}  ${p.amount.padStart(12)}`);
        });
    }));

  recurring
    .command('pause')
    .description('pause a template')
    .requiredOption('--id <id>', 'template id')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const tpl = setTemplateStatus(db, { id: opts.id, status: 'paused', actor: ctx.actor, dryRun: ctx.dryRun });
        output(ctx, { template: fmtTemplate(tpl) }, (d) => {
          if (tpl.dryRun) { console.log(`plan: pause template #${tpl.id} (${tpl.from} -> ${tpl.to})`); console.log('(dry run — nothing written)'); return; }
          console.log(`paused template #${d.template.id}`);
        });
    }));

  recurring
    .command('resume')
    .description('resume a paused template')
    .requiredOption('--id <id>', 'template id')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const tpl = setTemplateStatus(db, { id: opts.id, status: 'active', actor: ctx.actor, dryRun: ctx.dryRun });
        output(ctx, { template: fmtTemplate(tpl) }, (d) => {
          if (tpl.dryRun) { console.log(`plan: resume template #${tpl.id} (${tpl.from} -> ${tpl.to})`); console.log('(dry run — nothing written)'); return; }
          console.log(`resumed template #${d.template.id}`);
        });
    }));

  recurring
    .command('preview')
    .description('show what is due (no writes)')
    .option('--as-of <yyyy-mm-dd>', 'reference date (default today)')
    .option('--template <id>', 'only this template')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const plan = previewDue(db, { asOf: opts.asOf ?? null, templateId: opts.template ?? null });
        output(ctx, plan, (d) => {
          console.log(`due as of ${d.as_of}: ${d.templates.length} template(s) due`);
          for (const t of d.templates) {
            for (const run of t.runs) {
              if (run.kind === 'invoice') {
                console.log(`  [INVOICE] ${run.invoice.date}  ${run.invoice.contact_name ?? 'contact'} — ${run.invoice.lines?.map((l) => `${l.quantity}x ${l.description}`).join(' + ')}`);
                continue;
              }
              const kind = run.kind === 'reversal' ? 'REVERSE' : 'BOOK';
              console.log(`  [${kind}] ${run.entry.date}  ${run.entry.description}`);
              for (const p of run.entry.postings) console.log(`      ${p.code}  ${formatAmount(p.amountCents).padStart(12)}`);
            }
          }
        });
    }));

  recurring
    .command('run')
    .description('generate all due entries (idempotent, backfills missed periods)')
    .option('--as-of <yyyy-mm-dd>', 'reference date (default today)')
    .option('--template <id>', 'only this template')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const result = runDue(db, {
          asOf: opts.asOf ?? null, templateId: opts.template ?? null,
          actor: ctx.actor, dryRun: ctx.dryRun,
        });
        const data = {
          as_of: result.as_of, dry_run: result.dry_run,
          templates: result.templates.map((t) => ({
            template_id: t.template_id, name: t.name, ok: t.ok, error: t.error ?? null,
            runs: t.runs.map((r) => ({
              kind: r.kind ?? (r.generated ? undefined : 'entry'),
              entries: r.generated ? r.generated.map((g) => (
                g.kind === 'invoice'
                  ? { kind: 'invoice', invoice_id: g.invoice.id, date: g.invoice.date ?? g.entry?.date, state: 'draft' }
                  : { kind: g.kind, entry_id: g.entry.id, date: g.entry.date, state: g.entry.state }
              )) : [{
                // dry-run plan entries have no id yet — render as plans
                kind: r.kind ?? 'entry', entry_id: r.entry?.id ?? null,
                date: r.entry?.date, state: 'plan',
              }],
            })),
          })),
        };
        output(ctx, data, (d) => {
          const total = d.templates.reduce((s, t) => s + t.runs.length, 0);
          const failed = d.templates.filter((t) => !t.ok).length;
          console.log(`recurring run: ${total} period(s) across ${d.templates.length} template(s)${d.dry_run ? ' (dry run)' : ''}`);
          for (const t of d.templates) {
            if (!t.ok) { console.log(`  ✗ ${t.name}: ${t.error.code} — ${t.error.message}`); continue; }
            for (const r of t.runs) {
              for (const e of r.entries) {
                if (e.kind === 'invoice') {
                  console.log(`  ${e.date}  → draft invoice #${e.invoice_id} (finalize to book & number)`);
                } else if (e.entry_id == null) {
                  console.log(`  ${e.date}  ${e.kind === 'reversal' ? '→ reversal of previous entry' : '→ entry'} (plan)`);
                } else {
                  console.log(`  ${e.date}  ${e.kind === 'reversal' ? '→ reversal of' : '→ booked'} entry #${e.entry_id} (${e.state})`);
                }
              }
            }
          }
          if (failed) console.log(`  ${failed} template(s) failed — their schedules were left untouched`);
        });
    }));

  const dep = program.command('depreciation').description('depreciation schedules (linear, remainder-adjusted final run)');
  dep
    .command('add')
    .description('create a monthly depreciation template for an asset')
    .requiredOption('--name <name>', 'asset name')
    .requiredOption('--cost <amount>', 'purchase cost (e.g. 5370.00)')
    .requiredOption('--life-months <n>', 'useful life in months')
    .requiredOption('--start <yyyy-mm-dd>', 'first depreciation month')
    .option('--asset <code>', 'asset account', '1800')
    .option('--expense <code>', 'depreciation expense account', '4600')
    .option('--residual <amount>', 'residual value at end of life', '0')
    .option('--desc <description>', 'template description')
    .option('--dry-run', 'show the computed schedule without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const costCents = parseImportAmount(opts.cost);
        const residualCents = parseImportAmount(opts.residual);
        const lifeMonths = Number(opts.lifeMonths);
        const result = buildDepreciationTemplate(db, {
          name: opts.name, assetCode: opts.asset, expenseCode: opts.expense,
          costCents, residualCents, lifeMonths, startDate: opts.start,
          description: opts.desc ?? null, actor: ctx.actor, dryRun: ctx.dryRun,
        });
        if (result.dryRun) {
          output(ctx, {
            action: 'create depreciation template', name: opts.name,
            asset: opts.asset, expense: opts.expense,
            cost_cents: costCents, residual_cents: residualCents, life_months: lifeMonths,
            monthly_cents: result.monthly_cents, final_cents: result.final_cents,
            monthly: formatAmount(result.monthly_cents), final: formatAmount(result.final_cents),
            dryRun: true,
          }, (d) => {
            console.log(`plan: depreciate "${d.name}" ${d.monthly}/mo for ${d.life_months} months (final ${d.final})`);
            console.log(`  ${d.asset} (asset) -${d.monthly}  /  ${d.expense} (expense) +${d.monthly}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        output(ctx, {
          template: fmtTemplate(result.template),
          monthly: result.monthly, final: result.final,
          total: (result.total_cents / 100).toFixed(2),
        }, (d) => {
          console.log(`template #${d.template.id} — ${d.template.name}: ${d.monthly}/mo × ${d.template.runs - 1} + final ${d.final} = ${d.total}`);
        });
    }));
}
