// CAMT.053 (ISO 20022 bank statement) parser.
import { XMLParser } from 'fast-xml-parser';

export function bankError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

/** Parse a decimal string to integer cents (CAMT uses '.' decimals). */
function cents(value) {
  const s = String(value ?? '').trim();
  if (!s) return null;
  const n = parseFloat(s);
  if (!Number.isFinite(n)) return null;
  return Math.round(n * 100);
}

function text(node) {
  if (node == null) return null;
  if (typeof node === 'string') return node;
  return null;
}

function isoDate(node) {
  const s = text(node);
  if (!s) return null;
  return s.slice(0, 10); // strip timezone offset
}

/**
 * Parse CAMT.053 XML into transactions:
 * [{ date, amount_cents, counterparty, description, iban_counter }]
 * amount sign: CRDT = positive (money in), DBIT = negative (money out).
 */
export function parseCamt053(xmlText) {
  const parser = new XMLParser({
    ignoreAttributes: true,
    removeNSPrefix: true,
    parseTagValue: false,
    trimValues: true,
  });
  let doc;
  try {
    doc = parser.parse(xmlText);
  } catch (err) {
    throw bankError('INVALID_CAMT', `could not parse CAMT.053 XML: ${err.message}`);
  }

  const stmt = doc?.Document?.BkToCstmrStmt?.Stmt;
  if (!stmt) throw bankError('INVALID_CAMT', 'no BkToCstmrStmt/Stmt found in the XML');

  const stmts = Array.isArray(stmt) ? stmt : [stmt];
  const transactions = [];

  for (const s of stmts) {
    const iban = text(s?.Acct?.Id?.IBAN);
    if (!iban) throw bankError('INVALID_CAMT', 'statement is missing Acct/Id/IBAN');
    const entries = s?.Ntry;
    if (!entries) continue;
    const entryList = Array.isArray(entries) ? entries : [entries];

    for (const ntry of entryList) {
      const amount = cents(ntry?.Amt);
      if (amount == null) continue;
      const direction = String(ntry?.CdtDbtInd ?? '').toUpperCase();
      const date = isoDate(ntry?.BookgDt?.Dt ?? ntry?.ValDt?.Dt ?? ntry?.BookgDt?.Dbt);
      // The bank's own entry reference (AcctSvcrRef) is unique per entry and
      // goes into the dedup hash — two genuinely identical same-day payments
      // (same amount, same counterparty, same description) must both import.
      const bankRef = text(ntry?.AcctSvcrRef);
      const txDtls = ntry?.NtryDtls?.TxDtls;
      const txList = txDtls ? (Array.isArray(txDtls) ? txDtls : [txDtls]) : [{}];

      for (const tx of txList) {
        const rltd = tx?.RltdPties ?? {};
        // counterparty: the other party — creditor when money goes out, debtor when money comes in
        const counterparty = text(direction === 'DBIT' ? rltd?.Cdtr?.Nm : rltd?.Dbtr?.Nm)
          ?? text(rltd?.Dbtr?.Nm)
          ?? text(rltd?.Cdtr?.Nm);
        const ibanCounter = text(direction === 'DBIT' ? rltd?.Cdtr?.Acct?.IBAN : rltd?.Dbtr?.Acct?.IBAN);
        const ustrd = tx?.RmtInf?.Ustrd;
        const description = Array.isArray(ustrd) ? ustrd.filter(Boolean).join(' ') : (text(ustrd) ?? text(ntry?.AddtlNtryInf) ?? '');
        const signedAmount = direction === 'DBIT' ? -amount : amount;

        transactions.push({
          date,
          amount_cents: signedAmount,
          counterparty,
          description,
          iban_counter: ibanCounter ?? null,
          bank_ref: bankRef ?? null,
          iban,
        });
      }
    }
  }

  if (transactions.length === 0) {
    throw bankError('EMPTY_STATEMENT', 'no transactions found in the CAMT.053 statement');
  }
  return transactions;
}
