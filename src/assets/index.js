/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio fixed assets module (Phase 7) — depreciation schemes, asset register
// with mid-life adoption, monthly depreciation runs, disposal, activastaat.
//
// Design rules:
// - Assets enter the module at `recognition_date` with their cumulative
//   depreciation AT recognition; the purchase and the past depreciation are
//   already in the ledger and are NEVER re-booked. The module only books the
//   REMAINING depreciation, monthly, on the 1st of each month (first run =
//   the first 1st >= max(recognition, depreciation_start)).
// - Money is integer cents everywhere. Runs are cents-exact: linear monthly =
//   round(remaining/months); degressief = double-declining on the remaining
//   book value, but never less than the linear view (the standard NL
//   "switch to linear" rule, per AFAS/BC).
// - Every run is idempotent (asset_id + period), sourced 'assets' with
//   source_ref 'asset:<id>:<YYYY-MM>', and audited.
import { createEntry, postEntry } from '../core/entries.js';
import { getAccountByCode } from '../core/accounts.js';
import { record } from '../audit/index.js';

export function assetsError(code, message) {
  const err = new Error(message);
  err.code = code;
  return err;
}

const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;

function validDate(s) {
  if (!DATE_RE.test(s)) return false;
  const d = new Date(`${s}T00:00:00Z`);
  return !Number.isNaN(d.getTime()) && d.toISOString().slice(0, 10) === s;
}

function monthDiff(a, b) {
  const [y1, m1] = a.slice(0, 7).split('-').map(Number);
  const [y2, m2] = b.slice(0, 7).split('-').map(Number);
  return y2 * 12 + m2 - (y1 * 12 + m1);
}

function addMonths(dateStr, n) {
  const [y, m, d] = dateStr.split('-').map(Number);
  const total = y * 12 + (m - 1) + n;
  const yy = Math.floor(total / 12);
  const mm = (total % 12) + 1;
  // clamp the day to the target month's last day (Jan 31 + 1 = Feb 28/29)
  const lastDay = new Date(Date.UTC(yy, mm, 0)).getUTCDate();
  return `${yy}-${String(mm).padStart(2, '0')}-${String(Math.min(d, lastDay)).padStart(2, '0')}`;
}

/** Next period string after 'YYYY-MM'. */
function nextPeriod(period) {
  const [y, m] = period.split('-').map(Number);
  const total = y * 12 + m; // 1-based month; +1 lands in the next month
  return `${Math.floor(total / 12)}-${String((total % 12) + 1).padStart(2, '0')}`;
}

function firstRunPeriod(recognitionDate, startDate) {
  const effective = recognitionDate > startDate ? recognitionDate : startDate;
  const day = Number(effective.slice(8, 10));
  const month = effective.slice(0, 7);
  return day === 1 ? month : nextPeriod(month);
}

// --- schemes ----------------------------------------------------------------

export function listSchemes(db) {
  return db.prepare('SELECT * FROM depreciation_schemes ORDER BY id').all();
}

export function getScheme(db, id) {
  return db.prepare('SELECT * FROM depreciation_schemes WHERE id = ?').get(id) ?? null;
}

export function getSchemeByName(db, name) {
  return db.prepare('SELECT * FROM depreciation_schemes WHERE name = ?').get(name) ?? null;
}

/** The standard scheme: 5 years linear, monthly, 0% residual. Created lazily
 *  (existing DBs migrated to 009 have no seeded scheme). */
export function ensureDefaultScheme(db, actor = 'human') {
  const existing = getSchemeByName(db, 'Standaard 5 jaar lineair');
  if (existing) return existing;
  return createScheme(db, {
    name: 'Standaard 5 jaar lineair', method: 'lineair', lifeMonths: 60,
    residualBp: 0, actor,
  });
}

export function createScheme(db, {
  name, method = 'lineair', lifeMonths = 60, residualBp = 0, actor = 'human', dryRun = false,
}) {
  const clean = String(name ?? '').trim();
  if (!clean) throw assetsError('INVALID_NAME', 'scheme needs a name');
  if (getSchemeByName(db, clean)) throw assetsError('SCHEME_NAME_TAKEN', `scheme '${clean}' already exists`);
  if (!['lineair', 'degressief'].includes(method)) {
    throw assetsError('INVALID_METHOD', `method must be 'lineair' or 'degressief', got '${method}'`);
  }
  if (!Number.isInteger(lifeMonths) || lifeMonths < 1 || lifeMonths > 600) {
    throw assetsError('INVALID_LIFE', 'life-months must be an integer between 1 and 600');
  }
  if (!Number.isInteger(residualBp) || residualBp < 0 || residualBp > 10000) {
    throw assetsError('INVALID_RESIDUAL', 'residual-bp must be between 0 and 10000');
  }
  if (dryRun) {
    return { action: 'assets.scheme.add', name: clean, method, life_months: lifeMonths, residual_bp: residualBp, dryRun: true };
  }
  const info = db.prepare(
    'INSERT INTO depreciation_schemes (name, method, life_months, residual_bp, created_by) VALUES (?, ?, ?, ?, ?)',
  ).run(clean, method, lifeMonths, residualBp, actor);
  record(db, { actor, action: 'assets.scheme.add', command: 'assets scheme add', args: { name: clean, method, life_months: lifeMonths, residual_bp: residualBp }, outcome: 'ok' });
  return getScheme(db, info.lastInsertRowid);
}

// --- assets -----------------------------------------------------------------

function serializeAsset(db, row) {
  const scheme = getScheme(db, row.scheme_id);
  const accountRow = (id) => db.prepare('SELECT code, name FROM accounts WHERE id = ?').get(id);
  const assetAcc = accountRow(row.asset_account_id);
  const cumDepAcc = row.cum_dep_account_id ? accountRow(row.cum_dep_account_id) : null;
  const expenseAcc = accountRow(row.expense_account_id);
  return {
    ...row,
    asset_account_code: assetAcc?.code ?? null, asset_account_name: assetAcc?.name ?? null,
    cum_dep_account_code: cumDepAcc?.code ?? null, cum_dep_account_name: cumDepAcc?.name ?? null,
    expense_account_code: expenseAcc?.code ?? null, expense_account_name: expenseAcc?.name ?? null,
    scheme: scheme ? { id: scheme.id, name: scheme.name, method: scheme.method, life_months: scheme.life_months, residual_bp: scheme.residual_bp } : null,
  };
}

export function getAsset(db, id) {
  const row = db.prepare('SELECT * FROM assets WHERE id = ?').get(id);
  return row ? serializeAsset(db, row) : null;
}

export function listAssets(db, { status = null } = {}) {
  const rows = status
    ? db.prepare('SELECT * FROM assets WHERE status = ? ORDER BY id').all(status)
    : db.prepare('SELECT * FROM assets ORDER BY id').all();
  return rows.map((r) => serializeAsset(db, r));
}

/** Cumulative depreciation booked by the MODULE for an asset up to a period
 *  (inclusive). Recognition cum-dep is NOT included. */
export function moduleDepToPeriod(db, assetId, period) {
  const row = db.prepare(
    "SELECT COALESCE(SUM(amount_cents),0) AS total FROM asset_depreciation_runs WHERE asset_id = ? AND period <= ?",
  ).get(assetId, period);
  return row.total;
}

/** Pure schedule: the monthly amounts from firstPeriod through asOf, stopping
 *  at the residual. Linear: round(remaining/monthsLeft), remainder-adjusted
 *  tail. Degressief: double-declining on the remaining book value, floored at
 *  the linear view (the NL standard switch-to-linear rule).
 */
export function scheduleDepreciation({
  costCents, residualCents = 0, lifeMonths, method = 'lineair',
  firstPeriod, asOf,
}) {
  const depreciable = costCents - residualCents;
  if (depreciable <= 0 || !firstPeriod || !asOf || asOf < firstPeriod) return [];
  const rate = method === 'degressief' ? 2 / lifeMonths : 0;
  const out = [];
  let remaining = depreciable;
  let monthsLeft = lifeMonths;
  let p = firstPeriod;
  let guard = 0;
  while (p <= asOf && remaining > 0 && guard < 2000) {
    guard += 1;
    const linearAmt = Math.round(remaining / monthsLeft);
    const degAmt = method === 'degressief' ? Math.round(remaining * rate) : 0;
    let amt = method === 'degressief' ? Math.max(linearAmt, degAmt) : linearAmt;
    if (amt <= 0) amt = remaining; // tiny remainder books fully
    amt = Math.min(amt, remaining); // never overshoot the residual
    out.push({ period: p, amountCents: amt });
    remaining -= amt;
    monthsLeft -= 1;
    if (monthsLeft <= 0) monthsLeft = 1; // degressief tail safety
    p = nextPeriod(p);
  }
  return out;
}

function resolveAccount(db, code, expectedType, label) {
  if (!code) return null;
  const account = getAccountByCode(db, String(code).trim());
  if (!account) throw assetsError('ACCOUNT_NOT_FOUND', `${label} account ${code} does not exist`);
  if (expectedType && account.type !== expectedType) {
    throw assetsError('ACCOUNT_TYPE', `${label} account ${code} is '${account.type}', expected '${expectedType}'`);
  }
  return account;
}

export function addAsset(db, {
  name, category = null, serial = null, schemeId = null, method = null,
  lifeMonths = null, residualBp = null, residualCents = null,
  purchaseDate, purchasePriceCents, depreciationStartDate, recognitionDate,
  cumDepAtRecognitionCents = 0, assetAccount, cumDepAccount = null,
  expenseAccount, entryId = null, note = null, actor = 'human', dryRun = false,
}) {
  const cleanName = String(name ?? '').trim();
  if (!cleanName) throw assetsError('INVALID_NAME', 'asset needs a name');
  if (!validDate(purchaseDate)) throw assetsError('INVALID_DATE', `purchase-date '${purchaseDate}' must be yyyy-mm-dd`);
  if (!validDate(depreciationStartDate)) throw assetsError('INVALID_DATE', `depreciation-start-date '${depreciationStartDate}' must be yyyy-mm-dd`);
  if (!validDate(recognitionDate)) throw assetsError('INVALID_DATE', `recognition-date '${recognitionDate}' must be yyyy-mm-dd`);
  if (depreciationStartDate < purchaseDate) {
    throw assetsError('INVALID_DATE', 'depreciation-start-date cannot be before purchase-date');
  }
  if (recognitionDate < purchaseDate) {
    throw assetsError('INVALID_DATE', 'recognition-date cannot be before purchase-date');
  }
  if (!Number.isInteger(purchasePriceCents) || purchasePriceCents <= 0) {
    throw assetsError('INVALID_COST', 'purchase-price must be a positive amount in cents');
  }
  if (!Number.isInteger(cumDepAtRecognitionCents) || cumDepAtRecognitionCents < 0) {
    throw assetsError('INVALID_DEPRECIATION', 'cumulative depreciation at recognition must be >= 0');
  }

  // scheme: explicit id, or built from inline method/life/residual, or default
  let scheme;
  if (schemeId) {
    scheme = getScheme(db, schemeId);
    if (!scheme) throw assetsError('SCHEME_NOT_FOUND', `scheme ${schemeId} does not exist`);
  } else if (method || lifeMonths) {
    const schemeName = `${lifeMonths ?? 60} maanden ${method ?? 'lineair'} ${residualBp ?? 0}basis`;
    if (dryRun) {
      // a dry-run must not write: resolve an EXISTING scheme with the same
      // parameters, else use an in-memory plan object — calling
      // createScheme here would INSERT a row (and a second dry-run with the
      // same inline params would then fail SCHEME_NAME_TAKEN)
      scheme = getSchemeByName(db, schemeName) ?? {
        id: null, name: schemeName, method: method ?? 'lineair',
        life_months: lifeMonths ?? 60, residual_bp: residualBp ?? 0,
      };
    } else {
      scheme = createScheme(db, { name: schemeName, method: method ?? 'lineair', lifeMonths: lifeMonths ?? 60, residualBp: residualBp ?? 0, actor });
    }
  } else {
    if (dryRun) {
      scheme = getSchemeByName(db, 'Standaard 5 jaar lineair') ?? {
        id: null, name: 'Standaard 5 jaar lineair', method: 'lineair',
        life_months: 60, residual_bp: 0,
      };
    } else {
      scheme = ensureDefaultScheme(db, actor);
    }
  }
  const schemeResidualCents = Math.round((scheme.residual_bp / 10000) * purchasePriceCents);
  const residual = residualCents !== null ? residualCents : schemeResidualCents;
  if (!Number.isInteger(residual) || residual < 0 || residual >= purchasePriceCents) {
    throw assetsError('INVALID_RESIDUAL', `residual must be >= 0 and < purchase-price (got ${residual})`);
  }

  const assetAccountRow = resolveAccount(db, assetAccount, 'asset', 'asset');
  if (!assetAccountRow) throw assetsError('ACCOUNT_NOT_FOUND', 'asset account is required');
  const cumDepAccountRow = resolveAccount(db, cumDepAccount, 'asset', 'cumulative-depreciation');
  const expenseAccountRow = resolveAccount(db, expenseAccount, 'expense', 'expense');
  if (!expenseAccountRow) throw assetsError('ACCOUNT_NOT_FOUND', 'expense account is required');

  if (entryId) {
    const entry = db.prepare('SELECT id, date FROM journal_entries WHERE id = ?').get(entryId);
    if (!entry) throw assetsError('ENTRY_NOT_FOUND', `entry ${entryId} does not exist`);
  }

  const depreciable = purchasePriceCents - residual;
  if (cumDepAtRecognitionCents > depreciable) {
    throw assetsError('INVALID_DEPRECIATION',
      `cumulative depreciation at recognition (${cumDepAtRecognitionCents}) exceeds cost minus residual (${depreciable})`);
  }

  const elapsed = Math.max(0, monthDiff(depreciationStartDate, recognitionDate));
  const monthsLeft = scheme.life_months - elapsed;
  const firstPeriod = firstRunPeriod(recognitionDate, depreciationStartDate);
  const fullyDepreciated = monthsLeft <= 0 || cumDepAtRecognitionCents >= depreciable;

  if (dryRun) {
    return {
      asset: {
        name: cleanName, category, serial, purchase_date: purchaseDate,
        purchase_price_cents: purchasePriceCents, residual_cents: residual,
        depreciation_start_date: depreciationStartDate, recognition_date: recognitionDate,
        cum_dep_at_recognition_cents: cumDepAtRecognitionCents,
        scheme: scheme.name, method: scheme.method, life_months: scheme.life_months,
        asset_account: assetAccountRow.code, cum_dep_account: cumDepAccountRow?.code ?? null,
        expense_account: expenseAccountRow.code,
        months_left: Math.max(0, monthsLeft), first_run_period: firstPeriod,
        status: fullyDepreciated ? 'fully_depreciated' : 'active',
      },
      dryRun: true,
    };
  }

  const info = db.prepare(`
    INSERT INTO assets (name, category, serial, status, scheme_id, purchase_date,
      purchase_price_cents, residual_cents, depreciation_start_date, recognition_date,
      cum_dep_at_recognition_cents, asset_account_id, cum_dep_account_id,
      expense_account_id, entry_id, note, created_by)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(
    cleanName, category ?? null, serial ?? null,
    fullyDepreciated ? 'fully_depreciated' : 'active',
    scheme.id, purchaseDate, purchasePriceCents, residual,
    depreciationStartDate, recognitionDate, cumDepAtRecognitionCents,
    assetAccountRow.id, cumDepAccountRow?.id ?? null, expenseAccountRow.id,
    entryId ?? null, note ?? null, actor,
  );
  const asset = getAsset(db, info.lastInsertRowid);

  // GL reconciliation warnings (never blockers — migration must stay frictionless)
  const warnings = [];
  // posted entries only — a draft (source not posted) would inflate the
  // balance and silently suppress a real warning
  const assetBalance = db.prepare(`
    SELECT COALESCE(SUM(p.amount_cents),0) AS t FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
    WHERE p.account_id = ?
  `).get(assetAccountRow.id).t;
  if (assetBalance < purchasePriceCents) {
    warnings.push(`asset account ${assetAccountRow.code} carries ${assetBalance} on the ledger, less than the purchase price ${purchasePriceCents} — verify the purchase was booked there`);
  }
  if (cumDepAccountRow) {
    const cumBalance = db.prepare(`
      SELECT COALESCE(SUM(p.amount_cents),0) AS t FROM postings p
      JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
      WHERE p.account_id = ?
    `).get(cumDepAccountRow.id).t;
    if (Math.abs(cumBalance) < cumDepAtRecognitionCents) {
      warnings.push(`cumulative-depreciation account ${cumDepAccountRow.code} carries ${cumBalance} on the ledger, less than the recognised ${cumDepAtRecognitionCents}`);
    }
  }
  if (entryId && db.prepare('SELECT date FROM journal_entries WHERE id = ?').get(entryId).date !== purchaseDate) {
    warnings.push(`linked entry ${entryId} has a different date than purchase-date`);
  }

  record(db, {
    actor, action: 'assets.add', command: 'assets add',
    args: { name: cleanName, purchase_price_cents: purchasePriceCents, scheme: scheme.name, recognition_date: recognitionDate, cum_dep_at_recognition_cents: cumDepAtRecognitionCents },
    outcome: 'ok',
  });
  return { asset, warnings, dryRun: false };
}

// --- runs -------------------------------------------------------------------

export function runDue(db, { asOf = null, period = null, actor = 'human', dryRun = false }) {
  if (period != null && !/^\d{4}-(0[1-9]|1[0-2])$/.test(period)) {
    throw assetsError('INVALID_PERIOD', `period '${period}' must be YYYY-MM (01-12)`);
  }
  if (asOf != null && !validDate(asOf)) {
    throw assetsError('INVALID_DATE', `as-of '${asOf}' must be yyyy-mm-dd`);
  }
  const target = period ?? asOf?.slice(0, 7) ?? new Date().toISOString().slice(0, 7);
  const assets = listAssets(db);
  const plan = [];
  const toBook = [];

  for (const a of assets) {
    if (a.status !== 'active') continue;
    const firstPeriod = firstRunPeriod(a.recognition_date, a.depreciation_start_date);
    const elapsed = Math.max(0, monthDiff(a.depreciation_start_date, a.recognition_date));
    const monthsLeft = Math.max(1, a.scheme.life_months - elapsed);
    // only the REMAINING amount is depreciated by the module (the cumulative
    // depreciation at recognition is already booked in the ledger)
    const remainingCost = a.purchase_price_cents - a.residual_cents - a.cum_dep_at_recognition_cents;
    if (remainingCost <= 0) continue;
    const schedule = scheduleDepreciation({
      costCents: remainingCost, residualCents: 0,
      lifeMonths: monthsLeft, method: a.scheme.method, firstPeriod, asOf: target,
    });
    const booked = new Set(
      db.prepare('SELECT period FROM asset_depreciation_runs WHERE asset_id = ?').all(a.id).map((r) => r.period),
    );
    const due = schedule.filter((s) => !booked.has(s.period));
    if (due.length) {
      plan.push({
        asset_id: a.id, name: a.name, periods: due,
        total_cents: due.reduce((sum, s) => sum + s.amountCents, 0),
      });
      toBook.push({ asset: a, due });
    }
  }
  if (toBook.length === 0) return { booked: [], plan, dryRun };
  if (dryRun) return { booked: [], plan, dryRun: true };

  const result = [];
  const tx = db.transaction(() => {
    for (const { asset, due } of toBook) {
      for (const s of due) {
        const postings = [
          { code: asset.expense_account_code, amountCents: s.amountCents },
          { code: asset.cum_dep_account_code ?? asset.asset_account_code, amountCents: -s.amountCents },
        ];
        const entry = createEntry(db, {
          date: `${s.period}-01`, description: `Afschrijving ${asset.name} ${s.period}`,
          postings, source: 'assets', sourceRef: `asset:${asset.id}:${s.period}`, actor,
        });
        const posted = postEntry(db, { id: entry.id, actor });
        db.prepare('INSERT INTO asset_depreciation_runs (asset_id, period, entry_id, amount_cents, created_by) VALUES (?, ?, ?, ?, ?)')
          .run(asset.id, s.period, posted.id, s.amountCents, actor);
        result.push({ asset_id: asset.id, period: s.period, entry_id: posted.id, amount_cents: s.amountCents });
        // auto-complete: asset reached residual
        const totalDep = asset.cum_dep_at_recognition_cents + moduleDepToPeriod(db, asset.id, s.period);
        if (totalDep >= asset.purchase_price_cents - asset.residual_cents) {
          db.prepare("UPDATE assets SET status = 'fully_depreciated' WHERE id = ? AND status = 'active'").run(asset.id);
        }
      }
    }
  });
  tx();

  record(db, {
    actor, action: 'assets.run', command: 'assets run',
    args: { period: target, booked: result.length, assets: toBook.length },
    outcome: 'ok', entryIds: result.map((r) => r.entry_id),
  });
  return { booked: result, plan: dryRun ? plan : [], dryRun: false };
}

// --- activastaat ------------------------------------------------------------

export function register(db, { asOf = null, actor = 'human' } = {}) {
  if (asOf != null && !validDate(asOf)) {
    throw assetsError('INVALID_DATE', `as-of '${asOf}' must be yyyy-mm-dd`);
  }
  const targetPeriod = asOf ? asOf.slice(0, 7) : new Date().toISOString().slice(0, 7);
  const rows = [];
  for (const a of listAssets(db)) {
    const booked = moduleDepToPeriod(db, a.id, targetPeriod);
    const totalDep = a.cum_dep_at_recognition_cents + booked;
    const bookValue = Math.max(0, a.purchase_price_cents - totalDep);
    const remaining = Math.max(0, a.purchase_price_cents - a.residual_cents - totalDep);
    const firstPeriod = firstRunPeriod(a.recognition_date, a.depreciation_start_date);
    const elapsed = Math.max(0, monthDiff(a.depreciation_start_date, a.recognition_date));
    const monthsLeft = Math.max(1, a.scheme.life_months - elapsed);
    const remainingCost = Math.max(0, a.purchase_price_cents - a.residual_cents - a.cum_dep_at_recognition_cents);
    const nextSchedule = scheduleDepreciation({
      costCents: remainingCost, residualCents: 0,
      lifeMonths: monthsLeft, method: a.scheme.method, firstPeriod, asOf: nextPeriod(targetPeriod),
    });
    const bookedSet = new Set(db.prepare('SELECT period FROM asset_depreciation_runs WHERE asset_id = ?').all(a.id).map((r) => r.period));
    const nextRun = nextSchedule.find((s) => !bookedSet.has(s.period))?.period ?? null;
    rows.push({
      id: a.id, name: a.name, category: a.category, serial: a.serial, status: a.status,
      scheme: a.scheme.name, method: a.scheme.method, life_months: a.scheme.life_months,
      purchase_date: a.purchase_date, purchase_price_cents: a.purchase_price_cents,
      residual_cents: a.residual_cents,
      cum_dep_at_recognition_cents: a.cum_dep_at_recognition_cents,
      module_dep_to_date_cents: booked,
      total_cum_dep_cents: totalDep,
      book_value_cents: bookValue,
      remaining_to_depreciate_cents: remaining,
      next_run_period: nextRun,
      asset_account: a.asset_account_code, cum_dep_account: a.cum_dep_account_code,
      expense_account: a.expense_account_code,
      disposed_date: a.disposed_date, disposed_proceeds_cents: a.disposed_proceeds_cents,
    });
  }
  const totals = {
    purchase_price_cents: rows.reduce((s, r) => s + r.purchase_price_cents, 0),
    total_cum_dep_cents: rows.reduce((s, r) => s + r.total_cum_dep_cents, 0),
    book_value_cents: rows.reduce((s, r) => s + r.book_value_cents, 0),
  };
  return { as_of: targetPeriod, assets: rows, totals, actor };
}

// --- disposal ---------------------------------------------------------------

export function disposeAsset(db, {
  id, date, proceedsCents = 0, bankAccount = null, resultAccount = null,
  note = null, actor = 'human', dryRun = false,
}) {
  const asset = getAsset(db, id);
  if (!asset) throw assetsError('ASSET_NOT_FOUND', `asset ${id} does not exist`);
  if (asset.status === 'disposed') throw assetsError('ALREADY_DISPOSED', `asset ${id} is already disposed`);
  if (!validDate(date)) throw assetsError('INVALID_DATE', `date '${date}' must be yyyy-mm-dd`);
  if (date < asset.recognition_date) throw assetsError('INVALID_DATE', 'disposal date cannot be before the recognition date');
  if (!Number.isInteger(proceedsCents) || proceedsCents < 0) {
    throw assetsError('INVALID_AMOUNT', 'proceeds must be a non-negative amount in cents');
  }

  // Depreciation stops at the end of the month BEFORE the disposal month.
  const disposalMonth = date.slice(0, 7);
  const prevPeriod = addMonths(`${disposalMonth}-01`, -1).slice(0, 7);
  const moduleDep = moduleDepToPeriod(db, id, prevPeriod);
  const totalCumDep = asset.cum_dep_at_recognition_cents + moduleDep;
  const bookValue = asset.purchase_price_cents - totalCumDep;
  const result = proceedsCents - bookValue; // winst > 0, verlies < 0

  let bank = bankAccount ? resolveAccount(db, bankAccount, null, 'bank') : null;
  if (proceedsCents > 0 && !bank) {
    bank = resolveAccount(db, '1100', null, 'bank');
    if (!bank) throw assetsError('ACCOUNT_NOT_FOUND', 'bank account required for proceeds > 0 (default 1100 not found)');
  }
  const resultAccountRow = resolveAccount(db, resultAccount ?? '8100', null, 'result');

  const postings = [];
  if (proceedsCents > 0) postings.push({ code: bank.code, amountCents: proceedsCents });
  // the cumulative-depreciation leg lands on the cum-dep account when one is
  // set, otherwise on the asset account itself (its runs book there too —
  // this keeps the disposal entry balanced in both configurations)
  if (totalCumDep > 0) {
    const cumLegCode = asset.cum_dep_account_code ?? asset.asset_account_code;
    postings.push({ code: cumLegCode, amountCents: totalCumDep });
  }
  postings.push({ code: asset.asset_account_code, amountCents: -asset.purchase_price_cents });
  // the result leg is skipped when it would be zero — createEntry rejects
  // zero-amount postings, and disposing exactly at book value (or a fully
  // depreciated asset with no proceeds) would otherwise crash
  if (result !== 0) postings.push({ code: resultAccountRow.code, amountCents: -result }); // +winst credit, -verlies debit

  if (dryRun) {
    return {
      asset: { id: asset.id, name: asset.name },
      date, proceeds_cents: proceedsCents, book_value_cents: bookValue,
      result_cents: result, total_cum_dep_cents: totalCumDep,
      postings, dryRun: true,
    };
  }

  const description = `Afstoting ${asset.name} (${date})`;
  // Atomic: entry + asset-status change commit together. A crash between
  // postEntry's commit and the UPDATE assets left the entry posted with the
  // asset still active — a retry then booked the disposal twice.
  let posted = null;
  db.transaction(() => {
    const entry = createEntry(db, {
      date, description, postings, source: 'assets',
      sourceRef: `dispose:${asset.id}:${date}`, actor,
    });
    posted = postEntry(db, { id: entry.id, actor });
    db.prepare('UPDATE assets SET status = ?, disposed_date = ?, disposed_proceeds_cents = ?, disposal_entry_id = ? WHERE id = ?')
      .run('disposed', date, proceedsCents, posted.id, id);
  })();
  record(db, {
    actor, action: 'assets.dispose', command: 'assets dispose',
    args: { asset_id: id, date, proceeds_cents: proceedsCents, result_cents: result },
    outcome: 'ok', entryIds: [posted.id],
  });
  return {
    asset: { id: asset.id, name: asset.name, status: 'disposed', disposed_date: date, disposed_proceeds_cents: proceedsCents },
    entry: { id: posted.id, date, description, state: posted.state },
    book_value_cents: bookValue, result_cents: result,
    postings, dryRun: false,
  };
}
