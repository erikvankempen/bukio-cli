/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio invoice + contact — outgoing invoicing (Phase 3, FR3).
import { writeFileSync } from 'node:fs';
import { formatAmount, parseAmount } from '../core/money.js';
import {
  createContact, updateContact, createInvoice, creditInvoice, finalizeInvoice, getInvoice,
  formatQty, invoiceReminders, listContacts, listInvoices, markPaid, contactStatement,
} from '../invoice/index.js';
import { t, resolveLocale } from '../i18n/index.js';
import { invoiceToPdf } from '../invoice/pdf.js';
import { invoiceToUbl } from '../invoice/ubl.js';
import { sendPeppolInvoice } from '../invoice/peppol.js';
import { emailInvoice } from '../invoice/email.js';
import { ensureDb, makeCtx, output, fail, table, withDb, cliError } from './util.js';
import { emitReport } from './report.js';

function fmtLine(l) {
  return {
    line_no: l.line_no, description: l.description,
    quantity: formatQty(l.quantity), quantity_milli: l.quantity,
    unit: l.unit ?? null, item_id: l.item_id ?? null,
    unit_price_cents: l.unit_price_cents, unit_price: formatAmount(l.unit_price_cents),
    vat_code: l.vat_code, vat_rate_bp: l.vat_rate_bp,
    discount_type: l.discount_type, discount_value: l.discount_value,
    amount_cents: l.amount_cents, amount: formatAmount(l.amount_cents),
    vat_amount_cents: l.vat_amount_cents, vat_amount: formatAmount(l.vat_amount_cents),
  };
}

function fmtInvoice(i) {
  return {
    id: i.id, invoice_number: i.invoice_number, invoice_type: i.invoice_type,
    contact_id: i.contact_id, contact_name: i.contact?.name ?? null,
    date: i.date, due_date: i.due_date, delivery_date: i.delivery_date,
    status: i.status, reference: i.reference, notes: i.notes,
    language: i.language ?? 'nl',
    entry_id: i.entry_id, credit_for_invoice_id: i.credit_for_invoice_id,
    net_cents: i.net_cents, vat_cents: i.vat_cents, gross_cents: i.gross_cents,
    discount_type: i.discount_type, discount_value: i.discount_value,
    discount_cents: i.discount_cents,
    paid_cents: i.paid_cents, outstanding_cents: i.gross_cents - i.paid_cents,
    net: formatAmount(i.net_cents), vat: formatAmount(i.vat_cents),
    gross: formatAmount(i.gross_cents), paid: formatAmount(i.paid_cents),
    lines: i.lines.map(fmtLine),
    vat_breakdown: (i.vat_breakdown ?? []).map((b) => ({
      rate_bp: b.rate_bp, rate: b.rate_bp / 100,
      base: formatAmount(b.base_cents), vat: formatAmount(b.vat_cents),
    })),
    payments: i.payments.map((p) => ({ date: p.date, amount: formatAmount(p.amount_cents), method: p.method })),
  };
}

export function make(program) {
  const contact = program.command('contact').description('contacts (invoice counterparties)');
  contact
    .command('add')
    .description('add a contact')
    .requiredOption('--name <name>', 'contact name')
    .option('--address <address>', 'street address')
    .option('--postal-code <code>', 'postal code')
    .option('--city <city>', 'city')
    .option('--country <code>', 'country code', 'NL')
    .option('--email <email>', 'email')
    .option('--vat-id <id>', 'customer VAT id (required for reverse charge)')
    .option('--kvk <kvk>', 'customer KVK number')
    .option('--iban <iban>', 'bank account (IBAN) — needed to include the contact in payment batches')
    .option('--dry-run', 'validate without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const c = createContact(db, {
          name: opts.name, address: opts.address ?? null, postalCode: opts.postalCode ?? null,
          city: opts.city ?? null, country: opts.country, email: opts.email ?? null,
          vatId: opts.vatId ?? null, kvk: opts.kvk ?? null, iban: opts.iban ?? null,
          actor: ctx.actor, dryRun: ctx.dryRun,
        });
        output(ctx, { contact: c }, (d) => {
          if (d.contact.dryRun) {
            console.log(`plan: add contact '${d.contact.name}'${d.contact.iban ? ` (iban ${d.contact.iban})` : ''}`);
            console.log('(dry run — nothing written)');
            return;
          }
          console.log(`contact #${d.contact.id} ${d.contact.name}`);
        });
    }));
  contact
    .command('update')
    .description('update a contact (e.g. add the IBAN for payment batches)')
    .requiredOption('--id <id>', 'contact id')
    .option('--name <name>', 'contact name')
    .option('--address <address>', 'street address')
    .option('--postal-code <code>', 'postal code')
    .option('--city <city>', 'city')
    .option('--country <code>', 'country code')
    .option('--email <email>', 'email')
    .option('--vat-id <id>', 'customer tax id')
    .option('--kvk <kvk>', 'customer KVK number')
    .option('--iban <iban>', 'bank account (IBAN)')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const c = updateContact(db, {
          id: Number(opts.id), name: opts.name, address: opts.address, postalCode: opts.postalCode,
          city: opts.city, country: opts.country, email: opts.email, vatId: opts.vatId,
          kvk: opts.kvk, iban: opts.iban, actor: ctx.actor, dryRun: ctx.dryRun,
        });
        output(ctx, { contact: c }, (d) => {
          if (d.contact.dryRun) {
            console.log(`plan: update contact #${d.contact.id}`);
            for (const [k, v] of Object.entries(d.contact.changes)) {
              const snake = k.replace(/[A-Z]/g, (m) => `_${m.toLowerCase()}`);
              console.log(`  ${k}: '${d.contact.before[snake] ?? ''}' -> '${v}'`);
            }
            console.log('(dry run — nothing written)');
            return;
          }
          console.log(`contact #${d.contact.id} ${d.contact.name} updated (iban: ${d.contact.iban ?? 'none'})`);
        });
    }));
  contact
    .command('list')
    .description('list contacts')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const contacts = listContacts(db).map((c) => ({
          id: c.id, name: c.name, city: c.city, vat_id: c.vat_id, email: c.email,
        }));
        output(ctx, { contacts }, (d) => {
          table(d.contacts, [
            { key: 'id', label: '#' }, { key: 'name', label: 'name' },
            { key: 'city', label: 'city' }, { key: 'vat_id', label: 'tax id' },
          ]);
        });
    }));

  contact
    .command('statement')
    .description('statement: invoices + payments + payables with a running balance')
    .requiredOption('--id <id>', 'contact id')
    .option('--as-of <yyyy-mm-dd>', 'statement date (default: today)')
    .option('--format <format>', 'json|csv|xlsx|human')
    .option('--out <path>', 'output file (csv/xlsx)')
    .action((opts, command) => withDb(command, async (ctx, db) => {
        const data = contactStatement(db, { contactId: Number(opts.id), asOf: opts.asOf || null });
        const fmtRows = (d) => d.rows.map((r) => ({
          date: r.date, kind: r.kind, ref: r.ref, description: r.description,
          debit: formatAmount(r.debit_cents), credit: formatAmount(r.credit_cents),
          balance: formatAmount(r.balance_cents),
        }));
        const csvColumns = [
          { key: 'date', label: 'date' }, { key: 'kind', label: 'kind' },
          { key: 'ref', label: 'ref' }, { key: 'description', label: 'description' },
          { key: 'debit', label: 'debit' }, { key: 'credit', label: 'credit' },
          { key: 'balance', label: 'balance' },
        ];
        await emitReport(ctx, opts, data, {
          csvColumns,
          csvRows: fmtRows,
          sheets: (d) => [{
            name: 'Statement',
            columns: csvColumns.map((c) => ({ header: c.label, key: c.key })),
            rows: fmtRows(d),
          }],
          render: (d) => {
            console.log(`statement ${d.contact.name} (as of ${d.as_of})`);
            if (!d.rows.length) {
              console.log('(no transactions)');
              return;
            }
            table(fmtRows(d), [
              { key: 'date', label: 'date' }, { key: 'kind', label: 'kind' },
              { key: 'ref', label: 'ref' }, { key: 'description', label: 'description' },
              { key: 'debit', label: 'debit' }, { key: 'credit', label: 'credit' },
              { key: 'balance', label: 'balance' },
            ]);
            console.log(`balance: ${formatAmount(d.balance_cents)}`);
          },
        });
    }));

  const invoice = program.command('invoice').description('outgoing invoices');

  invoice
    .command('create')
    .description('create a draft invoice (compliance-validated at finalize)')
    .requiredOption('--contact <id>', 'contact id')
    .option('--lines <spec>', 'line spec "[QTYx] DESC @ PRICE [@ VATCODE] [@ -DISCOUNT]", comma-separated or repeatable', (v, acc) => [...acc, v], [])
    .option('--items <spec>', 'item spec "ID[:QTY][@PRICE][@VATCODE][@-DISCOUNT]", comma-separated or repeatable', (v, acc) => [...acc, v], [])
    .requiredOption('--date <yyyy-mm-dd>', 'invoice date')
    .option('--due-days <n>', 'payment term in days', '30')
    .option('--delivery-date <yyyy-mm-dd>', 'delivery/service date if different')
    .option('--description <text>', 'invoice description')
    .option('--reference <ref>', 'customer reference / PO number')
    .option('--notes <text>', 'free-text note (printed on the invoice)')
    .option('--discount-pct <pct>', 'discount on the total, percentage (e.g. 5)')
    .option('--discount-amount <amount>', 'discount on the total, fixed amount (e.g. 50.00)')
    .option('--language <lang>', 'invoice document language (en, nl, de, fr, da, fi, nb, sv, it, es, pt, nl-be, fr-lu; default: the company profile\'s language)')
    .option('--dry-run', 'validate without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        if (opts.discountPct !== undefined && opts.discountAmount !== undefined) {
          throw cliError('INVALID_DISCOUNT', 'pass either --discount-pct or --discount-amount, not both');
        }
        const discountType = opts.discountPct !== undefined ? 'pct'
          : opts.discountAmount !== undefined ? 'amount' : null;
        const discountValue = discountType === 'pct'
          ? Math.round(Number(opts.discountPct) * 100)
          : discountType === 'amount' ? parseAmount(opts.discountAmount) : null;
        // one engine call for both paths — createInvoice's dryRun contract
        // returns the same shape with dryRun: true
        const inv = createInvoice(db, {
          contactId: Number(opts.contact), lines: opts.lines?.length ? opts.lines : null,
          items: opts.items?.length ? opts.items : null, date: opts.date,
          dueDays: Number(opts.dueDays), deliveryDate: opts.deliveryDate ?? null,
          description: opts.description ?? null, reference: opts.reference ?? null,
          notes: opts.notes ?? null, discountType, discountValue,
          language: opts.language, actor: ctx.actor, dryRun: ctx.dryRun,
        });
        // dry-run plan keeps the raw engine shape; the real path wraps in
        // { invoice: fmtInvoice(...) } exactly as before (JSON contract)
        output(ctx, ctx.dryRun ? inv : { invoice: fmtInvoice(inv), dryRun: false }, (d) => {
          if (ctx.dryRun) {
            console.log(`plan: draft invoice for contact #${d.contact_id} on ${d.date} — net ${formatAmount(d.net_cents)}${d.vat_cents ? ` + ${formatAmount(d.vat_cents)} VAT` : ''} = ${formatAmount(d.gross_cents)}${d.discount_cents ? ` (discount ${formatAmount(d.discount_cents)})` : ''} [${d.language}]`);
            for (const l of d.lines) console.log(`  ${l.qty}x ${l.description} @ ${formatAmount(l.priceCents)}${l.vatCode ? ` @${l.vatCode}` : ''}${l.discountType ? ` @-${l.discountType === 'pct' ? `${l.discountValue / 100}%` : formatAmount(l.discountValue)}` : ''}`);
            console.log('(dry run — nothing written)');
            return;
          }
          const f = d.invoice;
          console.log(`invoice #${f.id} [draft] ${f.date} — ${f.contact_name}: ${f.gross} (VAT ${f.vat})${f.discount_cents ? ` (discount ${formatAmount(f.discount_cents)})` : ''} [${f.language}]`);
        });
    }));

  invoice
    .command('finalize')
    .description('assign the sequential number and book the entry (debtors/revenue/VAT)')
    .requiredOption('--id <id>', 'invoice id')
    .option('--dry-run', 'show the number + postings without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const result = finalizeInvoice(db, { id: opts.id, actor: ctx.actor, dryRun: ctx.dryRun });
        if (ctx.dryRun) {
          output(ctx, { ...result, dryRun: true }, (d) => {
            console.log(`plan: finalize as ${d.invoice_number} — net ${formatAmount(d.net)} / VAT ${formatAmount(d.vat)} / total ${formatAmount(d.gross)}`);
            for (const p of d.postings) console.log(`  ${p.code}  ${formatAmount(p.amountCents).padStart(12)}${p.vatCode ? ` @${p.vatCode}` : ''}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        output(ctx, {
          invoice: fmtInvoice(result.invoice), entry: { id: result.entry.id, state: result.entry.state }, dryRun: false,
        }, (d) => {
          console.log(`finalized ${d.invoice.invoice_number} — entry #${d.entry.id} (${d.entry.state})`);
        });
    }));

  invoice
    .command('list')
    .description('list invoices')
    .option('--status <status>', 'draft|sent|paid|overdue')
    .option('--type <type>', 'sales|credit')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        const locale = resolveLocale(ctx, db);
        try {
          const invoices = listInvoices(db, { status: opts.status ?? null, type: opts.type ?? null }).map(fmtInvoice);
          output(ctx, { invoices }, (d) => {
            table(d.invoices, [
              { key: 'id', label: '#' },
              { key: 'invoice_number', label: t('invlist.number', {}, locale) },
              { key: 'invoice_type', label: t('invlist.type', {}, locale) },
              { key: 'date', label: t('invlist.date', {}, locale) },
              { key: 'contact_name', label: t('invlist.customer', {}, locale) },
              { key: 'gross', label: t('invlist.total', {}, locale) },
              { key: 'status', label: t('invlist.status', {}, locale) },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  invoice
    .command('show')
    .description('show one invoice with lines and payments')
    .requiredOption('--id <id>', 'invoice id')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        const locale = resolveLocale(ctx, db);
        try {
          const inv = getInvoice(db, opts.id);
          if (!inv) throw cliError('NOT_FOUND', `invoice ${opts.id} does not exist`);
          output(ctx, { invoice: fmtInvoice(inv) }, (d) => {
            const i = d.invoice;
            console.log(`${i.invoice_number ?? t('status.draft', {}, locale)} [${i.status}] ${i.date} — ${i.contact_name}`);
            for (const l of i.lines) console.log(`  ${l.line_no}. ${l.description}  ${l.quantity}x ${l.unit_price}${l.vat_code ? ` @${l.vat_code}` : ''} = ${l.amount}`);
            console.log(`  net ${i.net} / vat ${i.vat} / total ${i.gross}`);
            for (const p of i.payments) console.log(`  ${t('entry.paid', { date: p.date, amount: p.amount, method: p.method }, locale)}`);
            if (i.outstanding_cents > 0) console.log(`  ${t('entry.outstanding', { amount: formatAmount(i.outstanding_cents) }, locale)}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  invoice
    .command('pdf')
    .description('render the invoice to PDF (headless Chromium)')
    .requiredOption('--id <id>', 'invoice id')
    .option('--out <path>', 'output path (default <number>.pdf in cwd)')
    .action((opts, command) => withDb(command, async (ctx, db) => {
        const inv = getInvoice(db, opts.id);
        if (!inv) throw cliError('NOT_FOUND', `invoice ${opts.id} does not exist`);
        if (!inv.invoice_number) throw cliError('NOT_FINALIZED', 'finalize the invoice first — a draft has no number yet');
        const outPath = opts.out ?? `${inv.invoice_number}.pdf`;
        const result = await invoiceToPdf(db, inv, { outPath });
        output(ctx, { path: result.path, bytes: result.bytes }, (d) => console.log(`wrote ${d.path} (${d.bytes} bytes)`));
    }));

  invoice
    .command('ubl')
    .description('export UBL 2.1 / Peppol BIS 3.0 XML')
    .requiredOption('--id <id>', 'invoice id')
    .option('--out <path>', 'output path (default <number>.xml in cwd)')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const inv = getInvoice(db, opts.id);
        if (!inv) throw cliError('NOT_FOUND', `invoice ${opts.id} does not exist`);
        if (!inv.invoice_number) throw cliError('NOT_FINALIZED', 'finalize the invoice first');
        const xml = invoiceToUbl(db, inv);
        const outPath = opts.out ?? `${inv.invoice_number}.xml`;
        writeFileSync(outPath, xml);
        output(ctx, { path: outPath, bytes: xml.length }, (d) => console.log(`wrote ${d.path} (${d.bytes} bytes)`));
    }));

  invoice
    .command('credit')
    .description('create a credit note (draft) for a finalized sales invoice')
    .requiredOption('--id <id>', 'sales invoice id')
    .option('--date <yyyy-mm-dd>', 'credit note date (default today)')
    .option('--reason <text>', 'reason (printed as the description)')
    .option('--dry-run', 'validate without writing')
    .action((opts, command) => withDb(command, (ctx, db) => {
        if (ctx.dryRun) {
          // same validation as the real run (creditInvoice dryRun): the old
          // branch echoed plans for nonexistent/unfinalized invoices as ok
          const plan = creditInvoice(db, {
            id: Number(opts.id), date: opts.date ?? null, reason: opts.reason ?? null,
            actor: ctx.actor, dryRun: true,
          });
          output(ctx, plan, (d) => {
            console.log(`plan: credit note for invoice #${d.for_invoice}${d.reason ? ` — "${d.reason}"` : ''}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        const credit = creditInvoice(db, { id: opts.id, date: opts.date ?? null, reason: opts.reason ?? null, actor: ctx.actor });
        output(ctx, { invoice: fmtInvoice(credit), dryRun: false }, (d) => {
          console.log(`credit note #${d.invoice.id} [draft] for ${d.invoice.reference} — ${d.invoice.gross}`);
          console.log('finalize it to assign the number and book the reversal');
        });
    }));

  invoice
    .command('peppol-send')
    .description('send the invoice to a Peppol access-point provider (env: BUKIO_PEPPOL_ENDPOINT, BUKIO_PEPPOL_TOKEN)')
    .requiredOption('--id <id>', 'invoice id')
    .option('--endpoint <url>', 'provider endpoint (overrides BUKIO_PEPPOL_ENDPOINT)')
    .option('--dry-run', 'validate config + payload without sending')
    .action((opts, command) => withDb(command, async (ctx, db) => {
        const inv = getInvoice(db, opts.id);
        if (!inv) throw cliError('NOT_FOUND', `invoice ${opts.id} does not exist`);
        if (!inv.invoice_number) throw cliError('NOT_FINALIZED', 'finalize the invoice first');
        const result = await sendPeppolInvoice(db, inv, { endpoint: opts.endpoint ?? null, dryRun: ctx.dryRun });
        output(ctx, result, (d) => {
          if (d.dryRun) {
            console.log(`plan: POST UBL for ${d.invoice_number} (${d.bytes} bytes) to ${d.endpoint}${d.configured ? ' (token set)' : ' (NO TOKEN — add BUKIO_PEPPOL_TOKEN)'}`);
            console.log('(dry run — nothing sent)');
          } else {
            console.log(`sent ${d.invoice_number} to ${d.endpoint} — HTTP ${d.status}${d.response ? `: ${d.response}` : ''}`);
          }
        });
    }));

  invoice
    .command('pay')
    .description('record a payment (tracking; the posting comes from the bank flow)')
    .requiredOption('--id <id>', 'invoice id')
    .requiredOption('--date <yyyy-mm-dd>', 'payment date')
    .option('--amount <amount>', 'amount (default: full outstanding)')
    .option('--method <method>', 'bank|cash|other', 'bank')
    .option('--dry-run', 'validate the payment without recording it')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const inv = getInvoice(db, opts.id);
        if (!inv) throw cliError('NOT_FOUND', `invoice ${opts.id} does not exist`);
        // money is integer cents — parseAmount rejects '12,34' (Dutch comma
        // would silently parse as 12.00 via parseFloat), '1e3' and garbage
        const amountCents = opts.amount
          ? parseAmount(opts.amount)
          : inv.gross_cents - inv.paid_cents;
        const paid = markPaid(db, { id: opts.id, date: opts.date, amountCents, method: opts.method, actor: ctx.actor, dryRun: ctx.dryRun });
        if (paid.dryRun) {
          output(ctx, { plan: paid }, (d) => {
            console.log(`plan: record payment of ${formatAmount(d.plan.amount_cents)} on invoice #${d.plan.invoice_id} (${formatAmount(d.plan.remaining_cents)} outstanding after)`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        output(ctx, { invoice: fmtInvoice(paid) }, (d) => {
          console.log(`payment recorded: ${d.invoice.invoice_number} now ${d.invoice.status} (paid ${d.invoice.paid}/${d.invoice.gross})`);
        });
    }));

  invoice
    .command('email')
    .description('email a finalized invoice (PDF attached) — SMTP config via BUKIO_SMTP_* env')
    .requiredOption('--id <id>', 'invoice id')
    .option('--to <email>', 'recipient (default: the contact email)')
    .option('--subject <text>', 'subject (default: "Invoice <nr> — <company>")')
    .option('--body <text>', 'plain-text body (default: a short Dutch/English intro)')
    .option('--no-pdf', "don't attach the PDF")
    .option('--dry-run', 'render and validate without sending')
    .action((opts, command) => withDb(command, async (ctx, db) => {
        const result = await emailInvoice(db, {
          id: Number(opts.id), to: opts.to ?? null, subject: opts.subject ?? null,
          body: opts.body ?? null, attachPdf: opts.pdf !== false,
          actor: ctx.actor, dryRun: ctx.dryRun,
        });
        output(ctx, result, (d) => {
          if (d.dryRun) {
            console.log(`plan: email invoice ${d.invoice_number} to ${d.to}`);
            console.log(`  subject: ${d.subject}`);
            if (d.attachment) console.log(`  attachment: ${d.attachment.filename} (${d.attachment.bytes} bytes)`);
            console.log('(dry run — nothing sent)');
            return;
          }
          console.log(`sent invoice ${d.invoice_number} to ${d.to} via ${d.server}`);
        });
    }));

  invoice
    .command('reminders')
    .description('list overdue and due-soon invoices; --draft-emails adds a reminder email draft per invoice')
    .option('--within-days <n>', 'also list sent invoices due within this many days', '7')
    .option('--draft-emails', 'add a draft reminder email (subject/body) per invoice — nothing is sent')
    .action((opts, command) => withDb(command, (ctx, db) => {
        const result = invoiceReminders(db, { withinDays: Number(opts.withinDays) });
        const locale = resolveLocale(ctx, db);
        if (opts.draftEmails) {
          const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
          result.reminders = result.reminders.map((r) => {
            const to = r.contact_email;
            const subject = t('email.reminderSubject', { number: r.invoice_number }, locale);
            const transfer = company?.iban
              ? t('email.reminderTransferIban', { iban: company.iban, number: r.invoice_number }, locale)
              : t('email.reminderTransferPlain', { number: r.invoice_number }, locale);
            const lines = t('email.reminderBody', {
              name: r.contact_name, number: r.invoice_number,
              outstanding: r.outstanding, dueDate: r.due_date, transfer, company: company?.name ?? ' ',
            }, locale).split('\n');
            return { ...r, draft_email: { to, subject, body: lines.join('\n') } };
          });
        }
        output(ctx, result, (d) => {
          if (!d.count) { console.log(t('reminder.none', { date: d.as_of }, locale)); return; }
          table(d.reminders, [
            { key: 'invoice_number', label: t('invlist.number', {}, locale) },
            { key: 'contact_name', label: t('invlist.customer', {}, locale) },
            { key: 'due_date', label: t('invlist.dueDate', {}, locale) },
            { key: 'days_overdue', label: t('invlist.days', {}, locale) },
            { key: 'outstanding', label: t('invlist.outstanding', {}, locale) },
            { key: 'remind', label: t('invlist.reminder', {}, locale) },
          ]);
          if (opts.draftEmails) {
            for (const r of d.reminders) {
              console.log(`\n--- ${r.invoice_number} -> ${r.draft_email.to ?? 'NO EMAIL ADDRESS'} ---`);
              console.log(`subject: ${r.draft_email.subject}`);
              console.log(r.draft_email.body);
            }
          }
        });
    }));
}
