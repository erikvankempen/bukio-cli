// bukio bank — accounts, import (CAMT.053/CSV), matching, reconciliation.
import { readFileSync } from 'node:fs';
import { parseCamt053 } from '../bank/camt.js';
import { parseBankCsv } from '../bank/csv.js';
import {
  autoMatch, getOrCreateBankAccount, getTransaction, importTransactions,
  linkTransaction, listBankAccounts, listTransactions, postFromTransaction,
  previewImport, setTransactionState, suggestUnmatched,
} from '../bank/index.js';
import { formatAmount } from '../core/money.js';
import { ensureDb, makeCtx, output, fail, table } from './util.js';

function parseBankFile(filePath, format, iban) {
  const content = readFileSync(filePath, 'utf8');
  const trimmed = content.trimStart();
  const detected = trimmed.startsWith('<') ? 'camt' : 'csv';
  const fmt = format && format !== 'auto' ? format : detected;
  if (fmt === 'camt') {
    return parseCamt053(content);
  }
  if (fmt === 'csv') {
    return parseBankCsv(content, { defaultIban: iban });
  }
  throw Object.assign(new Error(`unknown format '${fmt}' (use camt|csv|auto)`), { code: 'INVALID_FORMAT' });
}

function fmtTx(t) {
  return {
    id: t.id, date: t.date, amount_cents: t.amount_cents, amount: formatAmount(t.amount_cents),
    counterparty: t.counterparty, description: t.description, iban_counter: t.iban_counter,
    iban: t.iban, account_code: t.account_code, state: t.state, hash: t.hash,
  };
}

export function make(program) {
  const bank = program.command('bank').description('bank accounts, import and matching');

  bank
    .command('add')
    .description('register a bank account')
    .requiredOption('--iban <iban>', 'IBAN')
    .option('--name <name>', 'account name')
    .option('--account-code <code>', 'linked ledger account', '1100')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const account = getOrCreateBankAccount(db, { iban: opts.iban, name: opts.name ?? null, accountCode: opts.accountCode, dryRun: ctx.dryRun });
          if (account.dryRun) {
            output(ctx, { plan: account }, (d) => {
              console.log(`plan: register bank account ${d.plan.iban} (${d.plan.name ?? '-'}) -> ledger ${d.plan.account_code}${d.plan.would_create ? '' : ' (already registered)'}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          output(ctx, { bank_account: { iban: account.iban, name: account.name, account_code: account.account_code, id: account.id } }, (d) => {
            console.log(`bank account ${d.bank_account.iban} (${d.bank_account.name ?? '-'}) -> ledger ${d.bank_account.account_code}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  bank
    .command('list')
    .description('list bank accounts with balances and state counts')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const accounts = listBankAccounts(db).map((a) => ({
            iban: a.iban, name: a.name, account_code: a.account_code,
            transaction_count: a.transaction_count, unmatched_count: a.unmatched_count,
            balance_cents: a.balance_cents, balance: formatAmount(a.balance_cents),
          }));
          output(ctx, { accounts }, (d) => {
            table(d.accounts, [
              { key: 'iban', label: 'iban' },
              { key: 'name', label: 'name' },
              { key: 'account_code', label: 'ledger' },
              { key: 'transaction_count', label: 'tx' },
              { key: 'unmatched_count', label: 'unmatched' },
              { key: 'balance', label: 'balance' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  bank
    .command('import')
    .description('import bank transactions from CAMT.053 XML or bank CSV (idempotent)')
    .requiredOption('--file <path>', 'CAMT.053 XML or CSV file')
    .requiredOption('--iban <iban>', 'IBAN of the bank account')
    .option('--format <format>', 'camt|csv|auto (auto detects XML vs CSV)', 'auto')
    .option('--name <name>', 'bank account name (if created)')
    .option('--account-code <code>', 'linked ledger account', '1100')
    .option('--dry-run', 'show what would be imported without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const transactions = parseBankFile(opts.file, opts.format, opts.iban);
        const db = ensureDb(ctx);
        try {
          if (ctx.dryRun) {
            const preview = previewImport(db, { iban: opts.iban, transactions });
            output(ctx, {
              action: 'import bank transactions', file: opts.file,
              transactions: transactions.slice(0, 10).map((t) => ({ ...t, amount: formatAmount(t.amount_cents) })),
              ...preview, dryRun: true,
            }, (d) => {
              console.log(`plan: import ${d.total} transactions to ${d.iban} — ${d.imported} new, ${d.duplicates} duplicate`);
              for (const t of d.transactions) console.log(`  ${t.date}  ${t.amount.padStart(12)}  ${t.counterparty ?? ''}  ${t.description ?? ''}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          const result = importTransactions(db, {
            iban: opts.iban, transactions, name: opts.name ?? null,
            accountCode: opts.accountCode, actor: ctx.actor,
          });
          // rows the CSV parser could not read (bad amount/date) — never
          // silently dropped; the user must fix the file and re-import
          if (transactions.skipped?.length) result.skipped = transactions.skipped;
          output(ctx, { ...result, dryRun: false }, (d) => {
            console.log(`imported ${d.imported} of ${d.total} transactions to ${d.iban} (${d.duplicates} duplicates skipped)`);
            for (const s of d.skipped ?? []) {
              console.log(`  ⚠ line ${s.line} skipped: ${s.reason} — fix the file and re-import`);
            }
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  bank
    .command('transactions')
    .description('list bank transactions')
    .option('--iban <iban>', 'filter by account')
    .option('--state <state>', 'unmatched|matched|ignored')
    .option('--limit <n>', 'max rows', '200')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const rows = listTransactions(db, {
            state: opts.state || null,
            iban: opts.iban || null,
            limit: Number(opts.limit),
          });
          const data = { transactions: rows.map(fmtTx) };
          output(ctx, data, (d) => {
            table(d.transactions, [
              { key: 'id', label: '#' },
              { key: 'date', label: 'date' },
              { key: 'amount', label: 'amount' },
              { key: 'state', label: 'state' },
              { key: 'counterparty', label: 'counterparty' },
              { key: 'description', label: 'description' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  const match = bank.command('match').description('match transactions to entries');

  match
    .command('auto')
    .description('auto-match unmatched transactions to posted entries')
    .option('--window-days <n>', 'max |date difference| in days', '5')
    .option('--dry-run', 'show would-be matches without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const result = autoMatch(db, {
            windowDays: parseInt(opts.windowDays, 10) || 5,
            actor: ctx.actor,
            dryRun: ctx.dryRun,
          });
          const data = {
            matched: result.matched.map((m) => ({ ...m, amount: formatAmount(m.amount_cents) })),
            unmatched_remaining: result.unmatched_remaining,
            dryRun: ctx.dryRun,
          };
          output(ctx, data, (d) => {
            console.log(`auto-match: ${d.matched.length} matched, ${d.unmatched_remaining} unmatched remaining${d.dryRun ? ' (dry run)' : ''}`);
            for (const m of d.matched) {
              if (m.kind === 'invoice') {
                const fx = m.fx_delta_cents ? ` (fx ${formatAmount(m.fx_delta_cents)})` : '';
                console.log(`  tx #${m.tx_id} ${m.tx_date} ${m.amount.padStart(12)} -> invoice ${m.invoice_number} (${m.contact_name ?? ''})${fx}`);
              } else {
                console.log(`  tx #${m.tx_id} ${m.tx_date} ${m.amount.padStart(12)} -> entry #${m.entry_id} (${m.method}, ${m.day_diff}d)`);
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

  match
    .command('suggest')
    .description('list unmatched transactions with a proposed posting')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const rows = suggestUnmatched(db);
          const data = { suggestions: rows.map((t) => ({ ...fmtTx(t), suggested_account: t.suggested_account })) };
          output(ctx, data, (d) => {
            table(d.suggestions, [
              { key: 'id', label: '#' },
              { key: 'date', label: 'date' },
              { key: 'amount', label: 'amount' },
              { key: 'counterparty', label: 'counterparty' },
              { key: 'suggested_account', label: 'suggest' },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  match
    .command('link')
    .description('link a transaction to an existing posted entry')
    .requiredOption('--tx <id>', 'bank transaction id')
    .requiredOption('--entry <id>', 'entry id')
    .option('--method <method>', 'exact|fuzzy|rule|manual|agent', 'manual')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const txRow = linkTransaction(db, { txId: opts.tx, entryId: opts.entry, method: opts.method, actor: ctx.actor, dryRun: ctx.dryRun });
          if (txRow.dryRun) {
            output(ctx, { plan: txRow }, (d) => {
              console.log(`plan: link tx #${d.plan.tx_id} (${formatAmount(d.plan.amount_cents)} on ${d.plan.entry_date}) -> entry #${d.plan.entry_id}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          output(ctx, { transaction: fmtTx(txRow), entry_id: Number(opts.entry) }, (d) => {
            console.log(`linked tx #${d.transaction.id} -> entry #${d.entry_id}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  match
    .command('post')
    .description('post a new entry from an unmatched transaction (bank leg + counter leg)')
    .requiredOption('--tx <id>', 'bank transaction id')
    .requiredOption('--account <code>', 'counter account for the posting')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const txRow = getTransaction(db, opts.tx);
          if (!txRow) throw Object.assign(new Error(`bank transaction ${opts.tx} does not exist`), { code: 'NOT_FOUND' });
          if (ctx.dryRun) {
            output(ctx, {
              action: 'post entry from bank transaction',
              tx: fmtTx(txRow),
              postings: [
                { code: txRow.account_code, amount_cents: txRow.amount_cents, amount: formatAmount(txRow.amount_cents) },
                { code: opts.account, amount_cents: -txRow.amount_cents, amount: formatAmount(-txRow.amount_cents) },
              ],
              dryRun: true,
            }, (d) => {
              console.log(`plan: post entry from tx #${d.tx.id} (${d.tx.date})`);
              for (const p of d.postings) console.log(`  ${p.code}  ${p.amount}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          const { entry } = postFromTransaction(db, { txId: opts.tx, accountCode: opts.account, actor: ctx.actor });
          output(ctx, { entry_id: entry.id, state: entry.state }, (d) => {
            console.log(`posted entry #${d.entry_id} from tx #${opts.tx} (${d.state})`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  bank
    .command('ignore')
    .description('mark a transaction as ignored (e.g. transfer between own accounts)')
    .requiredOption('--tx <id>', 'bank transaction id')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const txRow = setTransactionState(db, { id: opts.tx, state: 'ignored', actor: ctx.actor, dryRun: ctx.dryRun });
          if (txRow.dryRun) {
            output(ctx, { plan: txRow }, (d) => {
              console.log(`plan: ignore tx #${d.plan.id} (${d.plan.from} -> ${d.plan.to})`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          output(ctx, { transaction: fmtTx(txRow) }, (d) => console.log(`ignored tx #${d.transaction.id}`));
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  bank
    .command('unignore')
    .description('re-open an ignored transaction')
    .requiredOption('--tx <id>', 'bank transaction id')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const txRow = setTransactionState(db, { id: opts.tx, state: 'unmatched', actor: ctx.actor, dryRun: ctx.dryRun });
          if (txRow.dryRun) {
            output(ctx, { plan: txRow }, (d) => {
              console.log(`plan: re-open tx #${d.plan.id} (${d.plan.from} -> ${d.plan.to})`);
              console.log('(dry run — nothing written)');
            });
            return;
          }
          output(ctx, { transaction: fmtTx(txRow) }, (d) => console.log(`re-opened tx #${d.transaction.id}`));
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
