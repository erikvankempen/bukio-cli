// bukio invoice + contact — outgoing invoicing (Phase 3, FR3).
import { writeFileSync } from 'node:fs';
import { formatAmount, parseAmount } from '../core/money.js';
import {
  createContact, updateContact, createInvoice, creditInvoice, finalizeInvoice, getInvoice,
  invoiceReminders, listContacts, listInvoices, markPaid, parseLineSpec,
} from '../invoice/index.js';
import { invoiceToPdf } from '../invoice/pdf.js';
import { invoiceToUbl } from '../invoice/ubl.js';
import { sendPeppolInvoice } from '../invoice/peppol.js';
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
    .option('--iban <iban>', 'bank account (IBAN) — needed to include the contact in payment batches')
    .option('--dry-run', 'validate without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
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
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
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
    .option('--vat-id <id>', 'customer btw-id')
    .option('--kvk <kvk>', 'customer KVK number')
    .option('--iban <iban>', 'bank account (IBAN)')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
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
    .command('peppol-send')
    .description('send the invoice to a Peppol access-point provider (env: BUKIO_PEPPOL_ENDPOINT, BUKIO_PEPPOL_TOKEN)')
    .requiredOption('--id <id>', 'invoice id')
    .option('--endpoint <url>', 'provider endpoint (overrides BUKIO_PEPPOL_ENDPOINT)')
    .option('--dry-run', 'validate config + payload without sending')
    .action(async (opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const inv = getInvoice(db, opts.id);
          if (!inv) throw Object.assign(new Error(`invoice ${opts.id} does not exist`), { code: 'NOT_FOUND' });
          if (!inv.invoice_number) throw Object.assign(new Error('finalize the invoice first'), { code: 'NOT_FINALIZED' });
          const result = await sendPeppolInvoice(db, inv, { endpoint: opts.endpoint ?? null, dryRun: ctx.dryRun });
          output(ctx, result, (d) => {
            if (d.dryRun) {
              console.log(`plan: POST UBL for ${d.invoice_number} (${d.bytes} bytes) to ${d.endpoint}${d.configured ? ' (token set)' : ' (NO TOKEN — add BUKIO_PEPPOL_TOKEN)'}`);
              console.log('(dry run — nothing sent)');
            } else {
              console.log(`sent ${d.invoice_number} to ${d.endpoint} — HTTP ${d.status}${d.response ? `: ${d.response}` : ''}`);
            }
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
    .option('--dry-run', 'validate the payment without recording it')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const inv = getInvoice(db, opts.id);
          if (!inv) throw Object.assign(new Error(`invoice ${opts.id} does not exist`), { code: 'NOT_FOUND' });
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
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  invoice
    .command('reminders')
    .description('list overdue and due-soon invoices; --draft-emails adds a reminder email draft per invoice')
    .option('--within-days <n>', 'also list sent invoices due within this many days', '7')
    .option('--draft-emails', 'add a draft reminder email (subject/body) per invoice — nothing is sent')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const result = invoiceReminders(db, { withinDays: Number(opts.withinDays) });
          if (opts.draftEmails) {
            const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
            result.reminders = result.reminders.map((r) => {
              const to = r.contact_email;
              const subject = `Betalingsherinnering factuur ${r.invoice_number}`;
              const lines = [
                `Beste ${r.contact_name},`,
                '',
                `Voor factuur ${r.invoice_number} staat nog ${r.outstanding} open (vervaldatum ${r.due_date}).`,
                company?.iban ? `Wilt u dit bedrag overmaken naar IBAN ${company.iban} o.v.v. ${r.invoice_number}?` : `Wilt u het openstaande bedrag overmaken o.v.v. ${r.invoice_number}?`,
                '',
                'Met vriendelijke groet,',
                company?.name ?? ' ',
              ];
              return { ...r, draft_email: { to, subject, body: lines.join('\n') } };
            });
          }
          output(ctx, result, (d) => {
            if (!d.count) { console.log(`no reminders as of ${d.as_of}`); return; }
            table(d.reminders, [
              { key: 'invoice_number', label: 'factuur' },
              { key: 'contact_name', label: 'klant' },
              { key: 'due_date', label: 'vervaldatum' },
              { key: 'days_overdue', label: 'dagen' },
              { key: 'outstanding', label: 'openstaand' },
              { key: 'remind', label: 'herinnering' },
            ]);
            if (opts.draftEmails) {
              for (const r of d.reminders) {
                console.log(`\n--- ${r.invoice_number} -> ${r.draft_email.to ?? 'GEEN E-MAILADRES'} ---`);
                console.log(`subject: ${r.draft_email.subject}`);
                console.log(r.draft_email.body);
              }
            }
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
