// bukio payments — SEPA payment batches (pain.001 export for bank-portal
// upload), payables (purchase invoices: transfer vs direct debit).
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import { readImportFile, parseImportAmount } from '../import/index.js';
import {
  addPayable, listPayables, markPayablePaid,
  createPaymentBatch, createPaymentBatchFromCsv,
  exportPaymentBatch, deletePaymentBatch, listPaymentBatches, getPaymentBatch,
} from '../payments/index.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

const parseLinesSpec = (spec) => String(spec ?? '').split(';').map((s) => s.trim()).filter(Boolean).map((s) => {
  const parts = s.split(':');
  const contact = parts[0];
  const amountCents = parseImportAmount(parts[1]);
  const reference = parts.slice(2).join(':') || null;
  return { contact, amountCents, reference };
});

export function make(program) {
  const payments = program.command('payments').description('SEPA payment batches (pain.001) and payables');

  const payables = payments.command('payables').description('purchase invoices to pay');
  payables
    .command('add')
    .description('register a purchase invoice (payable)')
    .requiredOption('--contact <contact>', 'contact id or name')
    .requiredOption('--ref <ref>', 'vendor invoice number')
    .requiredOption('--date <date>', 'invoice date YYYY-MM-DD')
    .option('--due <date>', 'due date YYYY-MM-DD')
    .requiredOption('--amount <amount>', 'amount (e.g. 123.45 or 123,45)')
    .option('--method <method>', "payment term: 'transfer' (SEPA batch) or 'direct-debit' (incasso, excluded from batches)", 'transfer')
    .option('--entry-id <id>', 'optional: linked expense booking (entry id)')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const amountCents = parseImportAmount(opts.amount);
          const r = addPayable(db, {
            contact: opts.contact, invoiceRef: opts.ref, date: opts.date, dueDate: opts.due,
            amountCents, method: opts.method === 'direct-debit' ? 'direct_debit' : 'transfer',
            entryId: opts.entryId ? Number(opts.entryId) : null, actor: ctx.actor, dryRun: opts.dryRun,
          });
          output(ctx, r, (d) => {
            if (d.dryRun) { console.log(`plan: payable '${d.contact_name}' ${d.invoice_ref} ${(d.amount_cents / 100).toFixed(2)} (${d.payment_method})`); console.log('(dry run — nothing written)'); return; }
            console.log(`payable #${d.id} '${d.contact_name}' ${d.invoice_ref} ${(d.amount_cents / 100).toFixed(2)} (${d.payment_method})`);
          });
        } finally { db.close(); }
      } catch (err) { fail(ctx, err); }
    });
  payables
    .command('list')
    .description('list payables (default: unpaid)')
    .option('--status <status>', 'unpaid|in_batch|paid (default unpaid)')
    .option('--method <method>', 'transfer|direct_debit')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const rows = listPayables(db, {
            status: opts.status ?? 'unpaid', method: opts.method === 'direct-debit' ? 'direct_debit' : (opts.method === 'transfer' ? 'transfer' : null),
          });
          output(ctx, { payables: rows }, (d) => {
            if (!d.payables.length) { console.log('no payables'); return; }
            table(d.payables.map((p) => ({ id: p.id, contact: p.contact_name, ref: p.invoice_ref, date: p.date, due: p.due_date ?? '-', amount: (p.amount_cents / 100).toFixed(2), method: p.payment_method, status: p.status })), [
              { key: 'id', label: 'id' }, { key: 'contact', label: 'contact' }, { key: 'ref', label: 'ref' },
              { key: 'date', label: 'date' }, { key: 'due', label: 'due' }, { key: 'amount', label: 'amount' },
              { key: 'method', label: 'method' }, { key: 'status', label: 'status' },
            ]);
          });
        } finally { db.close(); }
      } catch (err) { fail(ctx, err); }
    });
  payables
    .command('pay')
    .description('mark a payable as paid (after the bank statement confirms it)')
    .requiredOption('--id <id>', 'payable id')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const r = markPayablePaid(db, { id: Number(opts.id), actor: ctx.actor, dryRun: opts.dryRun });
          output(ctx, r, (d) => {
            if (d.dryRun) { console.log(`plan: mark payable ${d.payable_id} paid`); console.log('(dry run — nothing written)'); return; }
            console.log(`payable #${d.payable_id} marked paid`);
          });
        } finally { db.close(); }
      } catch (err) { fail(ctx, err); }
    });

  const batch = payments.command('batch').description('payment batches');
  batch
    .command('create')
    .description('create a payment batch from payables and/or explicit lines')
    .option('--lines <spec>', 'explicit lines: "CONTACT:AMOUNT[:REF];..." separated by ; (amounts keep comma-decimals, e.g. Vimexx:121,00)')
    .option('--csv <file>', 'batch CSV file: contact,amount,reference (or ;-delimited, Dutch amounts)')
    .option('--from-invoices', 'include all unpaid transfer payables (direct-debit excluded)')
    .option('--payable <ids>', 'only these payable ids (comma-separated)')
    .option('--date <date>', 'requested execution date YYYY-MM-DD (default today)')
    .option('--from-iban <iban>', 'debit account IBAN (default: company IBAN)')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          let lines = opts.lines ? parseLinesSpec(opts.lines) : [];
          const payableIds = opts.payable ? String(opts.payable).split(',').map((s) => Number(s.trim())).filter((n) => Number.isInteger(n)) : [];
          const fromInvoices = opts.fromInvoices || payableIds.length > 0;
          let res;
          if (opts.csv) {
            const csvText = readImportFile(opts.csv);
            res = createPaymentBatchFromCsv(db, { csvText, date: opts.date, debitIban: opts.fromIban, actor: ctx.actor, dryRun: opts.dryRun });
          } else if (fromInvoices) {
            const payables = listPayables(db, { status: 'unpaid', method: 'transfer' });
            const ids = payableIds.length ? payableIds : payables.map((p) => p.id);
            for (const id of ids) {
              if (!payables.some((x) => x.id === id)) {
                throw Object.assign(new Error(`payable ${id} is not unpaid+transfer (already batched or direct debit)`), { code: 'PAYABLE_NOT_ELIGIBLE' });
              }
            }
            res = createPaymentBatch(db, { date: opts.date, debitIban: opts.fromIban, lines, payableIds: ids, actor: ctx.actor, dryRun: opts.dryRun });
          } else {
            res = createPaymentBatch(db, { date: opts.date, debitIban: opts.fromIban, lines, payableIds: [], actor: ctx.actor, dryRun: opts.dryRun });
          }
          output(ctx, res, (d) => {
            if (d.dryRun) {
              console.log(`plan: payment batch ${d.batch_date} from ${d.debit_iban} — ${d.lines.length} line${d.lines.length === 1 ? '' : 's'}, total ${(d.total_cents / 100).toFixed(2)}`);
              for (const l of d.lines) console.log(`  ${l.name} ${l.iban} ${(l.amount_cents / 100).toFixed(2)}${l.reference ? ` — ${l.reference}` : ''}`);
              console.log('(dry run — nothing written)');
              return;
            }
            console.log(`batch #${d.id} — ${d.lines.length} lines, total ${(d.total_cents / 100).toFixed(2)}, status ${d.status}`);
          });
        } finally { db.close(); }
      } catch (err) { fail(ctx, err); }
    });
  batch
    .command('list')
    .description('list payment batches')
    .option('--status <status>', 'draft|exported|paid')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const rows = listPaymentBatches(db, { status: opts.status });
          output(ctx, { batches: rows }, (d) => {
            if (!d.batches.length) { console.log('no batches'); return; }
            table(d.batches.map((b) => ({ id: b.id, date: b.batch_date, debit: b.debit_iban, lines: b.lines.length, total: (b.total_cents / 100).toFixed(2), status: b.status, msg_id: b.msg_id ?? '-' })), [
              { key: 'id', label: 'id' }, { key: 'date', label: 'date' }, { key: 'debit', label: 'debit' },
              { key: 'lines', label: 'lines' }, { key: 'total', label: 'total' }, { key: 'status', label: 'status' },
              { key: 'msg_id', label: 'msg_id' },
            ]);
          });
        } finally { db.close(); }
      } catch (err) { fail(ctx, err); }
    });
  batch
    .command('show')
    .description('show a payment batch with its lines')
    .requiredOption('--id <id>', 'batch id')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const b = getPaymentBatch(db, Number(opts.id));
          if (!b) throw Object.assign(new Error(`batch ${opts.id} does not exist`), { code: 'BATCH_NOT_FOUND' });
          output(ctx, { batch: b }, (d) => {
            const x = d.batch;
            console.log(`batch #${x.id} — ${x.batch_date}, status ${x.status}, debit ${x.debit_iban}`);
            table(x.lines.map((l, i) => ({ line: i + 1, name: l.name, iban: l.iban, amount: (l.amount_cents / 100).toFixed(2), reference: l.reference ?? '-' })), [
              { key: 'line', label: 'line' }, { key: 'name', label: 'name' }, { key: 'iban', label: 'iban' },
              { key: 'amount', label: 'amount' }, { key: 'reference', label: 'reference' },
            ]);
          });
        } finally { db.close(); }
      } catch (err) { fail(ctx, err); }
    });
  batch
    .command('delete')
    .description('delete a DRAFT batch (releases payables back to unpaid)')
    .requiredOption('--id <id>', 'batch id')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const r = deletePaymentBatch(db, { id: Number(opts.id), actor: ctx.actor, dryRun: opts.dryRun });
          output(ctx, r, (d) => {
            if (d.dryRun) { console.log(`plan: delete draft batch ${d.batch_id}`); console.log('(dry run — nothing written)'); return; }
            console.log(`batch #${d.batch_id} deleted`);
          });
        } finally { db.close(); }
      } catch (err) { fail(ctx, err); }
    });
  batch
    .command('export')
    .description('export a batch as SEPA pain.001 XML for bank-portal upload (once per batch)')
    .requiredOption('--id <id>', 'batch id')
    .option('--schema <schema>', "schema version: '001.03' (default) or '001.09'", '001.03')
    .option('--out <file>', 'write the XML to this file')
    .option('--dry-run', 'show the plan without writing')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const r = exportPaymentBatch(db, { id: Number(opts.id), schema: opts.schema, actor: ctx.actor, dryRun: opts.dryRun });
          if (r.xml && opts.out) {
            mkdirSync(dirname(opts.out) || '.', { recursive: true });
            writeFileSync(opts.out, r.xml);
          }
          output(ctx, r, (d) => {
            if (d.dryRun) {
              console.log(`plan: export batch ${d.batch_id} as pain.${d.schema} — ${d.lines} lines, total ${(d.total_cents / 100).toFixed(2)}, MsgId ${d.msg_id}`);
              console.log('(dry run — nothing written)');
              return;
            }
            console.log(`batch #${d.batch_id} exported — ${d.lines} lines, MsgId ${d.msg_id}, sha256 ${d.file_hash.slice(0, 12)}…`);
            if (opts.out) console.log(`written to ${opts.out}`);
          });
        } finally { db.close(); }
      } catch (err) { fail(ctx, err); }
    });
}
