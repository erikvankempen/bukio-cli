/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Bank module — accounts, idempotent import, matching, reconciliation.
import { createHash } from 'node:crypto';
import { bankError } from './camt.js';
import { getAccountByCode } from '../core/accounts.js';
import { createEntry, getEntry, postEntry } from '../core/entries.js';
import { paymentFromBank, getInvoice, FX_MATCH_TOLERANCE_BP, FX_MATCH_FLOOR_CENTS } from '../invoice/index.js';
import { isValidIban as ibanMod97Valid } from '../core/iban.js';
import { record } from '../audit/index.js';

function txHash(iban, tx) {
  // bank_ref (CAMT AcctSvcrRef) is included so two genuinely identical
  // same-day payments (same amount/counterparty/description) each import —
  // without it the second is wrongly deduped as a duplicate.
  const raw = [iban, tx.date, tx.amount_cents, tx.counterparty ?? '', tx.description ?? '', tx.bank_ref ?? ''].join('|');
  return createHash('sha256').update(raw).digest('hex');
}

export function normalizeIban(iban) {
  // strip spaces AND dashes, matching the core normalizer — a dashed form
  // otherwise passes validation (core isValidIban normalizes internally) but
  // is stored dash-retaining, so a later import lookup misses and creates a
  // duplicate bank account for the same real IBAN
  return String(iban ?? '').trim().toUpperCase().replace(/[\s-]/g, '');
}

export function validateIban(iban) {
  // full ISO 13616 validity (format + mod-97), same as every other module —
  // a regex-only check accepted typo'd IBANs that could never match a real
  // CAMT statement, silently creating a second bank account on re-import
  if (!ibanMod97Valid(iban)) {
    throw bankError('INVALID_IBAN', `'${iban}' is not a valid IBAN`);
  }
}

export function getOrCreateBankAccount(db, { iban, name = null, accountCode = '1100', dryRun = false }) {
  const ibanNorm = normalizeIban(iban);
  validateIban(ibanNorm);
  if (getAccountByCode(db, accountCode) == null) {
    throw bankError('ACCOUNT_NOT_FOUND', `ledger account ${accountCode} does not exist`);
  }
  const existing = db.prepare('SELECT * FROM bank_accounts WHERE iban = ?').get(ibanNorm);
  if (existing) return existing;
  if (dryRun) {
    return { action: 'bank.add', iban: ibanNorm, name, account_code: accountCode, would_create: true, dryRun: true };
  }
  const info = db.prepare(
    'INSERT INTO bank_accounts (iban, name, account_code) VALUES (?, ?, ?)',
  ).run(ibanNorm, name, accountCode);
  return db.prepare('SELECT * FROM bank_accounts WHERE id = ?').get(info.lastInsertRowid);
}

export function listBankAccounts(db) {
  return db.prepare(`
    SELECT ba.*,
      COUNT(bt.id) AS transaction_count,
      COALESCE(SUM(CASE WHEN bt.state = 'unmatched' THEN 1 ELSE 0 END), 0) AS unmatched_count,
      COALESCE(SUM(bt.amount_cents), 0) AS balance_cents
    FROM bank_accounts ba
    LEFT JOIN bank_transactions bt ON bt.bank_account_id = ba.id
    GROUP BY ba.id
    ORDER BY ba.iban
  `).all();
}

/**
 * Preview an import without writing anything (dry-run): counts new vs
 * duplicate hashes against the current database.
 */
export function previewImport(db, { iban, transactions }) {
  const ibanNorm = normalizeIban(iban);
  validateIban(ibanNorm);
  let imported = 0;
  let duplicates = 0;
  const hasHash = db.prepare('SELECT 1 FROM bank_transactions WHERE hash = ?');
  for (const t of transactions) {
    if (hasHash.get(txHash(ibanNorm, t))) duplicates += 1;
    else imported += 1;
  }
  return { iban: ibanNorm, imported, duplicates, total: transactions.length };
}

/**
 * Import transactions: inserts only NEW hashes (idempotent).
 * Returns { imported, duplicates, total }.
 */
export function importTransactions(db, { iban, transactions, name = null, accountCode = '1100', actor = 'human' }) {
  if (!Array.isArray(transactions) || transactions.length === 0) {
    throw bankError('EMPTY_STATEMENT', 'no transactions to import');
  }
  // every transaction must carry a real calendar date — a garbage date would
  // be stored silently, then fail at post time or vanish from every report
  for (const t of transactions) {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(t.date)) {
      throw bankError('INVALID_DATE', `transaction date '${t.date}' must be YYYY-MM-DD`);
    }
    const d = new Date(`${t.date}T00:00:00Z`);
    if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== t.date) {
      throw bankError('INVALID_DATE', `transaction date '${t.date}' is not a valid calendar date`);
    }
  }
  const account = getOrCreateBankAccount(db, { iban, name, accountCode });
  const insertTx = db.prepare(`
    INSERT INTO bank_transactions (bank_account_id, date, amount_cents, counterparty, description, iban_counter, hash)
    VALUES (?, ?, ?, ?, ?, ?, ?)
  `);
  const hasHash = db.prepare('SELECT 1 FROM bank_transactions WHERE hash = ?');

  let imported = 0;
  let duplicates = 0;
  const tx = db.transaction(() => {
    for (const t of transactions) {
      const hash = txHash(account.iban, t);
      if (hasHash.get(hash)) { duplicates += 1; continue; }
      insertTx.run(account.id, t.date, t.amount_cents, t.counterparty, t.description, t.iban_counter, hash);
      imported += 1;
    }
    record(db, {
      actor, action: 'bank.import', command: 'bank import',
      args: { iban: account.iban, transactions: transactions.length, imported, duplicates },
      outcome: 'ok',
    });
  });
  tx();

  return { iban: account.iban, imported, duplicates, total: transactions.length };
}

export function listTransactions(db, { state = null, iban = null, limit = 200 } = {}) {
  if (!Number.isInteger(limit) || limit < 0) {
    throw bankError('INVALID_LIMIT', `limit must be a non-negative integer, got '${limit}'`);
  }
  const clauses = [];
  const params = [];
  if (state) { clauses.push('bt.state = ?'); params.push(state); }
  if (iban) {
    clauses.push('ba.iban = ?');
    params.push(normalizeIban(iban));
  }
  const where = clauses.length ? `WHERE ${clauses.join(' AND ')}` : '';
  return db.prepare(`
    SELECT bt.*, ba.iban AS iban, ba.account_code
    FROM bank_transactions bt
    JOIN bank_accounts ba ON ba.id = bt.bank_account_id
    ${where}
    ORDER BY bt.date DESC, bt.id DESC
    LIMIT ?
  `).all(...params, limit);
}

export function getTransaction(db, id) {
  return db.prepare(`
    SELECT bt.*, ba.iban AS iban, ba.account_code
    FROM bank_transactions bt JOIN bank_accounts ba ON ba.id = bt.bank_account_id
    WHERE bt.id = ?
  `).get(id) ?? null;
}

const VALID_TX_STATES = new Set(['unmatched', 'matched', 'ignored']);

export function setTransactionState(db, { id, state, actor = 'human', dryRun = false }) {
  const txRow = getTransaction(db, id);
  if (!txRow) throw bankError('NOT_FOUND', `bank transaction ${id} does not exist`);
  if (!VALID_TX_STATES.has(state)) {
    throw bankError('INVALID_STATE', `transaction state '${state}' must be one of ${[...VALID_TX_STATES].join(', ')}`);
  }
  if (dryRun) {
    return { action: `bank.${state}`, id, from: txRow.state, to: state, dryRun: true };
  }
  db.prepare('UPDATE bank_transactions SET state = ? WHERE id = ?').run(state, id);
  record(db, { actor, action: `bank.${state}`, command: 'bank match', args: { id }, outcome: 'ok' });
  return getTransaction(db, id);
}

/** Link a transaction to an existing posted entry. */
export function linkTransaction(db, { txId, entryId, method = 'manual', confidence = null, actor = 'human', dryRun = false }) {
  const txRow = getTransaction(db, txId);
  if (!txRow) throw bankError('NOT_FOUND', `bank transaction ${txId} does not exist`);
  if (txRow.state !== 'unmatched') {
    throw bankError('ALREADY_MATCHED', `bank transaction ${txId} is already ${txRow.state}`);
  }
  const entry = getEntry(db, entryId);
  if (!entry) throw bankError('NOT_FOUND', `entry ${entryId} does not exist`);
  if (entry.state !== 'posted') throw bankError('NOT_POSTED', `entry ${entryId} must be posted before linking`);

  if (dryRun) {
    return {
      action: 'bank.link', tx_id: txId, entry_id: entryId, method, confidence,
      entry_date: entry.date, amount_cents: txRow.amount_cents, dryRun: true,
    };
  }

  return db.transaction(() => {
    db.prepare(`
      INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
      VALUES (?, 'entry', ?, ?, ?, ?)
    `).run(txId, entryId, method, confidence, actor);
    db.prepare("UPDATE bank_transactions SET state = 'matched' WHERE id = ?").run(txId);
    record(db, {
      actor, action: 'bank.link', command: 'bank match --link',
      args: { txId, entryId, method, confidence }, outcome: 'ok', entryIds: [entryId],
    });
    return getTransaction(db, txId);
  })();
}

/** Post a new entry from an unmatched transaction (bank leg + counter leg). */
export function postFromTransaction(db, { txId, accountCode, actor = 'human', post = true }) {
  const txRow = getTransaction(db, txId);
  if (!txRow) throw bankError('NOT_FOUND', `bank transaction ${txId} does not exist`);
  if (txRow.state !== 'unmatched') {
    throw bankError('ALREADY_MATCHED', `bank transaction ${txId} is already ${txRow.state}`);
  }
  if (getAccountByCode(db, accountCode) == null) {
    throw bankError('ACCOUNT_NOT_FOUND', `account ${accountCode} does not exist`);
  }

  const description = txRow.description || txRow.counterparty || `Banktransactie ${txId}`;
  return db.transaction(() => {
    const entry = createEntry(db, {
      date: txRow.date,
      description,
      source: 'bank',
      sourceRef: `tx:${txId}`,
      actor,
      postings: [
        { code: txRow.account_code, amountCents: txRow.amount_cents },
        { code: accountCode, amountCents: -txRow.amount_cents },
      ],
    });
    const posted = post ? postEntry(db, { id: entry.id, actor }) : entry;
    const method = actor.startsWith('agent') ? 'agent' : 'manual';
    db.prepare(`
      INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
      VALUES (?, 'entry', ?, ?, 1.0, ?)
    `).run(txId, posted.id, method, actor);
    db.prepare("UPDATE bank_transactions SET state = 'matched' WHERE id = ?").run(txId);
    record(db, {
      actor, action: 'bank.post', command: 'bank match --post',
      args: { txId, accountCode }, outcome: 'ok', entryIds: [posted.id],
    });
    return { transaction: getTransaction(db, txId), entry: posted };
  })();
}

/**
 * Auto-match unmatched transactions to posted entries on the linked ledger
 * account. Best candidate = smallest |date difference| within windowDays.
 * method: 'exact' (<= 2 days) or 'fuzzy' (<= windowDays).
 * dryRun returns the would-be matches without writing.
 */
export function autoMatch(db, { windowDays = 5, actor = 'human', dryRun = false } = {}) {
  if (!Number.isInteger(windowDays) || windowDays < 0) {
    throw bankError('INVALID_WINDOW', `window-days must be a non-negative integer, got '${windowDays}'`);
  }
  const unmatched = db.prepare(`
    SELECT bt.*, ba.account_code, ba.iban
    FROM bank_transactions bt JOIN bank_accounts ba ON ba.id = bt.bank_account_id
    WHERE bt.state = 'unmatched'
    ORDER BY bt.date, bt.id
  `).all();

  const matches = [];
  // within-run consumption tracking: the candidate SQL excludes entries
  // already reconciled in PRIOR runs, but nothing stopped two same-amount
  // transactions in THIS run from matching the same posted entry — one entry
  // got two bank legs and the books stopped matching the statement. Track
  // claimed targets so each entry/invoice reconciles at most once per run.
  const usedEntryIds = new Set();
  const usedInvoiceIds = new Set();
  for (const txRow of unmatched) {
    // 1) exact/fuzzy match against already-booked entries on the bank account.
    // Bank-sourced entries (source='bank', source_ref 'tx:<id>') only match
    // transactions from the SAME bank account — two accounts sharing a ledger
    // code must not cross-match each other's postings. Manual entries (any
    // other source) are fair game on the shared ledger code.
    const used = [...usedEntryIds];
    const exclusion = used.length ? `AND e.id NOT IN (${used.map(() => '?').join(',')})` : '';
    const candidates = db.prepare(`
      SELECT e.id, e.date,
        ABS(julianday(e.date) - julianday(?)) AS day_diff
      FROM postings p
      JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
      JOIN accounts a ON a.id = p.account_id
      WHERE a.code = ? AND p.amount_cents = ?
        AND e.id NOT IN (SELECT target_id FROM reconciliations WHERE target_type = 'entry')
        ${exclusion}
        AND (
          e.source != 'bank'
          OR EXISTS (
            SELECT 1 FROM bank_transactions bt
            WHERE bt.id = CAST(SUBSTR(e.source_ref, 4) AS INTEGER)
              AND bt.bank_account_id = ?
          )
        )
      ORDER BY day_diff, e.id
      LIMIT 1
    `).all(txRow.date, txRow.account_code, txRow.amount_cents, ...used, txRow.bank_account_id);

    const best = candidates[0];
    if (best && best.day_diff <= windowDays) {
      matches.push({
        kind: 'entry',
        tx_id: txRow.id,
        tx_date: txRow.date,
        amount_cents: txRow.amount_cents,
        description: txRow.description,
        counterparty: txRow.counterparty,
        entry_id: best.id,
        entry_date: best.date,
        day_diff: best.day_diff,
        method: best.day_diff <= 2 ? 'exact' : 'fuzzy',
        confidence: best.day_diff <= 2 ? 0.99 : 0.8,
      });
      usedEntryIds.add(best.id);
      continue;
    }

    // 2) incoming money -> unpaid sales invoice whose outstanding matches the
    // amount within the FX sanity bound (FX_MATCH_TOLERANCE_BP, floor
    // FX_MATCH_FLOOR_CENTS): a foreign-currency invoice is EUR-translated at
    // invoice date, the payment arrives converted at payment date, so a small
    // difference is an FX move, not an error. Outstanding comes from the
    // DISCOUNTED totals (getInvoice derives net/vat/gross in one place) — a
    // line-sum query would ignore v0.13 discounts and never match. Must stay
    // in sync with the paymentFromBank tolerance or autoMatch matches what
    // posting rejects.
    if (txRow.amount_cents > 0) {
      const invoiceRows = db.prepare(`
        SELECT i.id, i.invoice_number, i.date, i.due_date, i.contact_id, c.name AS contact_name
        FROM invoices i
        LEFT JOIN contacts c ON c.id = i.contact_id
        WHERE i.invoice_type = 'sales' AND i.status IN ('sent','overdue')
        ORDER BY i.due_date IS NULL, i.due_date, i.id
      `).all();
      let best = null;
      for (const row of invoiceRows) {
        if (usedInvoiceIds.has(row.id)) continue; // already claimed this run
        const full = getInvoice(db, row.id);
        const outstanding = full.gross_cents - full.paid_cents;
        if (outstanding <= 0) continue;
        const tolerance = Math.max(Math.round(outstanding * FX_MATCH_TOLERANCE_BP / 10000), FX_MATCH_FLOOR_CENTS);
        const delta = Math.abs(outstanding - txRow.amount_cents);
        if (delta <= tolerance && (!best || delta < best.delta)) {
          best = { ...row, outstanding, delta };
        }
      }
      if (best) {
        matches.push({
          kind: 'invoice',
          tx_id: txRow.id,
          tx_date: txRow.date,
          amount_cents: txRow.amount_cents,
          description: txRow.description,
          counterparty: txRow.counterparty,
          invoice_id: best.id,
          invoice_number: best.invoice_number,
          contact_name: best.contact_name,
          fx_delta_cents: txRow.amount_cents - best.outstanding,
          method: 'invoice',
          confidence: 0.95,
        });
        usedInvoiceIds.add(best.id);
      }
    }
  }

  if (!dryRun) {
    const tx = db.transaction(() => {
      for (const m of matches) {
        if (m.kind === 'invoice') {
          // incoming payment: mark the invoice paid, post Bank/Debiteuren,
          // reconcile the transaction
          paymentFromBank(db, { invoiceId: m.invoice_id, bankTxId: m.tx_id, actor });
        } else {
          db.prepare(`
            INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
            VALUES (?, 'entry', ?, ?, ?, ?)
          `).run(m.tx_id, m.entry_id, m.method, m.confidence, actor);
          db.prepare("UPDATE bank_transactions SET state = 'matched' WHERE id = ?").run(m.tx_id);
        }
      }
      record(db, {
        actor, action: 'bank.auto_match', command: 'bank match --auto',
        args: { windowDays, matched: matches.length }, outcome: 'ok',
        entryIds: matches.map((m) => m.entry_id).filter(Boolean),
      });
    });
    tx();
  }

  return { matched: matches, unmatched_remaining: unmatched.length - matches.length };
}

/** Suggest a posting for each unmatched transaction (expense -> 4300, income -> 8000). */
export function suggestUnmatched(db) {
  const rows = db.prepare(`
    SELECT bt.*, ba.iban, ba.account_code
    FROM bank_transactions bt JOIN bank_accounts ba ON ba.id = bt.bank_account_id
    WHERE bt.state = 'unmatched'
    ORDER BY bt.date, bt.id
  `).all();
  return rows.map((t) => ({
    ...t,
    suggested_account: t.amount_cents > 0 ? '8000' : '4300',
  }));
}
