// Imports — opening balances, journal CSV, XML Auditfile Financieel (XAF) 4.0.
// Every importer follows the same contract:
//   * VALIDATE THE ENTIRE FILE FIRST — parse and check every row, collect ALL
//     problems, and abort with IMPORT_VALIDATION_FAILED + `details` (one entry
//     per offending line) when anything is wrong. Nothing is written.
//   * Import only when the file is clean, inside one transaction.
//   * Idempotent: entries carry source + source_ref natural keys; re-imports
//     skip boekstukken/boekstuknummers already present (reported as
//     `duplicates`), exactly like the bank import.
import { readFileSync } from 'node:fs';
import { XMLParser } from 'fast-xml-parser';
import { parseAmount, formatAmount } from '../core/money.js';
import { createAccount, getAccountByCode } from '../core/accounts.js';
import { createEntry, postEntry } from '../core/entries.js';
import { createContact, listContacts } from '../invoice/index.js';
import { record } from '../audit/index.js';

const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;
const CODE_RE = /^\d{1,6}$/;

export function importError(code, message, details = null) {
  const e = new Error(message);
  e.code = code;
  if (details) e.details = details;
  return e;
}

function validDate(s) {
  return typeof s === 'string' && DATE_RE.test(s)
    && !Number.isNaN(new Date(`${s}T00:00:00Z`).getTime());
}

function todayIso() {
  return new Date().toISOString().slice(0, 10);
}

// --- CSV plumbing -----------------------------------------------------------

function parseCsvLine(line) {
  // Dutch bookkeeping exports use ';' as delimiter exactly because amounts
  // carry decimal commas. Rule: if the line contains ';', split on ';' only;
  // otherwise split on ','. Quotes are respected in both modes.
  const delim = line.includes(';') ? ';' : ',';
  const out = [];
  let cur = '';
  let inQuotes = false;
  for (const ch of line) {
    if (ch === '"') inQuotes = !inQuotes;
    else if (ch === delim && !inQuotes) { out.push(cur); cur = ''; }
    else cur += ch;
  }
  out.push(cur);
  return out;
}

/** Split CSV text into data rows. Returns [{ line, cells }]; skips blanks. */
export function parseCsvRows(csvText) {
  const rows = [];
  const lines = String(csvText).replace(/^\uFEFF/, '').split(/\r?\n/);
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    if (line.trim() === '') continue;
    rows.push({ line: i + 1, cells: parseCsvLine(line).map((c) => c.trim()) });
  }
  return rows;
}

function normalizeHeader(h) {
  return String(h).toLowerCase().replace(/[^a-z0-9]/g, '');
}

function findColumn(header, aliases) {
  const norm = header.map(normalizeHeader);
  for (const alias of aliases) {
    const idx = norm.indexOf(normalizeHeader(alias));
    if (idx !== -1) return idx;
  }
  return -1;
}

/**
 * Parse an import amount. Imports come from Dutch bookkeeping exports, so
 * besides the strict international form we accept Dutch notation:
 *   1234.56  (international, unambiguous)
 *   1234,56  (decimal comma)
 *   1.234,56 (thousands dots + decimal comma)
 * Returns integer cents. Throws INVALID_AMOUNT for anything else.
 */
export function parseImportAmount(input) {
  if (typeof input !== 'string' || input.trim() === '') {
    throw importError('INVALID_AMOUNT', `invalid amount '${input}'`);
  }
  const s = input.trim();
  let normalized;
  if (s.includes(',') && s.includes('.')) normalized = s.replace(/\./g, '').replace(',', '.');
  else if (s.includes(',')) normalized = s.replace(',', '.');
  else normalized = s;
  if (!/^-?\d+(\.\d{1,2})?$/.test(normalized)) {
    throw importError('INVALID_AMOUNT', `invalid amount '${input}' — use e.g. 1234.56, 1234,56 or 1.234,56`);
  }
  return parseAmount(normalized);
}

/** Infer an account type+normal balance from observed net movement. */
function inferAccountType(code, netCents) {
  if (netCents < 0) return { type: 'income', normalBalance: 'credit' };
  return { type: 'expense', normalBalance: 'debit' };
}

/** XAF RekeningSoort -> account type (net movement decides asset vs liability). */
function rekeningType(soort, netCents) {
  const balans = /balans/i.test(soort ?? '');
  const credit = netCents < 0;
  if (balans) {
    return credit
      ? { type: 'liability', normalBalance: 'credit' }
      : { type: 'asset', normalBalance: 'debit' };
  }
  return credit
    ? { type: 'income', normalBalance: 'credit' }
    : { type: 'expense', normalBalance: 'debit' };
}

/** Create a missing account, inferring type from the net movement seen. */
function ensureAccount(db, code, name, netCents, createdList, typeHint = null) {
  const existing = getAccountByCode(db, code);
  if (existing) return existing;
  const { type, normalBalance } = typeHint ?? inferAccountType(code, netCents);
  const account = createAccount(db, {
    code, name: name || `Rekening ${code}`, type, normalBalance,
  });
  createdList.push({ code: account.code, name: account.name, type: account.type, normal_balance: account.normal_balance });
  return account;
}

/**
 * Chart sync for audit-file imports. On an EMPTY ledger (no postings yet) the
 * file's chart is authoritative: existing accounts with a different name are
 * renamed (+ type/balance aligned) — this is the migrate-into-bukio case where
 * the default starter chart must not hijack the file's account meanings.
 * Accounts that already carry postings are left untouched (only a warning).
 */
function syncAccountFromFile(db, code, name, typeHint, createdList, updatedList, warnings) {
  const existing = getAccountByCode(db, code);
  const cleanName = String(name ?? '').trim() || `Rekening ${code}`;
  if (!existing) {
    const { type, normalBalance } = typeHint ?? inferAccountType(code, 0);
    const account = createAccount(db, { code, name: cleanName, type, normalBalance });
    createdList.push({ code: account.code, name: account.name, type: account.type, normal_balance: account.normal_balance });
    return;
  }
  const nameDiffers = String(existing.name).trim().toLowerCase() !== cleanName.toLowerCase();
  if (!nameDiffers) return;
  const used = db.prepare('SELECT 1 FROM postings WHERE account_id = ? LIMIT 1').get(existing.id);
  if (used) {
    warnings.push(`account ${code} stays '${existing.name}' — it already has postings (file calls it '${cleanName}')`);
    return;
  }
  const { type, normalBalance } = typeHint ?? { type: existing.type, normal_balance: existing.normal_balance };
  const before = existing.name;
  db.prepare('UPDATE accounts SET name = ?, type = ?, normal_balance = ? WHERE id = ?')
    .run(cleanName, type, normalBalance, existing.id);
  updatedList.push({ code, from: before, to: cleanName, type, normal_balance: normalBalance });
}

// --- opening balances -------------------------------------------------------

/**
 * Import opening balances from CSV. Accepted layouts (optional header row):
 *   code,amount        amount signed: positive = debet, negative = credit
 *   code,debet,credit  Dutch style: exactly one of debet/credit filled
 * Validates the whole file first (accounts exist & active, amounts parse,
 * sum == 0) and aborts with a full error list otherwise. Creates ONE posted
 * entry (source 'import', source_ref 'opening-balances'); re-importing is
 * rejected with OPENING_ALREADY_IMPORTED.
 */
export function importOpeningBalances(db, { csvText, date = null, actor = 'human', dryRun = false }) {
  const theDate = date || todayIso();
  if (!validDate(theDate)) throw importError('INVALID_DATE', `date '${theDate}' must be yyyy-mm-dd`);

  const existing = db.prepare(
    "SELECT id FROM journal_entries WHERE source = 'import' AND source_ref = 'opening-balances'",
  ).get();
  if (existing) {
    throw importError('OPENING_ALREADY_IMPORTED', `opening balances were already imported (entry ${existing.id})`);
  }

  const rows = parseCsvRows(csvText);
  if (rows.length === 0) throw importError('EMPTY_CSV', 'opening-balances CSV has no data rows');

  const errors = [];
  const specs = [];
  for (const r of rows) {
    const cells = r.cells; // keep empties: layout is decided by column count
    let code;
    let amount;
    if (cells.length === 2) {
      [code, amount] = cells;
    } else if (cells.length === 3) {
      const [c, debet, credit] = cells;
      code = c;
      if (debet && credit) {
        errors.push({ line: r.line, error: 'INVALID_ROW: both debet and credit are filled on one line' });
        continue;
      }
      amount = debet || (credit ? `-${credit}` : '');
    } else {
      errors.push({
        line: r.line,
        error: `INVALID_ROW: expected "code,amount" or "code,debet,credit" (got ${cells.length} columns)`,
      });
      continue;
    }
    if (!CODE_RE.test(code)) {
      errors.push({ line: r.line, error: `INVALID_CODE: '${code}' must be 1-6 digits` });
      continue;
    }
    let amountCents;
    try {
      amountCents = parseImportAmount(amount);
    } catch (err) {
      errors.push({ line: r.line, error: `${err.code}: ${err.message}` });
      continue;
    }
    if (amountCents === 0) {
      errors.push({ line: r.line, error: 'INVALID_AMOUNT: amount must be non-zero' });
      continue;
    }
    const account = getAccountByCode(db, code);
    if (!account) {
      errors.push({ line: r.line, error: `ACCOUNT_NOT_FOUND: account ${code} does not exist (create it first or import the chart)` });
      continue;
    }
    if (!account.active) {
      errors.push({ line: r.line, error: `ACCOUNT_INACTIVE: account ${code} is inactive` });
      continue;
    }
    specs.push({ code, amountCents });
  }

  const sum = specs.reduce((acc, s) => acc + s.amountCents, 0);
  if (sum !== 0) {
    errors.push({
      line: 0,
      error: `UNBALANCED: opening balances sum to ${formatAmount(sum)} — debet must equal credit`,
    });
  }
  if (specs.length < 2 && errors.length === 0) {
    errors.push({ line: 0, error: 'TOO_FEW_POSTINGS: opening balances need at least 2 accounts' });
  }
  if (errors.length > 0) {
    throw importError(
      'IMPORT_VALIDATION_FAILED',
      `opening-balances file has ${errors.length} problem(s) — nothing imported`,
      errors,
    );
  }

  const totalDebit = specs.filter((s) => s.amountCents > 0).reduce((a, s) => a + s.amountCents, 0);
  const plan = {
    action: 'import opening balances',
    date: theDate,
    accounts: specs.length,
    total_debit_cents: totalDebit,
    total_credit_cents: -sum + totalDebit,
    dryRun: true,
  };
  if (dryRun) return plan;

  const description = 'Beginbalans';
  const entry = createEntry(db, {
    date: theDate, description,
    postings: specs.map((s) => ({ code: s.code, amountCents: s.amountCents })),
    source: 'import', sourceRef: 'opening-balances', actor,
  });
  const posted = postEntry(db, { id: entry.id, actor });
  record(db, {
    actor, action: 'import.opening-balances', command: 'import opening-balances',
    args: { date: theDate, accounts: specs.length, total_debit_cents: totalDebit },
    outcome: 'ok', entryIds: [entry.id],
  });
  return {
    entry: { id: posted.id, date: posted.date, description: posted.description, state: posted.state },
    accounts: specs.length,
    total_debit_cents: totalDebit,
    total_credit_cents: totalDebit,
    dryRun: false,
  };
}

// --- journal CSV ------------------------------------------------------------

const JOURNAL_COLUMNS = {
  date: ['datum', 'date', 'boekdatum', 'boekingsdatum', 'transactiedatum'],
  boekstuk: ['boekstuknummer', 'boekstuknr', 'boekstuk', 'document', 'nummer', 'dagboeknummer'],
  rekening: ['rekening', 'rekeningcode', 'rekeningnummer', 'account', 'code', 'grootboekrekening'],
  tegenrekening: ['tegenrekening', 'tegenrekeningcode', 'tegenrekeningnummer', 'contra', 'contracode'],
  bedrag: ['bedrag', 'amount', 'bedraginclbtw', 'bedragincl', 'amountinclvat', 'totaal'],
  omschrijving: ['omschrijving', 'description', 'tekst', 'memorie'],
  btwcode: ['btwcode', 'btw', 'btw-code', 'vatcode', 'btwcodeomschrijving'],
};

/**
 * Import a journal from CSV (SnelStart/Exact-style export layout). Header row
 * required; column names are matched by alias. Each data row is one boeking
 * line: +bedrag on rekening, -bedrag on tegenrekening. Rows sharing a
 * boekstuknummer become ONE entry (date = the boekstuk's date).
 *
 * Validation (whole file, before anything is written): header complete,
 * every date valid, every amount parses and is non-zero, both accounts exist
 * & active (or are created when createMissing is set), one date per boekstuk.
 * BtwCode columns are accepted but NOT booked — the import is net; distinct
 * codes are reported in `ignored_btw_codes`.
 */
export function importJournalCsv(db, { csvText, createMissing = false, actor = 'human', dryRun = false }) {
  const rows = parseCsvRows(csvText);
  if (rows.length === 0) throw importError('EMPTY_CSV', 'journal CSV has no data rows');

  const header = rows[0].cells;
  const col = {};
  for (const [key, aliases] of Object.entries(JOURNAL_COLUMNS)) {
    col[key] = findColumn(header, aliases);
  }
  const required = ['date', 'boekstuk', 'rekening', 'tegenrekening', 'bedrag'];
  const missing = required.filter((k) => col[k] === -1);
  if (missing.length > 0) {
    throw importError(
      'INVALID_CSV_HEADER',
      `journal CSV is missing column(s): ${missing.join(', ')} (got header: ${header.join(',')})`,
    );
  }

  const errors = [];
  const parsed = []; // { line, date, boekstuk, rekening, tegenrekening, bedragCents, omschrijving, btwcode }
  const btwCodes = new Set();
  for (const r of rows.slice(1)) {
    const cells = r.cells;
    const get = (k) => (col[k] !== -1 ? (cells[col[k]] ?? '') : '');
    const date = get('date');
    const boekstuk = get('boekstuk');
    const rekening = get('rekening');
    const tegenrekening = get('tegenrekening');
    const bedrag = get('bedrag');
    if (!date && !boekstuk && !rekening && !tegenrekening && !bedrag) continue; // blank line

    if (!validDate(date)) { errors.push({ line: r.line, error: `INVALID_DATE: '${date}' must be yyyy-mm-dd` }); }
    if (!boekstuk) { errors.push({ line: r.line, error: 'BOEKSTUK_REQUIRED: every row needs a boekstuknummer' }); }
    if (!CODE_RE.test(rekening)) { errors.push({ line: r.line, error: `INVALID_CODE: rekening '${rekening}' must be 1-6 digits` }); }
    if (!CODE_RE.test(tegenrekening)) { errors.push({ line: r.line, error: `INVALID_CODE: tegenrekening '${tegenrekening}' must be 1-6 digits` }); }
    let bedragCents = null;
    try { bedragCents = parseImportAmount(bedrag); } catch (err) { errors.push({ line: r.line, error: `${err.code}: ${err.message}` }); }
    if (bedragCents === 0) { errors.push({ line: r.line, error: 'INVALID_AMOUNT: bedrag must be non-zero' }); }
    if (bedragCents == null) bedragCents = 0;

    for (const code of [rekening, tegenrekening]) {
      if (!CODE_RE.test(code)) continue;
      const account = getAccountByCode(db, code);
      if (!account && !createMissing) {
        errors.push({ line: r.line, error: `ACCOUNT_NOT_FOUND: account ${code} does not exist (use --create-missing to create it)` });
      } else if (account && !account.active) {
        errors.push({ line: r.line, error: `ACCOUNT_INACTIVE: account ${code} is inactive` });
      }
    }

    const omschrijving = get('omschrijving');
    const btwcode = get('btwcode');
    if (btwcode) btwCodes.add(btwcode);
    parsed.push({
      line: r.line, date, boekstuk, rekening, tegenrekening, bedragCents,
      omschrijving, btwcode,
    });
  }
  if (parsed.length === 0) throw importError('EMPTY_CSV', 'journal CSV has no data rows after the header');

  // one date per boekstuk
  const byBoekstuk = new Map();
  for (const p of parsed) {
    if (!byBoekstuk.has(p.boekstuk)) byBoekstuk.set(p.boekstuk, new Set());
    byBoekstuk.get(p.boekstuk).add(p.date);
  }
  for (const [boekstuk, dates] of byBoekstuk) {
    if (dates.size > 1) {
      errors.push({ line: 0, error: `DATE_MISMATCH: boekstuk '${boekstuk}' has rows on different dates (${[...dates].join(', ')})` });
    }
  }

  if (errors.length > 0) {
    throw importError(
      'IMPORT_VALIDATION_FAILED',
      `journal file has ${errors.length} problem(s) — nothing imported`,
      errors,
    );
  }

  // net movement per account for type inference of created accounts
  const net = new Map();
  for (const p of parsed) {
    net.set(p.rekening, (net.get(p.rekening) ?? 0) + p.bedragCents);
    net.set(p.tegenrekening, (net.get(p.tegenrekening) ?? 0) - p.bedragCents);
  }

  const groups = [...byBoekstuk.keys()]; // first-seen order
  const bookstukCount = (s) => parsed.filter((p) => p.boekstuk === s).length;
  const firstOf = (s) => parsed.find((p) => p.boekstuk === s);
  const existingRefs = new Set(
    db.prepare("SELECT source_ref FROM journal_entries WHERE source = 'import' AND source_ref IS NOT NULL").all()
      .map((r) => r.source_ref),
  );

  const accountsCreated = [];
  const plan = {
    action: 'import journal',
    boekstukken: groups.length,
    lines: parsed.length,
    entries: groups.map((s) => ({ boekstuk: s, date: firstOf(s).date, lines: bookstukCount(s) })),
    create_missing: createMissing,
    ignored_btw_codes: [...btwCodes],
    duplicates: groups.filter((s) => existingRefs.has(`journal:${s}`)).length,
    dryRun: true,
  };
  if (dryRun) return plan;

  const imported = [];
  const duplicates = [];
  const tx = db.transaction(() => {
    for (const s of groups) {
      const ref = `journal:${s}`;
      if (existingRefs.has(ref)) { duplicates.push(s); continue; }
      const lines = parsed.filter((p) => p.boekstuk === s);
      if (createMissing) {
        for (const code of new Set(lines.flatMap((p) => [p.rekening, p.tegenrekening]))) {
          if (!getAccountByCode(db, code)) {
            ensureAccount(db, code, `Rekening ${code}`, net.get(code) ?? 0, accountsCreated);
          }
        }
      }
      const postings = lines.flatMap((p) => [
        { code: p.rekening, amountCents: p.bedragCents },
        { code: p.tegenrekening, amountCents: -p.bedragCents },
      ]);
      const description = lines.find((p) => p.omschrijving)?.omschrijving || `Boekstuk ${s}`;
      const entry = createEntry(db, {
        date: lines[0].date, description, postings,
        source: 'import', sourceRef: ref, actor,
      });
      const posted = postEntry(db, { id: entry.id, actor });
      imported.push({ id: posted.id, date: posted.date, description: posted.description, boekstuk: s });
    }
  });
  tx();

  record(db, {
    actor, action: 'import.journal', command: 'import journal',
    args: { boekstukken: imported.length, duplicates: duplicates.length, create_missing: createMissing },
    outcome: 'ok', entryIds: imported.map((e) => e.id),
  });
  return {
    imported: imported.length,
    duplicates: duplicates.length,
    entries: imported,
    accounts_created: accountsCreated,
    ignored_btw_codes: [...btwCodes],
    dryRun: false,
  };
}

// --- XAF 4.0 ----------------------------------------------------------------

function asArray(v) {
  if (v == null) return [];
  return Array.isArray(v) ? v : [v];
}

/**
 * Import an XML Auditfile Financieel 4.0 (Belastingdienst audit format).
 * Structure: XafHeader (version, company, fiscal year), Rekeningen (the
 * file's own chart — accounts missing from the bukio chart are created with a
 * sign-inferred type), Mutaties (one entry per Mutatie, bookstuknummer as
 * source_ref, +bedrag on rekening / -bedrag on tegenrekening per Boeking).
 * Whole-file validation first; idempotent per boekstuknummer; BtwCodes are
 * reported but not booked (the import is net).
 */
export function importXaf(db, { xmlText, actor = 'human', dryRun = false }) {
  let doc;
  try {
    doc = new XMLParser({ parseTagValue: false, trimValues: true }).parse(xmlText);
  } catch (err) {
    throw importError('INVALID_XAF', `cannot parse XML: ${err.message}`);
  }
  const xaf = doc.Xaf ?? doc.XAF;
  if (!xaf && doc.AuditFile) {
    return importAuditFileLayout(db, { auditFile: doc.AuditFile, actor, dryRun });
  }
  if (!xaf) throw importError('INVALID_XAF', 'root element must be <Xaf> (Belastingdienst) or <AuditFile> (general-ledger export)');

  const header = xaf.XafHeader ?? {};
  const version = String(header.Version ?? '');
  if (!/^4(\.\d+)?$/.test(version)) {
    throw importError('INVALID_XAF', `unsupported audit file version '${version || '(missing)'}' — expected 4.0`);
  }

  // company cross-check: a differing KVK means the file belongs to another
  // company — refuse. Name differences are only a warning.
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get() ?? null;
  const fileKvk = header.CompanyID ? String(header.CompanyID).trim() : null;
  const fileName = header.CompanyName ? String(header.CompanyName).trim() : null;
  const companyMismatch = [];
  if (company) {
    if (fileKvk && company.kvk && fileKvk !== company.kvk) {
      throw importError('COMPANY_MISMATCH', `audit file is for ${fileKvk} (${fileName ?? 'unknown'}), database is for ${company.kvk} (${company.name})`);
    }
    if (fileKvk && !company.kvk) {
      companyMismatch.push(`database has no KVK; file is for ${fileKvk}`);
    }
    if (fileName && company.name && fileName !== company.name) {
      companyMismatch.push(`company name differs: file '${fileName}' vs database '${company.name}'`);
    }
  }

  const rekeningen = asArray(xaf.Rekeningen?.Rekening);
  const mutaties = asArray(xaf.Mutaties?.Mutatie);

  const errors = [];
  const seenCodes = new Set();
  for (const r of rekeningen) {
    const code = String(r.RekeningCode ?? '').trim();
    if (!CODE_RE.test(code)) errors.push({ line: 0, error: `INVALID_CODE: rekening code '${code}' must be 1-6 digits` });
    else if (seenCodes.has(code)) errors.push({ line: 0, error: `DUPLICATE_CODE: rekening ${code} appears twice in the audit file` });
    else seenCodes.add(code);
  }

  const parsedMutaties = [];
  const btwCodes = new Set();
  for (const m of mutaties) {
    const boekstuk = String(m.Boekstuknummer ?? '').trim();
    const date = String(m.Datum ?? m.Factuurdatum ?? '').trim();
    const boekingen = asArray(m.Boekingen?.Boeking);
    if (!boekstuk) errors.push({ line: 0, error: 'BOEKSTUK_REQUIRED: every <Mutatie> needs a <Boekstuknummer>' });
    if (!validDate(date)) errors.push({ line: 0, error: `INVALID_DATE: mutatie '${boekstuk || '?'}' date '${date}' must be yyyy-mm-dd` });
    if (boekingen.length === 0) errors.push({ line: 0, error: `NO_BOEKINGEN: mutatie '${boekstuk || '?'}' has no <Boeking> rows` });

    const postings = [];
    for (const b of boekingen) {
      const rekening = String(b.RekeningCode ?? '').trim();
      const tegenrekening = String(b.TegenrekeningCode ?? '').trim();
      const bedrag = String(b.Bedrag ?? '').trim();
      let bedragCents = null;
      try { bedragCents = parseImportAmount(bedrag); } catch (err) { errors.push({ line: 0, error: `${err.code}: mutatie '${boekstuk || '?'}' bedrag '${bedrag}'` }); }
      if (bedragCents === 0) errors.push({ line: 0, error: `INVALID_AMOUNT: mutatie '${boekstuk || '?'}' bedrag must be non-zero` });
      for (const code of [rekening, tegenrekening]) {
        if (!CODE_RE.test(code)) {
          errors.push({ line: 0, error: `INVALID_CODE: '${code}' must be 1-6 digits (mutatie '${boekstuk || '?'}')` });
        } else if (!seenCodes.has(code) && !getAccountByCode(db, code)) {
          errors.push({ line: 0, error: `RECORDING_NOT_FOUND: rekening ${code} (mutatie '${boekstuk || '?'}') is not in <Rekeningen> nor in the chart` });
        }
      }
      const btwcode = String(b.BtwCode ?? '').trim();
      if (btwcode) btwCodes.add(btwcode);
      postings.push({
        rekening, tegenrekening,
        bedragCents: bedragCents == null ? 0 : bedragCents,
        omschrijving: String(b.Omschrijving ?? '').trim(),
      });
    }
    parsedMutaties.push({ boekstuk, date, postings, rekeningSoort: {} });
  }

  if (errors.length > 0) {
    throw importError(
      'IMPORT_VALIDATION_FAILED',
      `XAF file has ${errors.length} problem(s) — nothing imported`,
      errors,
    );
  }

  // net movement per code (file chart + used codes) for type inference
  const net = new Map();
  for (const r of rekeningen) net.set(String(r.RekeningCode).trim(), 0);
  for (const m of parsedMutaties) {
    for (const p of m.postings) {
      net.set(p.rekening, (net.get(p.rekening) ?? 0) + p.bedragCents);
      net.set(p.tegenrekening, (net.get(p.tegenrekening) ?? 0) - p.bedragCents);
    }
  }

  const existingRefs = new Set(
    db.prepare("SELECT source_ref FROM journal_entries WHERE source = 'xaf' AND source_ref IS NOT NULL").all()
      .map((r) => r.source_ref),
  );
  const accountsCreated = [];
  const accountsUpdated = [];
  const syncWarnings = [];
  const plan = {
    action: 'import xaf',
    company: { name: fileName, kvk: fileKvk, fiscal_year: header.FiscalYear ?? null, period: `${header.StartDate ?? ''}..${header.EndDate ?? ''}` },
    software: header.SoftwareName ? `${header.SoftwareName} ${header.SoftwareVersion ?? ''}`.trim() : null,
    rekeningen: rekeningen.length,
    mutaties: parsedMutaties.length,
    duplicates: parsedMutaties.filter((m) => existingRefs.has(m.boekstuk)).length,
    accounts_to_create: rekeningen.filter((r) => !getAccountByCode(db, String(r.RekeningCode).trim())).length,
    accounts_to_rename: rekeningen
      .map((r) => ({ code: String(r.RekeningCode ?? '').trim(), name: String(r.RekeningOmschrijving ?? '').trim() }))
      .filter(({ code, name }) => {
        const existing = getAccountByCode(db, code);
        return existing && name && String(existing.name).trim().toLowerCase() !== name.toLowerCase();
      }),
    company_mismatch: companyMismatch,
    ignored_btw_codes: [...btwCodes],
    dryRun: true,
  };
  if (dryRun) return plan;

  const imported = [];
  const duplicates = [];
  const tx = db.transaction(() => {
    // upsert the file's chart
    for (const r of rekeningen) {
      const code = String(r.RekeningCode).trim();
      const netCents = net.get(code) ?? 0;
      syncAccountFromFile(
        db, code,
        String(r.RekeningOmschrijving ?? '').trim() || `Rekening ${code}`,
        rekeningType(r.RekeningSoort, netCents),
        accountsCreated, accountsUpdated, syncWarnings,
      );
    }
    for (const m of parsedMutaties) {
      if (existingRefs.has(m.boekstuk)) { duplicates.push(m.boekstuk); continue; }
      const postings = m.postings.flatMap((p) => [
        { code: p.rekening, amountCents: p.bedragCents },
        { code: p.tegenrekening, amountCents: -p.bedragCents },
      ]);
      const description = m.postings.find((p) => p.omschrijving)?.omschrijving || `XAF ${m.boekstuk}`;
      const entry = createEntry(db, {
        date: m.date, description, postings,
        source: 'xaf', sourceRef: m.boekstuk, actor,
      });
      const posted = postEntry(db, { id: entry.id, actor });
      imported.push({ id: posted.id, date: posted.date, description: posted.description, boekstuk: m.boekstuk });
    }
  });
  tx();

  record(db, {
    actor, action: 'import.xaf', command: 'import xaf',
    args: { mutaties: imported.length, duplicates: duplicates.length, accounts_created: accountsCreated.length, accounts_updated: accountsUpdated.length },
    outcome: 'ok', entryIds: imported.map((e) => e.id),
  });
  return {
    imported: imported.length,
    duplicates: duplicates.length,
    entries: imported,
    accounts_created: accountsCreated,
    accounts_updated: accountsUpdated,
    chart_warnings: syncWarnings,
    header: {
      company_name: fileName, company_kvk: fileKvk, fiscal_year: header.FiscalYear ?? null,
      start_date: header.StartDate ?? null, end_date: header.EndDate ?? null,
      software: header.SoftwareName ? `${header.SoftwareName} ${header.SoftwareVersion ?? ''}`.trim() : null,
    },
    company_mismatch: companyMismatch,
    ignored_btw_codes: [...btwCodes],
    dryRun: false,
  };
}

export function readImportFile(filePath) {
  return readFileSync(filePath, 'utf8');
}

// --- contacts from audit files (Suppliers/Customers) ------------------------

/**
 * Import suppliers + customers from an audit file (XAF 4.0, either layout)
 * as invoice contacts. Whole-file validation first (every entry needs a
 * name); idempotent by name — contacts that already exist (case-insensitive)
 * or appear twice in the file are skipped as duplicates.
 */
export function importContacts(db, { xmlText, actor = 'human', dryRun = false }) {
  let doc;
  try {
    doc = new XMLParser({ parseTagValue: false, trimValues: true }).parse(xmlText);
  } catch (err) {
    throw importError('INVALID_XAF', `cannot parse XML: ${err.message}`);
  }
  const root = doc.Xaf ?? doc.XAF ?? doc.AuditFile;
  if (!root) throw importError('INVALID_XAF', 'root element must be <Xaf> or <AuditFile>');

  const mf = root.MasterFiles ?? root;
  const suppliers = asArray(mf.Suppliers?.Supplier);
  const customers = asArray(mf.Customers?.Customer);
  const entries = [
    ...suppliers.map((s, i) => ({ kind: 'supplier', id: String(s.SupplierID ?? '').trim(), raw: s, order: i })),
    ...customers.map((c, i) => ({ kind: 'customer', id: String(c.CustomerID ?? '').trim(), raw: c, order: i })),
  ];

  const errors = [];
  const contacts = [];
  for (const e of entries) {
    const r = e.raw;
    const name = String(r.CompanyName ?? '').trim() || String(r.Contact ?? '').trim();
    if (!name) {
      errors.push({ line: 0, error: `CONTACT_REQUIRED: ${e.kind} '${e.id || '?'}' has no CompanyName/Contact` });
      continue;
    }
    const street = String(r.Address?.StreetName ?? '').trim();
    const extra = String(r.Address?.AdditionalAddressDetail ?? '').trim();
    const address = extra ? `${street}, ${extra}` : street;
    contacts.push({
      kind: e.kind,
      name,
      address: address || null,
      postalCode: String(r.Address?.PostalCode ?? '').trim() || null,
      city: String(r.Address?.City ?? '').trim() || null,
      country: String(r.Address?.Country ?? '').trim().toUpperCase() || 'NL',
      email: String(r.Email ?? '').trim() || null,
      vatId: String(r.TaxRegistrationNumber ?? '').trim() || null,
    });
  }
  if (errors.length > 0) {
    throw importError(
      'IMPORT_VALIDATION_FAILED',
      `audit file has ${errors.length} problem(s) — nothing imported`,
      errors,
    );
  }

  const existingNames = new Set(listContacts(db).map((c) => c.name.toLowerCase()));
  const seen = new Set();
  const fresh = contacts.filter((c) => {
    const key = c.name.toLowerCase();
    if (seen.has(key) || existingNames.has(key)) return false;
    seen.add(key);
    return true;
  });
  const duplicates = contacts.length - fresh.length;

  if (dryRun) {
    return {
      suppliers: suppliers.length, customers: customers.length,
      contacts: contacts.length, contacts_to_create: fresh.length,
      duplicates, dryRun: true,
    };
  }

  const imported = [];
  for (const c of fresh) {
    const contact = createContact(db, { ...c, actor });
    imported.push({ id: contact.id, name: contact.name, kind: c.kind });
  }
  record(db, {
    actor, action: 'import.contacts', command: 'import contacts',
    args: { imported: imported.length, duplicates, suppliers: suppliers.length, customers: customers.length },
    outcome: 'ok',
  });
  return {
    imported: imported.length, duplicates, contacts: imported,
    suppliers: suppliers.length, customers: customers.length, dryRun: false,
  };
}

// --- XAF 4.0 AuditFile layout (root <AuditFile>) ------------------------------
//
// The generic audit-file layout used by general-ledger software exports:
//   Header            (AuditFileVersion, CompanyName, CompanyID, FiscalYear,
//                      StartDate, EndDate, CurrencyCode, SoftwareDescription)
//   MasterFiles/GeneralLedgerAccounts/Account   (AccountID, AccountDescription,
//                      AccountType: Asset|Liability|Equity|Revenue|Expense)
//   GeneralLedgerEntries/Journal[]/Transaction[]  (TransactionID,
//                      TransactionDate, Description; Line[] with AccountID +
//                      DebitAmount | CreditAmount, TaxInformation, ...)
// Each Line is a complete posting on one account (no tegenrekening); the entry
// is the sum of its lines. Tax codes (e.g. NOVAT) are reported, not booked.

const AUDITFILE_ACCOUNT_TYPES = {
  asset: { type: 'asset', normalBalance: 'debit' },
  liability: { type: 'liability', normalBalance: 'credit' },
  equity: { type: 'equity', normalBalance: 'credit' },
  revenue: { type: 'income', normalBalance: 'credit' },
  expense: { type: 'expense', normalBalance: 'debit' },
};

function importAuditFileLayout(db, { auditFile, actor, dryRun }) {
  const header = auditFile.Header ?? {};
  const version = String(header.AuditFileVersion ?? '');
  if (!/^4(\.\d+)?$/.test(version)) {
    throw importError('INVALID_XAF', `unsupported audit file version '${version || '(missing)'}' — expected 4.0`);
  }

  // company cross-check: only an 8-digit CompanyID is a plausible KVK; other
  // ids (e.g. the exporting database's row id) fall back to the name check.
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get() ?? null;
  const fileKvk = header.CompanyID ? String(header.CompanyID).trim() : null;
  const fileName = String(header.CompanyName ?? header.BusinessName ?? '').trim() || null;
  const currency = String(header.CurrencyCode ?? 'EUR').trim();
  const companyMismatch = [];
  if (company) {
    if (fileKvk && /^\d{8}$/.test(fileKvk) && company.kvk && fileKvk !== company.kvk) {
      throw importError('COMPANY_MISMATCH', `audit file is for ${fileKvk} (${fileName ?? 'unknown'}), database is for ${company.kvk} (${company.name})`);
    }
    if (fileName && company.name && fileName.toLowerCase() !== company.name.toLowerCase()) {
      companyMismatch.push(`company name differs: file '${fileName}' vs database '${company.name}'`);
    }
  }
  if (currency !== 'EUR') {
    companyMismatch.push(`currency ${currency} — imported amounts are booked as EUR`);
  }

  const accounts = asArray(auditFile.MasterFiles?.GeneralLedgerAccounts?.Account);
  const journals = asArray(auditFile.GeneralLedgerEntries?.Journal);
  const transactions = journals.flatMap((j) => asArray(j.Transaction));

  const errors = [];
  const seenCodes = new Set();
  const accountTypeByCode = new Map();
  for (const a of accounts) {
    const code = String(a.AccountID ?? '').trim();
    if (!CODE_RE.test(code)) errors.push({ line: 0, error: `INVALID_CODE: account '${code}' must be 1-6 digits` });
    else if (seenCodes.has(code)) errors.push({ line: 0, error: `DUPLICATE_CODE: account ${code} appears twice in the audit file` });
    else seenCodes.add(code);
    const t = AUDITFILE_ACCOUNT_TYPES[String(a.AccountType ?? '').toLowerCase().trim()];
    if (t) accountTypeByCode.set(code, t);
  }

  const parsed = [];
  const taxCodes = new Set();
  for (const t of transactions) {
    const id = String(t.TransactionID ?? '').trim();
    const date = String(t.TransactionDate ?? '').trim();
    const lines = asArray(t.Line);
    if (!id) errors.push({ line: 0, error: 'TRANSACTION_REQUIRED: every <Transaction> needs a <TransactionID>' });
    if (!validDate(date)) errors.push({ line: 0, error: `INVALID_DATE: transaction '${id || '?'}' date '${date}' must be yyyy-mm-dd` });
    if (lines.length === 0) errors.push({ line: 0, error: `NO_LINES: transaction '${id || '?'}' has no <Line> rows` });

    const postings = [];
    let sum = 0;
    for (const ln of lines) {
      const code = String(ln.AccountID ?? '').trim();
      const debit = String(ln.DebitAmount ?? '').trim();
      const credit = String(ln.CreditAmount ?? '').trim();
      if (!CODE_RE.test(code)) {
        errors.push({ line: 0, error: `INVALID_CODE: '${code}' must be 1-6 digits (transaction '${id || '?'}')` });
      } else if (!seenCodes.has(code) && !getAccountByCode(db, code)) {
        errors.push({ line: 0, error: `RECORDING_NOT_FOUND: account ${code} (transaction '${id || '?'}') is not in the file's accounts nor in the chart` });
      }
      let amountCents = null;
      if (debit && credit) {
        errors.push({ line: 0, error: `INVALID_AMOUNT: transaction '${id || '?'}' line ${code} has both DebitAmount and CreditAmount` });
      } else if (!debit && !credit) {
        errors.push({ line: 0, error: `INVALID_AMOUNT: transaction '${id || '?'}' line ${code} has no DebitAmount/CreditAmount` });
      } else {
        try {
          amountCents = debit ? parseImportAmount(debit) : -parseImportAmount(credit);
        } catch (err) {
          errors.push({ line: 0, error: `${err.code}: transaction '${id || '?'}' line ${code} amount '${debit || credit}'` });
        }
      }
      if (amountCents === 0) errors.push({ line: 0, error: `INVALID_AMOUNT: transaction '${id || '?'}' line ${code} must be non-zero` });
      const taxInfo = Array.isArray(ln.TaxInformation) ? ln.TaxInformation[0] : ln.TaxInformation;
      const taxCode = String(taxInfo?.TaxCode ?? '').trim();
      if (taxCode) taxCodes.add(taxCode);
      sum += amountCents ?? 0;
      postings.push({ code, amountCents: amountCents ?? 0 });
    }
    if (sum !== 0) {
      errors.push({ line: 0, error: `UNBALANCED: transaction '${id || '?'}' lines sum to ${formatAmount(sum)} — debet must equal credit` });
    }
    const description = String(t.Description ?? lines[0]?.Description ?? '').trim() || `Boekstuk ${id || '?'}`;
    parsed.push({ id, date, description, postings });
  }

  if (errors.length > 0) {
    throw importError(
      'IMPORT_VALIDATION_FAILED',
      `XAF file has ${errors.length} problem(s) — nothing imported`,
      errors,
    );
  }

  const existingRefs = new Set(
    db.prepare("SELECT source_ref FROM journal_entries WHERE source = 'xaf' AND source_ref IS NOT NULL").all()
      .map((r) => r.source_ref),
  );
  const plan = {
    action: 'import xaf',
    company: {
      name: fileName, kvk: fileKvk, fiscal_year: header.FiscalYear ?? null,
      period: `${header.StartDate ?? ''}..${header.EndDate ?? ''}`,
    },
    software: header.SoftwareDescription ? String(header.SoftwareDescription).trim() : null,
    rekeningen: accounts.length,
    mutaties: parsed.length,
    duplicates: parsed.filter((p) => existingRefs.has(p.id)).length,
    accounts_to_create: accounts.filter((a) => !getAccountByCode(db, String(a.AccountID ?? '').trim())).length,
    accounts_to_rename: accounts
      .map((a) => ({ code: String(a.AccountID ?? '').trim(), name: String(a.AccountDescription ?? '').trim() }))
      .filter(({ code, name }) => {
        const existing = getAccountByCode(db, code);
        return existing && name && String(existing.name).trim().toLowerCase() !== name.toLowerCase();
      }),
    company_mismatch: companyMismatch,
    ignored_btw_codes: [...taxCodes],
    dryRun: true,
  };
  if (dryRun) return plan;

  // net movement per code (file chart + used codes) for type inference
  const net = new Map();
  for (const a of accounts) net.set(String(a.AccountID).trim(), 0);
  for (const p of parsed) {
    for (const po of p.postings) net.set(po.code, (net.get(po.code) ?? 0) + po.amountCents);
  }

  const accountsCreated = [];
  const accountsUpdated = [];
  const syncWarnings = [];
  const imported = [];
  let duplicates = 0;
  const tx = db.transaction(() => {
    for (const a of accounts) {
      const code = String(a.AccountID).trim();
      const netCents = net.get(code) ?? 0;
      syncAccountFromFile(
        db, code,
        String(a.AccountDescription ?? '').trim() || `Rekening ${code}`,
        accountTypeByCode.get(code) ?? inferAccountType(code, netCents),
        accountsCreated, accountsUpdated, syncWarnings,
      );
    }
    for (const p of parsed) {
      if (existingRefs.has(p.id)) { duplicates += 1; continue; }
      const entry = createEntry(db, {
        date: p.date, description: p.description, postings: p.postings,
        source: 'xaf', sourceRef: p.id, actor,
      });
      const posted = postEntry(db, { id: entry.id, actor });
      existingRefs.add(p.id);
      imported.push({ id: posted.id, date: posted.date, description: posted.description, state: posted.state, source_ref: p.id });
    }
  });
  tx();

  record(db, {
    actor, action: 'import.xaf', command: 'import xaf',
    args: { transactions: parsed.length, imported: imported.length, duplicates, accounts_created: accountsCreated.length, accounts_updated: accountsUpdated.length, layout: 'auditfile' },
    outcome: 'ok', entryIds: imported.map((e) => e.id),
  });
  return {
    imported: imported.length,
    duplicates,
    entries: imported,
    accounts_created: accountsCreated,
    accounts_updated: accountsUpdated,
    chart_warnings: syncWarnings,
    header: {
      company_name: fileName, company_kvk: fileKvk, fiscal_year: header.FiscalYear ?? null,
      start_date: header.StartDate ?? null, end_date: header.EndDate ?? null,
      software: header.SoftwareDescription ? String(header.SoftwareDescription).trim() : null,
    },
    company_mismatch: companyMismatch,
    ignored_btw_codes: [...taxCodes],
    dryRun: false,
  };
}
