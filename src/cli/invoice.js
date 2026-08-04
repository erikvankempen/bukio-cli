// bukio invoice + contact — outgoing invoicing (Phase 3, FR3).
import { writeFileSync } from 'node:fs';
import { formatAmount } from '../core/money.js';
import {
  createContact, createInvoice, creditInvoice, finalizeInvoice, getInvoice,
  listContacts, listInvoices, markPaid, parseLineSpec,
} from '../invoice/index.js';
import { invoiceToPdf } from '../invoice/pdf.js';
import { invoiceToUbl } from '../invoice/ubl.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

function fmtLine(l) {
  return {
    line_no: l.line_no, description: l.description, quantity: l.quantity,
    unit_price_cents: l.unit_price_cents, unit_price: formatAmount(l.unit_price_cents),
    vat_code: l.vat_code, vat_rate_bp: l.vat_rate_bp,
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
    entry_id: i.entry_id, credit_for_invoice_id: i.credit_for_invoice_id,
    net_cents: i.net_cents, vat_cents: i.vat_cents, gross_cents: i.gross_cents,
    paid_cents: i.paid_cents, outstanding_cents: i.gross_cents - i.paid_cents,
    net: formatAmount(i.net_cents), vat: formatAmount(i.vat_cents),
    gross: formatAmount(i.gross_cents), paid: formatAmount(i.paid_cents),
    lines: i.lines.map(fmtLine),
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
    .option('--vat-id <id>', 'customer btw-id (required for btw verlegd)')
    .option('--kvk <kvk>', 'customer KVK number')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const c = createContact(db, {
            name: opts.name, address: opts.address ?? null, postalCode: opts.postalCode ?? null,
            city: opts.city ?? null, country: opts.country, email: opts.email ?? null,
            vatId: opts.vatId ?? null, kvk: opts.kvk ?? null, actor: ctx.actor,
          });
          output(ctx, { contact: c }, (d) => console.log(`contact #${d.contact.id} ${d.contact.name}`));
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
  contact
    .command('list')
    .description('list contacts')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const contacts = listContacts(db).map((c) => ({
            id: c.id, name: c.name, city: c.city, vat_id: c.vat_id, email: c.email,
          }));
          output(ctx, { contacts }, (d) => {
            table(d.contacts, [
              { key: 'id', label: '#' }, { key: 'name', label: 'name' },
              { key: 'city', label: 'city' }, { key: 'vat_id', label: 'btw-id' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  const invoice = program.command('invoice').description('outgoing invoices');

  invoice
    .command('create')
    .description('create a draft invoice (compliance-validated at finalize)')
    .requiredOption('--contact <id>', 'contact id')
    .requiredOption('--lines <spec>', 'line spec "[QTYx] DESC @ PRICE [@ VATCODE]", comma-separated or repeatable')
    .requiredOption('--date <yyyy-mm-dd>', 'invoice date')
    .option('--due-days <n>', 'payment term in days', '30')
    .option('--delivery-date <yyyy-mm-dd>', 'delivery/service date if different')
    .option('--description <text>', 'invoice description')
    .option('--reference <ref>', 'customer reference / PO number')
    .option('--notes <text>', 'free-text note (printed on the invoice)')
    .option('--dry-run', 'validate without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const lines = [opts.lines];
        if (ctx.dryRun) {
          const parsed = lines.flatMap((s) => s.split(',')).map(parseLineSpec);
          output(ctx, {
            action: 'create draft invoice',
            contact: Number(opts.contact), date: opts.date, due_days: Number(opts.dueDays),
            lines: parsed, dryRun: true,
          }, (d) => {
            console.log(`plan: draft invoice for contact #${d.contact} on ${d.date}`);
            for (const l of d.lines) console.log(`  ${l.qty}x ${l.description} @ ${formatAmount(l.priceCents)}${l.vatCode ? ` @${l.vatCode}` : ''}`);
            console.log('(dry run — nothing written)');
          });
          return;
        }
        const db = ensureDb(ctx);
        try {
          const inv = createInvoice(db, {
            contactId: Number(opts.contact), lines: [opts.lines], date: opts.date,
            dueDays: Number(opts.dueDays), deliveryDate: opts.deliveryDate ?? null,
            description: opts.description ?? null, reference: opts.reference ?? null,
            notes: opts.notes ?? null, actor: ctx.actor,
          });
          output(ctx, { invoice: fmtInvoice(inv), dryRun: false }, (d) => {
            console.log(`invoice #${d.invoice.id} [draft] ${d.invoice.date} — ${d.invoice.contact_name}: ${d.invoice.gross} (btw ${d.invoice.vat})`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  invoice
    .command('finalize')
    .description('assign the sequential number and book the entry (Debiteuren/Omzet/btw)')
    .requiredOption('--id <id>', 'invoice id')
    .option('--dry-run', 'show the number + postings without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const result = finalizeInvoice(db, { id: opts.id, actor: ctx.actor, dryRun: ctx.dryRun });
          if (ctx.dryRun) {
            output(ctx, { ...result, dryRun: true }, (d) => {
              console.log(`plan: finalize as ${d.invoice_number} — net ${formatAmount(d.net)} / btw ${formatAmount(d.vat)} / totaal ${formatAmount(d.gross)}`);
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
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  invoice
    .command('list')
    .description('list invoices')
    .option('--status <status>', 'draft|sent|paid|overdue')
    .option('--type <type>', 'sales|credit')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const invoices = listInvoices(db, { status: opts.status ?? null, type: opts.type ?? null }).map(fmtInvoice);
          output(ctx, { invoices }, (d) => {
            table(d.invoices, [
              { key: 'id', label: '#' },
              { key: 'invoice_number', label: 'nummer' },
              { key: 'invoice_type', label: 'type' },
              { key: 'date', label: 'datum' },
              { key: 'contact_name', label: 'klant' },
              { key: 'gross', label: 'totaal' },
              { key: 'status', label: 'status' },
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
        try {
          const inv = getInvoice(db, opts.id);
          if (!inv) throw Object.assign(new Error(`invoice ${opts.id} does not exist`), { code: 'NOT_FOUND' });
          output(ctx, { invoice: fmtInvoice(inv) }, (d) => {
            const i = d.invoice;
            console.log(`${i.invoice_number ?? 'concept'} [${i.status}] ${i.date} — ${i.contact_name}`);
            for (const l of i.lines) console.log(`  ${l.line_no}. ${l.description}  ${l.quantity}x ${l.unit_price}${l.vat_code ? ` @${l.vat_code}` : ''} = ${l.amount}`);
            console.log(`  net ${i.net} / btw ${i.vat} / totaal ${i.gross}`);
            for (const p of i.payments) console.log(`  betaald ${p.date}: ${p.amount} (${p.method})`);
            if (i.outstanding_cents > 0) console.log(`  openstaand: ${formatAmount(i.outstanding_cents)}`);
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
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const inv = getInvoice(db, opts.id);
          if (!inv) throw Object.assign(new Error(`invoice ${opts.id} does not exist`), { code: 'NOT_FOUND' });
          if (!inv.invoice_number) throw Object.assign(new Error('finalize the invoice first — a concept has no number yet'), { code: 'NOT_FINALIZED' });
          const outPath = opts.out ?? `${inv.invoice_number}.pdf`;
          const result = await invoiceToPdf(db, inv, { outPath });
          output(ctx, { path: result.path, bytes: result.bytes }, (d) => console.log(`wrote ${d.path} (${d.bytes} bytes)`));
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  invoice
    .command('ubl')
    .description('export UBL 2.1 / Peppol BIS 3.0 XML')
    .requiredOption('--id <id>', 'invoice id')
    .option('--out <path>', 'output path (default <number>.xml in cwd)')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const inv = getInvoice(db, opts.id);
          if (!inv) throw Object.assign(new Error(`invoice ${opts.id} does not exist`), { code: 'NOT_FOUND' });
          if (!inv.invoice_number) throw Object.assign(new Error('finalize the invoice first'), { code: 'NOT_FINALIZED' });
          const xml = invoiceToUbl(db, inv);
          const outPath = opts.out ?? `${inv.invoice_number}.xml`;
          writeFileSync(outPath, xml);
          output(ctx, { path: outPath, bytes: xml.length }, (d) => console.log(`wrote ${d.path} (${d.bytes} bytes)`));
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  invoice
    .command('credit')
    .description('create a credit note (draft) for a finalized sales invoice')
    .requiredOption('--id <id>', 'sales invoice id')
    .option('--date <yyyy-mm-dd>', 'credit note date (default today)')
    .option('--reason <text>', 'reason (printed as the description)')
    .option('--dry-run', 'validate without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          if (ctx.dryRun) {
            output(ctx, { action: 'create credit note', for_invoice: Number(opts.id), reason: opts.reason ?? null, dryRun: true }, (d) => {
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
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  invoice
    .command('pay')
    .description('record a payment (tracking; the posting comes from the bank flow)')
    .requiredOption('--id <id>', 'invoice id')
    .requiredOption('--date <yyyy-mm-dd>', 'payment date')
    .option('--amount <amount>', 'amount (default: full outstanding)')
    .option('--method <method>', 'bank|cash|other', 'bank')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const inv = getInvoice(db, opts.id);
          if (!inv) throw Object.assign(new Error(`invoice ${opts.id} does not exist`), { code: 'NOT_FOUND' });
          const amountCents = opts.amount
            ? Math.round(parseFloat(opts.amount) * 100)
            : inv.gross_cents - inv.paid_cents;
          const paid = markPaid(db, { id: opts.id, date: opts.date, amountCents, method: opts.method, actor: ctx.actor });
          output(ctx, { invoice: fmtInvoice(paid) }, (d) => {
            console.log(`payment recorded: ${d.invoice.invoice_number} now ${d.invoice.status} (paid ${d.invoice.paid}/${d.invoice.gross})`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
