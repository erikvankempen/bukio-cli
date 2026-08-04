// Accounts — chart of accounts CRUD + default chart seeding.
import { DEFAULT_CHART } from './chart.js';

const VALID_TYPES = ['asset', 'liability', 'equity', 'income', 'expense'];
const VALID_NORMAL = ['debit', 'credit'];

export function accountError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

export function createAccount(db, { code, name, type, normalBalance, rgsCode = null }) {
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

export function listAccounts(db, { activeOnly = false } = {}) {
  if (activeOnly) {
    return db.prepare('SELECT * FROM accounts WHERE active = 1 ORDER BY code').all();
  }
  return db.prepare('SELECT * FROM accounts ORDER BY code').all();
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
