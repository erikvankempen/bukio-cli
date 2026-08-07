// Dutch bank CSV parser — tolerant of Rabo / ING / ABN AMRO / generic exports.
import { bankError } from './camt.js';

const HEADER_ALIASES = {
  date: ['datum', 'date', 'boekingsdatum', 'transaction date', 'transactiedatum'],
  counterparty: ['naam', 'name', 'tegenpartij', 'begunstigde', 'begunstigd door', 'crediteur', 'debiteur', 'naam / omschrijving'],
  description: ['omschrijving', 'description', 'mededelingen', 'memo', 'opmerkingen', 'naam / omschrijving'],
  amount: ['bedrag', 'amount', 'bedrag (eur)', 'bedrag(eur)', 'bedrag (in eur)', 'bedrag in eur', 'transaction amount'],
  afbij: ['af bij', 'af/bij', 'af-bij', 'bij/af', 'af bij (eur)', 'credit debet'],
  iban_counter: ['tegenrekening', 'iban tegenrekening', 'tegenrekening iban', 'counter iban', 'reknr tegenpartij'],
  code: ['code', 'mutatiesoort', 'transactiesoort'],
};

function normalizeHeader(h) {
  return String(h).trim().toLowerCase().replace(/\s+/g, ' ');
}

function detectDelimiter(line) {
  const semicolons = (line.match(/;/g) || []).length;
  const commas = (line.match(/,/g) || []).length;
  return semicolons >= commas ? ';' : ',';
}

function splitLine(line, delimiter) {
  const out = [];
  let cur = '';
  let inQuotes = false;
  for (const ch of line) {
    if (ch === '"') inQuotes = !inQuotes;
    else if (ch === delimiter && !inQuotes) { out.push(cur); cur = ''; }
    else cur += ch;
  }
  out.push(cur);
  return out;
}

/**
 * Lenient Dutch amount parser: handles '1.234,56', '1234.56', '1.234',
 * '-12,50', '(12,50)', '€ 12,50'. Returns cents or null.
 */
export function parseBankAmount(value) {
  let s = String(value ?? '').trim().replace(/["'\s€]/g, '');
  if (!s) return null;
  let negative = false;
  if (s.startsWith('-') || s.startsWith('(')) negative = true;
  if (s.endsWith(')') || s.endsWith('-')) { negative = s.endsWith('-') ? true : negative; s = s.slice(0, -1); }
  s = s.replace(/^-/, '').replace(/[()]/g, '');
  if (!/^[\d.,]+$/.test(s)) return null;

  let cents;
  if (s.includes('.') && s.includes(',')) {
    const decIdx = Math.max(s.lastIndexOf('.'), s.lastIndexOf(','));
    const intPart = s.slice(0, decIdx).replace(/[.,]/g, '');
    const frac = s.slice(decIdx + 1).replace(/[.,]/g, '').padEnd(2, '0').slice(0, 2);
    cents = parseInt(intPart, 10) * 100 + parseInt(frac, 10);
  } else if (s.includes(',')) {
    const [i, f] = s.split(',');
    cents = parseInt(i.replace(/\./g, ''), 10) * 100 + parseInt((f || '').padEnd(2, '0').slice(0, 2), 10);
  } else if (s.includes('.')) {
    const parts = s.split('.');
    if (parts.length === 2 && parts[1].length <= 2) {
      cents = parseInt(parts[0], 10) * 100 + parseInt(parts[1].padEnd(2, '0'), 10);
    } else {
      cents = parseInt(s.replace(/\./g, ''), 10) * 100; // thousands separator, integer amount
    }
  } else {
    cents = parseInt(s, 10) * 100;
  }
  return negative ? -cents : cents;
}

function findColumn(header, aliases) {
  for (const [key, names] of Object.entries(aliases)) {
    if (names.includes(normalizeHeader(header))) return key;
  }
  return null;
}

/**
 * Parse a bank CSV export into [{ date, amount_cents, counterparty, description, iban_counter, iban }].
 * The IBAN may come from a --iban argument (caller injects it).
 * Header row is required; columns are matched by alias (Dutch + English).
 */
export function parseBankCsv(csvText, { defaultIban = null } = {}) {
  const lines = csvText.split(/\r?\n/).filter((l) => l.trim() !== '');
  if (lines.length < 2) throw bankError('EMPTY_CSV', 'bank CSV needs a header row and at least one transaction');

  const delimiter = detectDelimiter(lines[0]);
  const header = splitLine(lines[0], delimiter).map(normalizeHeader);
  const columns = header.map((h) => findColumn(h, HEADER_ALIASES));

  const idx = {};
  for (const key of Object.keys(HEADER_ALIASES)) idx[key] = columns.indexOf(key);

  if (idx.date === -1 || idx.amount === -1) {
    throw bankError('INVALID_CSV_HEADER',
      `bank CSV needs at least 'datum' and 'bedrag' columns (got: ${header.join(', ')})`);
  }

  const transactions = [];
  const skipped = [];
  for (let i = 1; i < lines.length; i += 1) {
    const row = splitLine(lines[i], delimiter);
    if (row.length === 1 && row[0].trim() === '') continue;
    const get = (key) => (idx[key] >= 0 ? (row[idx[key]] ?? '').trim() : '');

    const date = get('date').slice(0, 10);
    const amount = parseBankAmount(get('amount'));
    if (!date || amount == null) {
      // never drop money silently — report the row so the user can fix the file
      skipped.push({ line: i + 1, reason: !date ? 'missing/invalid date' : `unparseable amount '${get('amount')}'` });
      continue;
    }

    let signedAmount = amount;
    const afbij = get('afbij').toLowerCase();
    if (afbij.startsWith('af')) signedAmount = -Math.abs(amount);
    else if (afbij.startsWith('bij')) signedAmount = Math.abs(amount);

    const ibanCounter = get('iban_counter') || null;
    transactions.push({
      date,
      amount_cents: signedAmount,
      counterparty: get('counterparty') || null,
      description: get('description') || null,
      iban_counter: ibanCounter,
      iban: defaultIban,
    });
  }

  if (transactions.length === 0) {
    throw bankError('EMPTY_STATEMENT', 'no parseable transactions found in the CSV');
  }
  // skipped-row visibility: same array shape as before, with the skipped
  // rows attached (the CLI surfaces them as a warning)
  transactions.skipped = skipped;
  return transactions;
}
