/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// VAT module (optional) — codes, VAT-aware booking, OB manual-filing readout.
// Only active when company.vat_module = 1. The core ledger stays VAT-agnostic:
// this module expands "@code" posting specs into full entries (including the
// VAT ledger legs) and computes the OB-aangifte fields for manual filing.
import { vatError } from './errors.js';
import { createAccount, getAccountByCode } from '../core/accounts.js';
import { createEntry, parsePostingSpecs, postEntry } from '../core/entries.js';
import { formatAmount } from '../core/money.js';
import { getProfile, resolveProfile } from '../jurisdictions/index.js';
import { record } from '../audit/index.js';

// NL profile data is the single source of truth (Phase A M4); these exports
// stay as NL conveniences for the CLI/tests — contents identical to the
// legacy hardcoded arrays
export const VAT_ACCOUNTS = getProfile('NL').tax.accounts.ledger;
export const VAT_CODES = getProfile('NL').tax.codes;

export function isVatEnabled(db) {
  const company = db.prepare('SELECT vat_module, kor_flag FROM company').get();
  return Boolean(company && company.vat_module && !company.kor_flag);
}

function requireVat(db) {
  if (!isVatEnabled(db)) {
    throw vatError('VAT_MODULE_OFF', 'the VAT module is not enabled for this company (enable with `bukio vat enable`)');
  }
}

/** Enable the VAT module: flag + VAT accounts + VAT codes. Idempotent. */
export function enableVatModule(db, { actor = 'human' } = {}) {
  const company = db.prepare('SELECT vat_module, kor_flag FROM company').get();
  if (company.kor_flag) {
    throw vatError('KOR_ACTIVE', 'this company uses the KOR (kleineondernemersregeling) — the VAT module cannot be enabled');
  }
  const { tax } = resolveProfile(db);
  const tx = db.transaction(() => {
    db.prepare('UPDATE company SET vat_module = 1 WHERE id = 1').run();
    for (const a of tax.accounts.ledger) {
      if (!getAccountByCode(db, a.code)) createAccount(db, a);
    }
    const insertCode = db.prepare(
      'INSERT OR IGNORE INTO vat_codes (code, rate_bp, type, eu_reverse, description) VALUES (?, ?, ?, ?, ?)',
    );
    for (const c of tax.codes) insertCode.run(c.code, c.rateBp, c.type, c.euReverse, c.description);
    record(db, { actor, action: 'vat.enable', command: 'vat enable', args: {}, outcome: 'ok' });
  });
  tx();
  return { vat_module: 1, accounts: tax.accounts.ledger.map((a) => a.code), codes: tax.codes.map((c) => c.code) };
}

export function listVatCodes(db) {
  return db.prepare('SELECT * FROM vat_codes ORDER BY rate_bp DESC, code').all();
}

/**
 * Parse posting specs with optional VAT: "CODE:AMOUNT[@VATCODE]".
 * Returns [{ code, amountCents, vatCode|null }].
 */
export function parseVatPostingSpecs(raw) {
  const out = [];
  for (const item of Array.isArray(raw) ? raw : [raw]) {
    for (const token of String(item).split(',')) {
      const t = token.trim();
      if (!t) continue;
      const m = t.match(/^(\d{1,6}):(.+?)(?:@([A-Z0-9.]+))?$/);
      if (!m) throw vatError('INVALID_POSTING', `posting '${t}' must be CODE:AMOUNT[@VATCODE] (e.g. 8000:-100.00@21)`);
      out.push({ code: m[1], amountCents: parsePostingSpecs([`${m[1]}:${m[2]}`])[0].amountCents, vatCode: m[3] ?? null });
    }
  }
  return out;
}

/**
 * Expand VAT-aware posting specs into core postings:
 * - '@code' postings get vat_code + computed vat_amount (net = amount).
 * - A VAT ledger leg is added automatically (2500 te betalen for output /
 *   reverse / private, 1500 te vorderen for input).
 * The caller's other postings must balance the gross amounts (e.g. the bank
 * leg carries the VAT-inclusive amount).
 */
export function expandVatPostings(db, specs) {
  requireVat(db);
  const expanded = [];
  const vatLegs = [];
  // reverse charge / privégebruik VAT is due at the standard rate — per profile
  const reverseRate = resolveProfile(db).tax.reverseChargeEffectiveRateBp;
  // the auto VAT legs land on the PROFILE's clearing accounts (NL 1500/2500,
  // BE 411/451, FR 44566/44571, GB 2110/2100, LU 421611/461411, ...) — the
  // old hardcoded '2500'/'1500' posted to nonexistent accounts (ACCOUNT_
  // NOT_FOUND) or silently misbooked onto foreign codes on every non-NL
  // market (e.g. NO 1500 is Kundefordringer/debtors, 2500 Betalbar skatt)
  const { tax } = resolveProfile(db);
  const inputAcc = tax.accounts.ledger.find((a) => a.type === 'asset');
  const outputAcc = tax.accounts.ledger.find((a) => a.type === 'liability');
  if (!inputAcc || !outputAcc) {
    throw vatError('FORMAT_NOT_SUPPORTED', `the jurisdiction profile's VAT ledger must declare one asset and one liability clearing account (got: ${tax.accounts.ledger.map((a) => a.code).join(', ')})`);
  }

  for (const spec of specs) {
    const account = getAccountByCode(db, spec.code);
    if (!account) throw vatError('ACCOUNT_NOT_FOUND', `account ${spec.code} does not exist`);

    if (spec.vatCode) {
      const vat = db.prepare('SELECT * FROM vat_codes WHERE code = ?').get(spec.vatCode);
      if (!vat) throw vatError('VAT_CODE_NOT_FOUND', `vat code '${spec.vatCode}' does not exist`);
      if (vat.type === 'margin') {
        throw vatError('VAT_MARGIN_NOT_SUPPORTED', 'margeregeling cannot be split automatically — book it manually');
      }
      // Reverse charge / privégebruik: 0% codes, but the VAT due on the
      // verlegde levering / private use is computed at the standard rate.
      const effectiveRateBp = (vat.type === 'reverse' || vat.type === 'private') ? reverseRate : vat.rate_bp;
      // private use (@P) is ALWAYS a deemed supply — the VAT is owed (credit
      // 2500), regardless of the posting's sign. Following the posting's sign
      // here would turn a debit-signed private-use booking (e.g. an expense
      // taken privately) into a 2500 DEBIT that reduces te-betalen and makes
      // the readout report negative 1d/5a — understating the position.
      const vatAmount = vat.type === 'private'
        ? -Math.round(Math.abs(spec.amountCents * effectiveRateBp / 10000))
        : Math.round(Math.abs(spec.amountCents * effectiveRateBp / 10000)) * Math.sign(spec.amountCents);

      const isOutput = account.type === 'income' || vat.type === 'private';
      const vatAccountCode = isOutput ? outputAcc.code : inputAcc.code;
      expanded.push({
        code: spec.code, amountCents: spec.amountCents,
        vatCode: vat.code, vatAmountCents: vatAmount,
        fxCurrency: spec.fxCurrency ?? null, fxAmountCents: spec.fxAmountCents ?? null,
      });
      // Reverse charge (R/RE): NO auto VAT leg. The VAT is self-assessed
      // (verschuldigd) and claimed back (aftrekbaar) — the two legs net to
      // zero on the books, so a single leg with the posting's sign would
      // create a phantom 2500 debit (or credit) that corrupts the te-betalen
      // balance and makes vatFile under/over-file. The tagged posting still
      // carries vatAmountCents so the OB readout derives 4a/4b + 5b. This
      // matches the invoice path, which books verlegd lines with no VAT leg.
      // Private use (@P) and standard codes DO get a leg: you owe (credit
      // 2500) or claim back (debit 1500) real VAT.
      // A 0-rate code (@V vrijgesteld, @0 nultarief) computes a 0 leg — it is
      // skipped entirely (createEntry rejects zero-amount postings); the
      // tagged posting still carries vatAmountCents 0 so the readout reports
      // the base (1c/2a).
      if (vatAmount !== 0 && vat.type !== 'reverse') vatLegs.push({ code: vatAccountCode, amountCents: vatAmount });
    } else {
      expanded.push({
        code: spec.code, amountCents: spec.amountCents, vatCode: null, vatAmountCents: null,
        fxCurrency: spec.fxCurrency ?? null, fxAmountCents: spec.fxAmountCents ?? null,
      });
    }
  }

  // Rounding-drift absorption: with FX conversion every leg rounds
  // independently, so for some amounts net + vat != gross by a cent or two —
  // the entry would come out UNBALANCED. The drift is absorbed into the
  // largest UNTAGGED user leg (the classic rounding-adjustment rule); the
  // auto-added VAT legs are excluded (their amounts are derived, not chosen).
  const result = [...expanded, ...vatLegs];
  const sum = result.reduce((s, p) => s + p.amountCents, 0);
  if (sum !== 0) {
    const userLegs = result.slice(0, expanded.length);
    const candidates = userLegs.filter((p) => !p.vatCode);
    if (candidates.length) {
      const target = candidates.reduce((a, b) => (Math.abs(b.amountCents) > Math.abs(a.amountCents) ? b : a));
      target.amountCents -= sum;
    }
  }

  return result;
}

/** Book a VAT-aware entry via the core engine. */
export function bookVatEntry(db, {
  date, description, postings, source = 'manual', sourceRef = null, actor = 'human', post = false,
}) {
  const expanded = expandVatPostings(db, postings);
  const corePostings = expanded.map((p) => ({
    code: p.code, amountCents: p.amountCents, vatCode: p.vatCode, vatAmountCents: p.vatAmountCents,
    fxCurrency: p.fxCurrency, fxAmountCents: p.fxAmountCents,
  }));
  let entry = createEntry(db, {
    date, description, postings: corePostings, source, sourceRef, actor,
  });
  if (post) entry = postEntry(db, { id: entry.id, actor });
  return { entry, expanded };
}

/** Parse '2026-Q2' or '2026-07' into { from, to } (ISO dates). */
export function parsePeriod(period) {
  const q = period.match(/^(\d{4})-Q([1-4])$/);
  if (q) {
    const [, y, qn] = q;
    const from = `${y}-${String((qn - 1) * 3 + 1).padStart(2, '0')}-01`;
    const to = new Date(Date.UTC(Number(y), Number(qn) * 3, 0)).toISOString().slice(0, 10);
    return { from, to, label: period };
  }
  const m = period.match(/^(\d{4})-(\d{2})$/);
  if (m) {
    const [, y, mn] = m;
    if (Number(mn) < 1 || Number(mn) > 12) {
      throw vatError('INVALID_PERIOD', `period '${period}' must be YYYY-Qn or YYYY-MM (month 01-12)`);
    }
    const from = `${y}-${mn}-01`;
    const to = new Date(Date.UTC(Number(y), Number(mn), 0)).toISOString().slice(0, 10);
    return { from, to, label: period };
  }
  throw vatError('INVALID_PERIOD', `period '${period}' must be YYYY-Qn or YYYY-MM`);
}

/**
 * OB-aangifte manual-filing readout for a period.
 * Computes fields 1a-5d from posted entries carrying VAT codes.
 * This is an aid for MANUAL filing in Mijn Belastingdienst — bukio never
 * submits anything.
 */
// OB readout builders keyed by profile.tax.returnLayout. NL is the only
// layout in Phase A ('ob-1a-5d'); future markets register their own mapper.
const OB_LAYOUTS = {
  'ob-1a-5d': buildObReadoutNl,
};

export function obReadout(db, { period }) {
  requireVat(db);
  const { tax } = resolveProfile(db);
  const builder = OB_LAYOUTS[tax.returnLayout];
  if (!builder) {
    throw vatError('FORMAT_NOT_SUPPORTED', `return layout '${tax.returnLayout}' has no builder (registered: ${Object.keys(OB_LAYOUTS).join(', ')})`);
  }
  return builder(db, { period });
}

function buildObReadoutNl(db, { period }) {
  const { from, to, label } = parsePeriod(period);
  const { tax } = resolveProfile(db);
  const clearing = tax.accounts.ledger.map((a) => a.code);

  const rows = db.prepare(`
    SELECT p.amount_cents, p.vat_amount_cents, vc.code AS vat_code, vc.rate_bp, vc.type, vc.eu_reverse,
           a.type AS account_type, a.code AS account_code
    FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
    JOIN vat_codes vc ON vc.id = p.vat_code_id
    JOIN accounts a ON a.id = p.account_id
    WHERE e.date >= ? AND e.date <= ?
      AND p.vat_code_id IS NOT NULL
      AND a.code NOT IN (${clearing.map((c) => `'${c}'`).join(',')})
    ORDER BY e.id, p.id
  `).all(from, to);

  const f = {
    '1a': 0, '1b': 0, '1c': 0, '1d': 0,
    '2a': 0, '2b': 0,
    '3a': 0, '3b': 0, '3c': 0,
    '4a': 0, '4b': 0,
    '5a': 0, '5b': 0, '5c': 0,
  };

  for (const r of rows) {
    const rate = r.rate_bp;
    if (r.type === 'standard' || r.type === 'exempt') {
      if (r.account_type === 'income') {
        // output (sales): base = -amount, vat = -vat_amount
        const base = -r.amount_cents;
        if (rate === 2100) f['1a'] += base;
        else if (rate === 900) f['1b'] += base;
        else f['1c'] += base;
        if (r.type === 'standard') f['5a'] += -r.vat_amount_cents;
      } else {
        // input (purchases): base = amount
        const base = r.amount_cents;
        if (rate === 2100) f['3a'] += base;
        else if (rate === 900) f['3b'] += base;
        else f['3c'] += base;
        if (r.type === 'standard') f['5b'] += r.vat_amount_cents;
      }
    } else if (r.type === 'reverse') {
      if (r.account_type === 'income' && r.eu_reverse) {
        // uitgaande verlegde EU levering (0%): base naar 2a, geen btw
        f['2a'] += -r.amount_cents;
      } else if (r.account_type === 'income') {
        // uitgaande verlegde BINNENLANDSE levering (R, zeldzaam maar legaal):
        // de base blijft een belaste levering in NL -> 1c (0%-tarief valt in
        // 'andere tarieven'); de btw is niet verschuldigd door ons (geen 5a)
        f['1c'] += -r.amount_cents;
      } else if (r.account_type === 'expense') {
        // inkoop met verlegde btw: base naar 3a (binnenland) / 3b (EU),
        // btw verschuldigd naar 4a/4b — and claimed back as voorbelasting
        // (5b), so pure reverse charge nets zero.
        f[r.eu_reverse ? '3b' : '3a'] += r.amount_cents;
        f[r.eu_reverse ? '4b' : '4a'] += r.vat_amount_cents;
        f['5b'] += r.vat_amount_cents;
      }
    } else if (r.type === 'private') {
      // privégebruik is a deemed supply: the base (1d) is always positive and
      // the VAT (5a) is always owed — the readout must not inherit the sign
      // of whatever side the user booked the posting on
      f['1d'] += Math.abs(r.amount_cents);
      f['5a'] += -r.vat_amount_cents;
    }
    // type 'margin' is excluded from the return
  }

  f['5d'] = f['5a'] + f['4a'] + f['4b'] + f['5c'] - f['5b'];

  return {
    period: label,
    from,
    to,
    fields: f,
    to_pay_cents: f['5d'],
    to_pay: String((f['5d'] / 100).toFixed(2)),
    note: 'Manual filing aid only — bukio never submits. Fields 2b (non-EU exports) and 5c are not tracked and shown as 0.',
  };
}

/** Record that a period was filed manually. */
export function markFiled(db, { period, actor = 'human' }) {
  requireVat(db);
  const { from, to, label } = parsePeriod(period);
  const readout = obReadout(db, { period: label });
  db.prepare(`
    INSERT INTO vat_returns (type, period, status, fields_json, filed_at)
    VALUES ('OB', ?, 'filed', ?, ?)
    ON CONFLICT(type, period) DO UPDATE SET status = 'filed', fields_json = excluded.fields_json, filed_at = excluded.filed_at
  `).run(label, JSON.stringify(readout.fields), new Date().toISOString());
  record(db, { actor, action: 'vat.filed', command: 'vat readout --mark-filed', args: { period: label }, outcome: 'ok' });
  return { period: label, from, to, status: 'filed' };
}

// --- Filing & settlement (af te dragen omzetbelasting) ----------------------
// Dutch practice: at filing the net VAT position is reclassified from the
// 1500/2500 clearing accounts to a separate liability account ('Af te dragen
// omzetbelasting', default 2510); the later bank payment cancels that balance.
// The OB-aangifte is filed in WHOLE euros rounded in your favour, so the
// payment differs from the booked (exact-cents) liability by a few cents —
// that rounding difference is booked to the P&L (difference account) at
// settlement, per the Belastingdienst rule that amounts are rounded per line.
export const VAT_FILE_ACCOUNT_DEFAULT = '2510';
export const VAT_DIFFERENCE_ACCOUNT_DEFAULT = '4700';
// per-line rounding is < €0.50 and the return has ~9 lines, so a legit
// settlement difference is at most a few euros; beyond €5.00 something else
// is wrong (wrong amount, wrong filing)
export const VAT_SETTLE_MAX_DIFFERENCE_CENTS = 500;

/** Signed balance of an account over POSTED postings (debit-positive convention). */
function accountBalance(db, code) {
  const row = db.prepare(`
    SELECT COALESCE(SUM(p.amount_cents), 0) AS bal
    FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
    JOIN accounts a ON a.id = p.account_id
    WHERE a.code = ?
  `).get(code);
  return row.bal;
}

/** Net VAT position: positive = you owe (2500 te betalen), negative = refund (1500 te vorderen). */
export function vatNetPosition(db) {
  requireVat(db);
  const ledger = resolveProfile(db).tax.accounts.ledger.map((a) => a.code);
  const total = ledger.reduce((sum, code) => sum + accountBalance(db, code), 0);
  return -total;
}

/** Ensure the 'Af te dragen omzetbelasting' liability account exists (idempotent). */
function ensureVatSettlementAccount(db, account) {
  const resolved = resolveVatSettlementAccount(db, account);
  if (!getAccountByCode(db, resolved)) {
    // taxonomy code only when the profile's chart is taxonomy-mapped (NL RGS:
    // BSCH.12); stamping the NL code on PCN/SKR-03/PCG charts would pollute
    // the taxonomy column with a foreign scheme
    const { reporting } = resolveProfile(db);
    const usesTaxonomy = reporting.defaultChart.some((a) => a.taxonomyCode);
    createAccount(db, {
      code: resolved, name: vatSettlementAccountName(db), type: 'liability',
      normalBalance: 'credit', taxonomyCode: usesTaxonomy ? 'BSCH.12' : null,
    });
  }
  return resolved;
}

function vatSettlementAccountName(db) {
  return resolveProfile(db).tax.accounts.settlementAccountName;
}

function isVatSettlementAccount(db, a) {
  if (!a) return false;
  const { tax, reporting } = resolveProfile(db);
  // the profile-declared settlement account is the canonical af-te-dragen
  // position when it is the SEEDED chart account (bilingual charts: BE 451
  // 'TVA à payer — Te betalen BTW', DE 1780, GB 2120 — labels differ from
  // vatSettlementAccountName), so name-equality alone no longer silently falls to a
  // numeric successor. A FOREIGN account parked on the fileDefault code
  // (NL legacy-import collisions; 2510 is auto-created, never seeded)
  // still falls through to the next free code.
  const seeded = (reporting.defaultChart ?? []).find((c) => c.code === tax.accounts.fileDefault);
  if (
    seeded && a.code === tax.accounts.fileDefault
    && a.name === seeded.name && a.type === 'liability' && a.normal_balance === 'credit'
  ) return true;
  return a.name === vatSettlementAccountName(db) && a.type === 'liability' && a.normal_balance === 'credit';
}

/**
 * Decide which account code the af-te-dragen position lands on, WITHOUT
 * writing anything (used by both the plan and the real run):
 * - code free            -> as requested
 * - code exists and IS the af-te-dragen account (right name/type) -> reuse
 * - code taken by another account -> the next free numeric code (2510 -> 2511
 *   -> ...), reusing an af-te-dragen account found along the way; a
 *   non-numeric code with no successor is an error (VAT_ACCOUNT_COLLISION).
 */
function resolveVatSettlementAccount(db, account) {
  const existing = getAccountByCode(db, account);
  if (!existing) return account;
  if (isVatSettlementAccount(db, existing)) return account;
  if (!/^\d+$/.test(account)) {
    throw vatError('VAT_ACCOUNT_COLLISION', `account ${account} exists but is not '${vatSettlementAccountName(db)}' and has no numeric successor — pick a free code with --account`);
  }
  let code = String(Number(account) + 1);
  let guard = 0;
  while (getAccountByCode(db, code)) {
    if (isVatSettlementAccount(db, getAccountByCode(db, code))) return code; // a previous filing already landed here
    code = String(Number(code) + 1);
    if (++guard > 999) {
      throw vatError('VAT_ACCOUNT_COLLISION', `no free numeric successor after ${account} — pick a free code with --account`);
    }
  }
  return code;
}

/**
 * Reclassify the outstanding VAT position to the af-te-dragen account at filing.
 * The move uses the EXACT booked amounts — the OB form is filed in rounded
 * whole euros, and the cent-level difference is settled to the P&L later
 * (vatSettle), never by distorting the VAT clearing accounts.
 */
export function vatFile(db, { account = null, period = null, desc = null, actor = 'human', dryRun = false }) {
  requireVat(db);
  const { tax } = resolveProfile(db);
  account = account ?? tax.accounts.fileDefault;
  // clearing accounts come from the profile ledger (NL: 1500 te vorderen /
  // 2500 te betalen) — identified by type, not by hardcoded codes
  const inputAcc = tax.accounts.ledger.find((a) => a.type === 'asset');
  const outputAcc = tax.accounts.ledger.find((a) => a.type === 'liability');
  if (!inputAcc || !outputAcc) {
    throw vatError('FORMAT_NOT_SUPPORTED', `the jurisdiction profile's VAT ledger must declare one asset and one liability clearing account (got: ${tax.accounts.ledger.map((a) => a.code).join(', ')})`);
  }
  const balInput = accountBalance(db, inputAcc.code);
  const balOutput = accountBalance(db, outputAcc.code);
  const net = -(balOutput + balInput); // positive = owe
  if (net === 0) {
    throw vatError('VAT_NOTHING_TO_FILE', `no outstanding VAT position to reclassify (${outputAcc.code}/${inputAcc.code} net is zero)`);
  }
  // Resolve the af-te-dragen account BEFORE building the plan: a requested
  // code that is taken by another account falls to the next free numeric
  // code, and the caller sees exactly where the position will land.
  account = resolveVatSettlementAccount(db, account);
  // The FULL clearing position moves to the af-te-dragen account: both
  // clearing accounts are emptied (2500 te betalen holds the credit/output
  // legs, 1500 te vorderen the debit/input legs) and the NET lands there
  // (credit when you owe, debit when you get a refund).
  const postings = [
    { code: outputAcc.code, amountCents: -balOutput },
    { code: inputAcc.code, amountCents: -balInput },
    { code: account, amountCents: balOutput + balInput },
  ].filter((p) => p.amountCents !== 0);
  const owe = net > 0;
  const liability = Math.abs(net);
  const description = desc ?? `OB-aangifte${period ? ` ${period}` : ''} verlegging naar ${account} (${owe ? 'te betalen' : 'te ontvangen'})`;

  if (dryRun) {
    return {
      action: 'vat.file', dryRun: true, account, owe, liability_cents: liability,
      postings: postings.map((p) => ({ code: p.code, amount_cents: p.amountCents })),
    };
  }

  const entry = db.transaction(() => {
    ensureVatSettlementAccount(db, account);
    const created = createEntry(db, {
      date: new Date().toISOString().slice(0, 10), description,
      postings, source: 'manual', actor,
    });
    postEntry(db, { id: created.id, actor });
    record(db, {
      actor, action: 'vat.file', command: 'vat file',
      args: { account, period, owe, liability_cents: liability, description }, outcome: 'ok', entryIds: [created.id],
    });
    return created;
  })();
  return { action: 'vat.file', entry_id: entry.id, account, owe, liability_cents: liability, postings };
}

/**
 * Book the bank payment that cancels the af-te-dragen balance. The bank
 * transaction amount (the FILED, whole-euro amount) clears the exact-cents
 * liability; the rounding difference goes to the difference account in the
 * P&L (default 4700 Overige bedrijfskosten).
 *
 * `tx` carries the bank transaction's fields (amount_cents: outgoing negative,
 * account_code: the bank's ledger code). The caller (CLI) fetches the
 * transaction and links it to the booked entry.
 */
export function vatSettle(db, {
  txAmountCents, txDate, bankAccountCode, account = null,
  differenceAccount = null,
  period = null, desc = null, actor = 'human', dryRun = false,
}) {
  requireVat(db);
  const { tax } = resolveProfile(db);
  account = account ?? tax.accounts.fileDefault;
  differenceAccount = differenceAccount ?? tax.accounts.differenceDefault;
  if (!Number.isInteger(txAmountCents)) throw vatError('INVALID_AMOUNT', 'tx amount must be an integer number of cents');
  const balance = accountBalance(db, account);
  if (balance === 0) {
    throw vatError('VAT_SETTLE_NOTHING', `no outstanding balance on ${account} (af te dragen omzetbelasting) to settle`);
  }
  const owe = balance < 0; // af-te-dragen credit (negative) = te betalen
  const liability = Math.abs(balance);
  const paid = Math.abs(txAmountCents);
  if (owe && txAmountCents >= 0) {
    throw vatError('VAT_SETTLE_DIRECTION', `paying ${account} (te betalen) requires an OUTGOING bank transaction, got +${paid} cents`);
  }
  if (!owe && txAmountCents <= 0) {
    throw vatError('VAT_SETTLE_DIRECTION', `receiving a refund on ${account} (te ontvangen) requires an INCOMING bank transaction, got ${txAmountCents} cents`);
  }
  if (!getAccountByCode(db, differenceAccount)) {
    throw vatError('INVALID_DIFFERENCE_ACCOUNT', `difference account ${differenceAccount} does not exist (pick an expense account, e.g. ${tax.accounts.differenceDefault})`);
  }

  // rounding difference: +debit (loss, paid more than booked) / -credit (gain,
  // rounded in your favour). Legs: cancel the af-te-dragen balance, book bank,
  // difference -> P&L.
  const difference = (owe ? 1 : -1) * (paid - liability);
  if (Math.abs(difference) > VAT_SETTLE_MAX_DIFFERENCE_CENTS) {
    throw vatError(
      'VAT_SETTLE_DIFFERENCE_TOO_LARGE',
      `settlement difference is ${Math.abs(difference)} cents vs liability ${liability} — a VAT filing rounds per line (< €0.50/line), so this looks like the wrong amount; max allowed is ${VAT_SETTLE_MAX_DIFFERENCE_CENTS} cents`,
    );
  }
  const postings = [
    { code: account, amountCents: owe ? liability : -liability },
    { code: bankAccountCode, amountCents: txAmountCents },
  ];
  if (difference !== 0) postings.push({ code: differenceAccount, amountCents: difference });
  const description = desc ?? `Betaling OB-aangifte${period ? ` ${period}` : ''} — af te dragen omzetbelasting (afrondingsverschil ${formatAmount(difference)})`;

  if (dryRun) {
    return {
      action: 'vat.settle', dryRun: true, account, owe, liability_cents: liability, paid_cents: paid,
      difference_cents: difference, difference_account: differenceAccount, date: txDate,
      postings: postings.map((p) => ({ code: p.code, amount_cents: p.amountCents })),
    };
  }

  const entry = db.transaction(() => {
    const created = createEntry(db, {
      date: txDate ?? new Date().toISOString().slice(0, 10), description,
      postings, source: 'manual', actor,
    });
    postEntry(db, { id: created.id, actor });
    record(db, {
      actor, action: 'vat.settle', command: 'vat settle',
      args: {
        account, period, owe, liability_cents: liability, paid_cents: paid, difference_cents: difference,
        difference_account: differenceAccount, description,
      },
      outcome: 'ok', entryIds: [created.id],
    });
    return created;
  })();
  return {
    action: 'vat.settle', entry_id: entry.id, account, owe, liability_cents: liability, paid_cents: paid,
    difference_cents: difference, difference_account: differenceAccount, postings,
  };
}
