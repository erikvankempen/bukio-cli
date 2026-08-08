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
import { record } from '../audit/index.js';

export const VAT_ACCOUNTS = [
  { code: '1500', name: 'Te vorderen omzetbelasting', type: 'asset', normalBalance: 'debit', rgsCode: 'BVOR.11' },
  { code: '2500', name: 'Te betalen omzetbelasting', type: 'liability', normalBalance: 'credit', rgsCode: 'BSCH.12' },
];

export const VAT_CODES = [
  { code: '21', rateBp: 2100, type: 'standard', euReverse: 0, description: '21% hoog tarief' },
  { code: '9', rateBp: 900, type: 'standard', euReverse: 0, description: '9% laag tarief' },
  { code: '0', rateBp: 0, type: 'standard', euReverse: 0, description: '0% nultarief' },
  { code: 'V', rateBp: 0, type: 'exempt', euReverse: 0, description: 'Vrijgesteld' },
  { code: 'R', rateBp: 0, type: 'reverse', euReverse: 0, description: 'Verlegd (binnenland)' },
  { code: 'RE', rateBp: 0, type: 'reverse', euReverse: 1, description: 'Verlegd (EU)' },
  { code: 'M', rateBp: 0, type: 'margin', euReverse: 0, description: 'Marge' },
  { code: 'P', rateBp: 0, type: 'private', euReverse: 0, description: 'Privégebruik' },
];

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
  const tx = db.transaction(() => {
    db.prepare('UPDATE company SET vat_module = 1 WHERE id = 1').run();
    for (const a of VAT_ACCOUNTS) {
      if (!getAccountByCode(db, a.code)) createAccount(db, a);
    }
    const insertCode = db.prepare(
      'INSERT OR IGNORE INTO vat_codes (code, rate_bp, type, eu_reverse, description) VALUES (?, ?, ?, ?, ?)',
    );
    for (const c of VAT_CODES) insertCode.run(c.code, c.rateBp, c.type, c.euReverse, c.description);
    record(db, { actor, action: 'vat.enable', command: 'vat enable', args: {}, outcome: 'ok' });
  });
  tx();
  return { vat_module: 1, accounts: VAT_ACCOUNTS.map((a) => a.code), codes: VAT_CODES.map((c) => c.code) };
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
      const m = t.match(/^(\d{1,6}):(.+?)(?:@([A-Z0-9]+))?$/);
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
      // verlegde levering / private use is computed at the standard rate (21%).
      const effectiveRateBp = (vat.type === 'reverse' || vat.type === 'private') ? 2100 : vat.rate_bp;
      const vatAmount = Math.round(Math.abs(spec.amountCents * effectiveRateBp / 10000))
        * Math.sign(spec.amountCents);

      const isOutput = account.type === 'income' || vat.type === 'private';
      const vatAccountCode = (vat.type === 'reverse' || isOutput) ? '2500' : '1500';
      expanded.push({
        code: spec.code, amountCents: spec.amountCents,
        vatCode: vat.code, vatAmountCents: vatAmount,
        fxCurrency: spec.fxCurrency ?? null, fxAmountCents: spec.fxAmountCents ?? null,
      });
      // The VAT leg carries the SAME sign as the tagged posting:
      // sales -> credit on 2500 (te betalen), purchases -> debit on 1500 (te vorderen).
      // A 0-rate code (@V vrijgesteld, @0 nultarief) computes a 0 leg — it is
      // skipped entirely (createEntry rejects zero-amount postings); the
      // tagged posting still carries vatAmountCents 0 so the readout reports
      // the base (1c/2a).
      if (vatAmount !== 0) vatLegs.push({ code: vatAccountCode, amountCents: vatAmount });
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
export function obReadout(db, { period }) {
  requireVat(db);
  const { from, to, label } = parsePeriod(period);

  const rows = db.prepare(`
    SELECT p.amount_cents, p.vat_amount_cents, vc.code AS vat_code, vc.rate_bp, vc.type, vc.eu_reverse,
           a.type AS account_type, a.code AS account_code
    FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
    JOIN vat_codes vc ON vc.id = p.vat_code_id
    JOIN accounts a ON a.id = p.account_id
    WHERE e.date >= ? AND e.date <= ?
      AND p.vat_code_id IS NOT NULL
      AND a.code NOT IN ('1500','2500')
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
      f['1d'] += -r.amount_cents;
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
  return -(accountBalance(db, '2500') + accountBalance(db, '1500'));
}

/** Ensure the 'Af te dragen omzetbelasting' liability account exists (idempotent). */
function ensureAfTeDragenAccount(db, account) {
  if (!getAccountByCode(db, account)) {
    createAccount(db, {
      code: account, name: 'Af te dragen omzetbelasting', type: 'liability',
      normalBalance: 'credit', rgsCode: 'BSCH.12',
    });
  }
}

/**
 * Reclassify the outstanding VAT position to the af-te-dragen account at filing.
 * The move uses the EXACT booked amounts — the OB form is filed in rounded
 * whole euros, and the cent-level difference is settled to the P&L later
 * (vatSettle), never by distorting the VAT clearing accounts.
 */
export function vatFile(db, { account = VAT_FILE_ACCOUNT_DEFAULT, period = null, desc = null, actor = 'human', dryRun = false }) {
  requireVat(db);
  const bal2500 = accountBalance(db, '2500');
  const bal1500 = accountBalance(db, '1500');
  const net = -(bal2500 + bal1500); // positive = owe
  if (net === 0) {
    throw vatError('VAT_NOTHING_TO_FILE', 'no outstanding VAT position to reclassify (2500/1500 net is zero)');
  }
  // The FULL clearing position moves to 2510: both clearing accounts are
  // emptied (2500 te betalen holds the credit/output legs, 1500 te vorderen
  // the debit/input legs) and the NET lands on 'af te dragen omzetbelasting'
  // (credit when you owe, debit when you get a refund).
  const postings = [
    { code: '2500', amountCents: -bal2500 },
    { code: '1500', amountCents: -bal1500 },
    { code: account, amountCents: bal2500 + bal1500 },
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
    ensureAfTeDragenAccount(db, account);
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
  txAmountCents, txDate, bankAccountCode, differenceAccount = VAT_DIFFERENCE_ACCOUNT_DEFAULT,
  period = null, desc = null, actor = 'human', dryRun = false,
}) {
  requireVat(db);
  if (!Number.isInteger(txAmountCents)) throw vatError('INVALID_AMOUNT', 'tx amount must be an integer number of cents');
  const balance = accountBalance(db, VAT_FILE_ACCOUNT_DEFAULT);
  if (balance === 0) {
    throw vatError('VAT_SETTLE_NOTHING', `no outstanding balance on ${VAT_FILE_ACCOUNT_DEFAULT} (af te dragen omzetbelasting) to settle`);
  }
  const owe = balance < 0; // 2510 credit (negative) = te betalen
  const liability = Math.abs(balance);
  const paid = Math.abs(txAmountCents);
  if (owe && txAmountCents >= 0) {
    throw vatError('VAT_SETTLE_DIRECTION', `paying ${VAT_FILE_ACCOUNT_DEFAULT} (te betalen) requires an OUTGOING bank transaction, got +${paid} cents`);
  }
  if (!owe && txAmountCents <= 0) {
    throw vatError('VAT_SETTLE_DIRECTION', `receiving a refund on ${VAT_FILE_ACCOUNT_DEFAULT} (te ontvangen) requires an INCOMING bank transaction, got ${txAmountCents} cents`);
  }
  if (!getAccountByCode(db, differenceAccount)) {
    throw vatError('INVALID_DIFFERENCE_ACCOUNT', `difference account ${differenceAccount} does not exist (pick an expense account, e.g. ${VAT_DIFFERENCE_ACCOUNT_DEFAULT})`);
  }

  // rounding difference: +debit (loss, paid more than booked) / -credit (gain,
  // rounded in your favour). Legs: cancel 2510, book bank, difference -> P&L.
  const difference = (owe ? 1 : -1) * (paid - liability);
  if (Math.abs(difference) > VAT_SETTLE_MAX_DIFFERENCE_CENTS) {
    throw vatError(
      'VAT_SETTLE_DIFFERENCE_TOO_LARGE',
      `settlement difference is ${Math.abs(difference)} cents vs liability ${liability} — a VAT filing rounds per line (< €0.50/line), so this looks like the wrong amount; max allowed is ${VAT_SETTLE_MAX_DIFFERENCE_CENTS} cents`,
    );
  }
  const postings = [
    { code: VAT_FILE_ACCOUNT_DEFAULT, amountCents: owe ? liability : -liability },
    { code: bankAccountCode, amountCents: txAmountCents },
  ];
  if (difference !== 0) postings.push({ code: differenceAccount, amountCents: difference });
  const description = desc ?? `Betaling OB-aangifte${period ? ` ${period}` : ''} — af te dragen omzetbelasting (afrondingsverschil ${formatAmount(difference)})`;

  if (dryRun) {
    return {
      action: 'vat.settle', dryRun: true, owe, liability_cents: liability, paid_cents: paid,
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
        period, owe, liability_cents: liability, paid_cents: paid, difference_cents: difference,
        difference_account: differenceAccount, description,
      },
      outcome: 'ok', entryIds: [created.id],
    });
    return created;
  })();
  return {
    action: 'vat.settle', entry_id: entry.id, owe, liability_cents: liability, paid_cents: paid,
    difference_cents: difference, difference_account: differenceAccount, postings,
  };
}
