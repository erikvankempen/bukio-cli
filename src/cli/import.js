/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio import — opening balances, journal CSV, XML Auditfile (XAF) 4.0,
// and inbound e-invoices (EN 16931/Peppol UBL → payables).
// All importers validate the ENTIRE file before writing anything; a file with
// any problem is rejected with IMPORT_VALIDATION_FAILED + per-line details.
import { formatAmount } from '../core/money.js';
import {
  importJournalCsv, importOpeningBalances, importXaf, importContacts, readImportFile,
} from '../import/index.js';
import { importUblInvoice } from '../import/ubl-invoice.js';
import { ensureDb, makeCtx, output, fail } from './util.js';

export function make(program) {
  const imp = program.command('import').description('import data: opening balances, journal CSV, XAF 4.0 audit file');

  imp
    .command('opening-balances')
    .description('import opening balances from CSV (code,amount | code,debet,credit) into one posted Beginbalans entry')
    .requiredOption('--file <path>', 'CSV file')
    .option('--date <yyyy-mm-dd>', 'entry date (default: today)')
    .option('--dry-run', 'validate the whole file and show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const csvText = readImportFile(opts.file);
          const result = importOpeningBalances(db, {
            csvText, date: opts.date ?? null, actor: ctx.actor, dryRun: ctx.dryRun,
          });
          if (ctx.dryRun) {
            output(ctx, result, (d) => {
              console.log(`plan: import opening balances on ${d.date} — ${d.accounts} accounts`);
              console.log(`  debet ${formatAmount(d.total_debit_cents)}  =  credit ${formatAmount(d.total_credit_cents)}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          output(ctx, result, (d) => {
            console.log(`imported opening balances (${d.accounts} accounts) as entry #${d.entry.id} [${d.entry.state}] on ${d.entry.date}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  imp
    .command('journal')
    .description('import a journal from CSV (SnelStart/Exact-style: datum,boekstuknummer,rekening,tegenrekening,bedrag)')
    .requiredOption('--file <path>', 'CSV file')
    .option('--create-missing', 'create unknown accounts (type inferred from the net movement)')
    .option('--dry-run', 'validate the whole file and show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const csvText = readImportFile(opts.file);
          const result = importJournalCsv(db, {
            csvText, createMissing: Boolean(opts.createMissing), actor: ctx.actor, dryRun: ctx.dryRun,
          });
          if (ctx.dryRun) {
            output(ctx, result, (d) => {
              console.log(`plan: import journal — ${d.boekstukken} boekstukken / ${d.lines} lines${d.create_missing ? ' (create missing accounts)' : ''}`);
              for (const e of d.entries.slice(0, 10)) {
                console.log(`  ${e.date}  ${e.boekstuk}  (${e.lines} line${e.lines === 1 ? '' : 's'})`);
              }
              if (d.duplicates) console.log(`  (${d.duplicates} already imported — will skip)`);
              if (d.ignored_btw_codes.length) console.log(`  note: VAT codes ${d.ignored_btw_codes.join(', ')} ignored (net amounts imported)`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          output(ctx, result, (d) => {
            console.log(`imported ${d.imported} boekstukken${d.duplicates ? ` (${d.duplicates} duplicates skipped)` : ''}`);
            if (d.accounts_created.length) {
              console.log(`created accounts: ${d.accounts_created.map((a) => `${a.code} (${a.type})`).join(', ')}`);
            }
            if (d.ignored_btw_codes.length) {
              console.log(`note: VAT codes ${d.ignored_btw_codes.join(', ')} ignored — verify the booked amounts`);
            }
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  imp
    .command('contacts')
    .description('import suppliers/customers from an audit file (XAF 4.0) as contacts')
    .requiredOption('--file <path>', 'XAF 4.0 XML file')
    .option('--dry-run', 'validate the whole file and show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const xmlText = readImportFile(opts.file);
          const result = importContacts(db, { xmlText, actor: ctx.actor, dryRun: ctx.dryRun });
          if (ctx.dryRun) {
            output(ctx, result, (d) => {
              console.log(`plan: import contacts — ${d.suppliers} suppliers / ${d.customers} customers`);
              console.log(`  ${d.contacts_to_create} contacts will be created`);
              if (d.duplicates) console.log(`  (${d.duplicates} already known — will skip)`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          output(ctx, result, (d) => {
            console.log(`imported contacts: ${d.imported}${d.duplicates ? ` (${d.duplicates} duplicates skipped)` : ''}`);
            for (const c of d.contacts) console.log(`  ${c.kind.padEnd(8)} ${c.name}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  imp
    .command('xaf')
    .description('import an XML Auditfile Financieel 4.0 (Belastingdienst audit format)')
    .requiredOption('--file <path>', 'XAF 4.0 XML file')
    .option('--dry-run', 'validate the whole file and show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const xmlText = readImportFile(opts.file);
          const result = importXaf(db, { xmlText, actor: ctx.actor, dryRun: ctx.dryRun });
          if (ctx.dryRun) {
            output(ctx, result, (d) => {
              console.log(`plan: import XAF 4.0 — ${d.company.name ?? 'unknown company'} (${d.company.registration_id ?? 'no kvk'}) ${d.company.fiscal_year ?? ''}`);
              console.log(`  ${d.rekeningen} accounts / ${d.mutaties} mutations`);
              if (d.accounts_to_create) console.log(`  ${d.accounts_to_create} accounts will be created`);
              if (d.accounts_to_rename?.length) {
                console.log(`  ${d.accounts_to_rename.length} accounts will be renamed to the file's chart:`);
                for (const r of d.accounts_to_rename) console.log(`    ${r.code} -> ${r.name}`);
              }
              if (d.duplicates) console.log(`  (${d.duplicates} already imported — will skip)`);
              if (d.ignored_btw_codes.length) console.log(`  note: VAT codes ${d.ignored_btw_codes.join(', ')} ignored (net amounts imported)`);
              for (const w of d.company_mismatch) console.log(`  warning: ${w}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          output(ctx, result, (d) => {
            console.log(`imported XAF: ${d.imported} mutations${d.duplicates ? ` (${d.duplicates} duplicates skipped)` : ''}`);
            if (d.accounts_created.length) {
              console.log(`created accounts: ${d.accounts_created.map((a) => `${a.code} (${a.type})`).join(', ')}`);
            }
            if (d.accounts_updated?.length) {
              console.log(`renamed accounts: ${d.accounts_updated.map((a) => `${a.code} '${a.from}' -> '${a.to}'`).join(', ')}`);
            }
            if (d.accounts_rgs_backfilled?.length) {
              console.log(`RGS backfilled: ${d.accounts_rgs_backfilled.map((a) => `${a.code} ${a.name} -> ${a.taxonomy_code}`).join(', ')}`);
            }
            for (const w of d.chart_warnings ?? []) console.log(`  warning: ${w}`);
            if (d.ignored_btw_codes.length) {
              console.log(`note: VAT codes ${d.ignored_btw_codes.join(', ')} ignored — verify the booked amounts`);
            }
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err); // fail() already prints per-line details in human mode
      }
    });

  imp
    .command('invoice')
    .description('import an inbound e-invoice (EN 16931 / Peppol BIS 3.0 UBL) into the payables register')
    .requiredOption('--file <path>', 'UBL invoice XML file')
    .option('--contact <id>', 'explicit contact id (otherwise matched by tax id / name)')
    .option('--create-missing', 'create the supplier contact from the file when no match exists')
    .option('--dry-run', 'validate the whole file and show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const xmlText = readImportFile(opts.file);
          const result = importUblInvoice(db, {
            xmlText, contact: opts.contact != null ? Number(opts.contact) : null,
            createMissing: Boolean(opts.createMissing), actor: ctx.actor, dryRun: ctx.dryRun,
          });
          if (ctx.dryRun) {
            output(ctx, result, (d) => {
              console.log(`plan: import invoice ${d.invoice_ref} from ${d.supplier} (${formatAmount(d.amount_cents)} incl. VAT)`);
              console.log(`  date ${d.date} — due ${d.due_date}`);
              if (Object.keys(d.vat_by_rate).length) {
                console.log(`  VAT: ${Object.entries(d.vat_by_rate).map(([r, c]) => `${r}% = ${formatAmount(c)}`).join(', ')}`);
              }
              console.log(`  contact: ${d.contact.name}${d.contact.created ? ' (will be created)' : ''}`);
              console.log('(dry run — nothing written — no journal entry is created; book it via the normal workflow)');
            });
            return;
          }
          output(ctx, result, (d) => {
            console.log(`imported invoice ${d.invoice_ref} from ${d.supplier} as payable (${formatAmount(d.amount_cents)} incl. VAT, due ${d.due_date})`);
            if (d.duplicates) console.log(`(${d.duplicates} duplicate skipped)`);
            if (d.contacts_created) console.log(`created contact #${d.contact.id} ${d.contact.name}`);
            console.log('note: no journal entry was created — book the invoice via the normal workflow');
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
