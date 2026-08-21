/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// CAMT.053 (ISO 20022 bank statement) parser.
import { XMLParser } from 'fast-xml-parser';

export function bankError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

/**
 * Parse a decimal string to integer cents (CAMT uses '.' decimals).
 * Integer-only (house rule: no float money math). ISO 20022 amounts allow
 * up to 5 fraction digits; a third+ digit rounds half-up, exactly matching
 * the old parseFloat(x)*100+Math.round semantics without the float path.
 */
function cents(value) {
  const s = String(value ?? '').trim();
  if (!s) return null;
  const m = s.match(/^(-?)(\d+)(?:\.(\d+))?$/);
  if (!m) return null; // not a plain decimal (rejects '1e5', '1.2.3', '€ 5')
  const negative = m[1] === '-';
  const whole = parseInt(m[2], 10);
  const frac = (m[3] ?? '').padEnd(2, '0').slice(0, 2);
  let c = whole * 100 + parseInt(frac || '0', 10);
  const rest = (m[3] ?? '').slice(2);
  if (rest && parseInt(rest[0], 10) >= 5) c += 1; // half-up on the 3rd decimal
  return negative && c !== 0 ? -c : c;
}

function text(node) {
  if (node == null) return null;
  if (typeof node === 'string') {
    // an EMPTY element (<Nm></Nm>) parses to '' — treat it as absent so the
    // counterparty/description fallback chains (??) fall through instead of
    // storing an empty string as the counterparty
    return node.trim() ? node : null;
  }
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
      if (amount == null) {
        // a statement whose Ntry has no parseable <Amt> is corrupt — silently
        // skipping it would import a partial statement and make the account
        // balance diverge from the bank's without any warning
        throw bankError('INVALID_CAMT', 'Ntry without a valid cbc:Amt — the statement is corrupt; fix it before importing');
      }
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
