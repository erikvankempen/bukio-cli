/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// IBAN validation (ISO 13616): country prefix + mod-97 check.
// Used for the company's own account, contact accounts, and payment batches.

const IBAN_RE = /^[A-Z]{2}[0-9]{2}[A-Z0-9]{11,30}$/;

/** Strip spaces/dashes and uppercase. */
export function normalizeIban(iban) {
  return String(iban ?? '').replace(/[\s-]/g, '').toUpperCase();
}

/** Full IBAN validity: format + mod-97 (rearranged, letters as A=10..Z=35). */
export function isValidIban(iban) {
  const s = normalizeIban(iban);
  if (!IBAN_RE.test(s)) return false;
  const rearranged = s.slice(4) + s.slice(0, 4);
  let num = '';
  for (const ch of rearranged) {
    num += /[0-9]/.test(ch) ? ch : String(ch.charCodeAt(0) - 55);
  }
  // IBANs are at most 34 chars -> 68 digits, safe for BigInt
  return BigInt(num) % 97n === 1n;
}
