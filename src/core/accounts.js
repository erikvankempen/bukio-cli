// Accounts — chart of accounts CRUD, CSV import, default chart seeding.
import { readFileSync } from 'node:fs';
import { DEFAULT_CHART } from './chart.js';

const VALID_TYPES = ['asset', 'liability', 'equity', 'income', 'expense'];
const VALID_NORMAL = ['debit', 'credit'];
const RGS_RE = /^[A-Z]{2,5}\.\d{2}(\.\d{1,3})*$/;

export function accountError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

export function createAccount(db, { code, name, type, normalBalance, rgsCode = null }) {
  validateAccount({ code, name, type, normalBalance, rgsCode });
  try {
    const info = db.prepare(
      'INSERT INTO accounts (code, name, type, rgs_code, normal_balance) VALUES (?, ?, ?, ?, ?)',
    ).run(code, name.trim(), type, rgsCode, normalBalance);
    return getAccount(db, info.lastInsertRowid);
  } catch (err) {
    if (String(err.message).includes('UNIQUE constraint failed: accounts.code')) {
      throw accountError('ACCOUNT_EXISTS', `account code ${code} already exists`);
    }
    if (String(err.message).includes('CHECK constraint failed')) {
      throw accountError('INVALID_COMBINATION',
        `type '${type}' requires normal_balance '${type === 'asset' || type === 'expense' ? 'debit' : 'credit'}'`);
    }
    throw err;
  }
}

export function getAccount(db, id) {
  return db.prepare('SELECT * FROM accounts WHERE id = ?').get(id) ?? null;
}

export function getAccountByCode(db, code) {
  return db.prepare('SELECT * FROM accounts WHERE code = ?').get(code) ?? null;
}

export function listAccounts(db, { type = null, includeInactive = false } = {}) {
  const clauses = [];
  const params = [];
  if (type) {
    clauses.push('type = ?');
    params.push(type);
  }
  if (!includeInactive) {
    clauses.push('active = 1');
  }
  const where = clauses.length ? `WHERE ${clauses.join(' AND ')}` : '';
  return db.prepare(`SELECT * FROM accounts ${where} ORDER BY code`).all(...params);
}

/** Seed the default chart; skips codes that already exist. Returns count of new accounts. */
export function seedDefaultChart(db) {
  let created = 0;
  for (const a of DEFAULT_CHART) {
    if (getAccountByCode(db, a.code)) continue;
    createAccount(db, a);
    created += 1;
  }
  return created;
}

export function deactivateAccount(db, code) {
  const account = getAccountByCode(db, code);
  if (!account) throw accountError('ACCOUNT_NOT_FOUND', `account ${code} does not exist`);
  if (!account.active) throw accountError('ALREADY_INACTIVE', `account ${code} is already inactive`);
  db.prepare('UPDATE accounts SET active = 0 WHERE id = ?').run(account.id);
  return getAccount(db, account.id);
}

export function reactivateAccount(db, code) {
  const account = getAccountByCode(db, code);
  if (!account) throw accountError('ACCOUNT_NOT_FOUND', `account ${code} does not exist`);
  if (account.active) throw accountError('ALREADY_ACTIVE', `account ${code} is already active`);
  db.prepare('UPDATE accounts SET active = 1 WHERE id = ?').run(account.id);
  return getAccount(db, account.id);
}

/** Validate one account row object (shared by createAccount and CSV import). */
export function validateAccount({ code, name, type, normalBalance, rgsCode = null }) {
  if (!code || typeof code !== 'string' || !/^\d{1,6}$/.test(code)) {
    throw accountError('INVALID_CODE', `account code '${code}' must be 1-6 digits`);
  }
  if (!name || !name.trim()) throw accountError('INVALID_NAME', 'account name is required');
  if (!VALID_TYPES.includes(type)) {
    throw accountError('INVALID_TYPE', `account type '${type}' must be one of ${VALID_TYPES.join(', ')}`);
  }
  if (!VALID_NORMAL.includes(normalBalance)) {
    throw accountError('INVALID_NORMAL_BALANCE', `normal_balance '${normalBalance}' must be debit or credit`);
  }
  if (rgsCode != null && rgsCode !== '' && !RGS_RE.test(rgsCode)) {
    throw accountError('INVALID_RGS_CODE', `rgs_code '${rgsCode}' does not look like an RGS code (e.g. BMVA.02)`);
  }
}

/**
 * Import a chart from CSV. Columns (header row required):
 *   code,name,type,normal_balance[,rgs_code]
 * Valid rows are created; invalid rows are skipped with a reported error.
 * Returns { created, skipped, total, errors: [{line, error}] }.
 */
export function importChartCsv(db, csvText) {
  const lines = csvText.split(/\r?\n/).filter((l) => l.trim() !== '');
  if (lines.length < 2) {
    throw accountError('EMPTY_CSV', 'chart CSV must have a header row and at least one account');
  }
  const header = parseCsvLine(lines[0]).map((h) => h.trim());
  const expected = ['code', 'name', 'type', 'normal_balance'];
  for (const col of expected) {
    if (!header.includes(col)) {
      throw accountError('INVALID_CSV_HEADER', `chart CSV is missing column '${col}' (got: ${header.join(',')})`);
    }
  }
  const idx = Object.fromEntries(header.map((h, i) => [h, i]));

  const created = [];
  const errors = [];
  for (let i = 1; i < lines.length; i += 1) {
    const row = parseCsvLine(lines[i]).map((c) => c.trim());
    if (row.length === 1 && row[0] === '') continue;
    try {
      const account = {
        code: row[idx.code],
        name: row[idx.name],
        type: row[idx.type],
        normalBalance: row[idx.normal_balance],
        rgsCode: idx.rgs_code != null ? (row[idx.rgs_code] || null) : null,
      };
      validateAccount(account);
      if (getAccountByCode(db, account.code)) {
        throw accountError('ACCOUNT_EXISTS', `account ${account.code} already exists (skipped)`);
      }
      created.push(createAccount(db, account));
    } catch (err) {
      errors.push({ line: i + 1, error: `${err.code || 'ERROR'}: ${err.message}` });
    }
  }
  return { created: created.length, skipped: errors.length, total: lines.length - 1, errors };
}

function parseCsvLine(line) {
  const out = [];
  let cur = '';
  let inQuotes = false;
  for (const ch of line) {
    if (ch === '"') {
      inQuotes = !inQuotes;
    } else if (ch === ',' && !inQuotes) {
      out.push(cur);
      cur = '';
    } else {
      cur += ch;
    }
  }
  out.push(cur);
  return out;
}

export function readChartCsvFile(filePath) {
  return readFileSync(filePath, 'utf8');
}
