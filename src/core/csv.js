/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Shared CSV plumbing — the only quote-aware line splitter in the codebase.
// bank/csv.js, import/index.js and core/accounts.js previously each carried
// their own copy of this loop; behaviour is byte-identical, only the
// delimiter source differs per caller.

/** Split one CSV line into cells — quote-aware, honours a single delimiter. */
export function splitCsvLine(line, delim) {
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

/** Decide ';' vs ',' from a sample line: more semicolons wins, ties -> ';' (Dutch exports). */
export function detectDelimiter(line) {
  const semicolons = (line.match(/;/g) || []).length;
  const commas = (line.match(/,/g) || []).length;
  return semicolons >= commas ? ';' : ',';
}
