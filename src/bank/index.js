// Bank module — accounts, idempotent import, matching, reconciliation.
import { createHash } from 'node:crypto';
import { bankError } from './camt.js';
import { getAccountByCode } from '../core/accounts.js';
import { createEntry, getEntry, postEntry } from '../core/entries.js';
import { record } from '../audit/index.js';

function txHash(iban, tx) {
  const raw = [iban, tx.date, tx.amount_cents, tx.counterparty ?? '', tx.description ?? ''].join('|');
  return createHash('sha256').update(raw).digest('hex');
}

export function normalizeIban(iban) {
  return String(iban ?? '').trim().toUpperCase().replace(/\s+/g, '');
}

export function validateIban(iban) {
  if (!/^[A-Z]{2}\d{2}[A-Z0-9]{10,30}$/.test(iban)) {
    throw bankError('INVALID_IBAN', `'${iban}' is not a valid IBAN`);
  }
}

export function getOrCreateBankAccount(db, { iban, name = null, accountCode = '1100' }) {
  const ibanNorm = normalizeIban(iban);
  validateIban(ibanNorm);
  if (getAccountByCode(db, accountCode) == null) {
    throw bankError('ACCOUNT_NOT_FOUND', `ledger account ${accountCode} does not exist`);
  }
  const existing = db.prepare('SELECT * FROM bank_accounts WHERE iban = ?').get(ibanNorm);
  if (existing) return existing;
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
  const clauses = [];
  const params = [];
  if (state) { clauses.push('bt.state = ?'); params.push(state); }
  if (iban) {
    clauses.push('ba.iban = ?');
    params.push(String(iban).toUpperCase().replace(/\s+/g, ''));
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

export function setTransactionState(db, { id, state, actor = 'human' }) {
  const txRow = getTransaction(db, id);
  if (!txRow) throw bankError('NOT_FOUND', `bank transaction ${id} does not exist`);
  db.prepare('UPDATE bank_transactions SET state = ? WHERE id = ?').run(state, id);
  record(db, { actor, action: `bank.${state}`, command: 'bank match', args: { id }, outcome: 'ok' });
  return getTransaction(db, id);
}

/** Link a transaction to an existing posted entry. */
export function linkTransaction(db, { txId, entryId, method = 'manual', confidence = null, actor = 'human' }) {
  const txRow = getTransaction(db, txId);
  if (!txRow) throw bankError('NOT_FOUND', `bank transaction ${txId} does not exist`);
  if (txRow.state !== 'unmatched') {
    throw bankError('ALREADY_MATCHED', `bank transaction ${txId} is already ${txRow.state}`);
  }
  const entry = getEntry(db, entryId);
  if (!entry) throw bankError('NOT_FOUND', `entry ${entryId} does not exist`);
  if (entry.state !== 'posted') throw bankError('NOT_POSTED', `entry ${entryId} must be posted before linking`);

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
}

/**
 * Auto-match unmatched transactions to posted entries on the linked ledger
 * account. Best candidate = smallest |date difference| within windowDays.
 * method: 'exact' (<= 2 days) or 'fuzzy' (<= windowDays).
 * dryRun returns the would-be matches without writing.
 */
export function autoMatch(db, { windowDays = 5, actor = 'human', dryRun = false } = {}) {
  const unmatched = db.prepare(`
    SELECT bt.*, ba.account_code, ba.iban
    FROM bank_transactions bt JOIN bank_accounts ba ON ba.id = bt.bank_account_id
    WHERE bt.state = 'unmatched'
    ORDER BY bt.date, bt.id
  `).all();

  const matches = [];
  for (const txRow of unmatched) {
    const candidates = db.prepare(`
      SELECT e.id, e.date,
        ABS(julianday(e.date) - julianday(?)) AS day_diff
      FROM postings p
      JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
      JOIN accounts a ON a.id = p.account_id
      WHERE a.code = ? AND p.amount_cents = ?
        AND e.id NOT IN (SELECT target_id FROM reconciliations WHERE target_type = 'entry')
      ORDER BY day_diff
      LIMIT 1
    `).all(txRow.date, txRow.account_code, txRow.amount_cents);

    const best = candidates[0];
    if (best && best.day_diff <= windowDays) {
      matches.push({
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
    }
  }

  if (!dryRun) {
    const tx = db.transaction(() => {
      for (const m of matches) {
        db.prepare(`
          INSERT INTO reconciliations (bank_tx_id, target_type, target_id, method, confidence, created_by)
          VALUES (?, 'entry', ?, ?, ?, ?)
        `).run(m.tx_id, m.entry_id, m.method, m.confidence, actor);
        db.prepare("UPDATE bank_transactions SET state = 'matched' WHERE id = ?").run(m.tx_id);
      }
      record(db, {
        actor, action: 'bank.auto_match', command: 'bank match --auto',
        args: { windowDays, matched: matches.length }, outcome: 'ok',
        entryIds: matches.map((m) => m.entry_id),
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
