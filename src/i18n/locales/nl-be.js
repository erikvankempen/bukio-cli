/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// i18n table for 'nl-be' — split from src/i18n/index.js into per-language
// modules (16 Aug 2026). Keys/values byte-identical to the previous
// single-file layout; the parity guard in test/i18n.test.js keeps all
// full tables at the same key set.
//
// REGIONAL OVERRIDE — only the keys that differ from the base language.
export default {

  "pdf.credit": "CREDITNOTA",
  "pdf.kvk": "KBO",
  "pdf.btw": "BTW",
  "pdf.vat": "BTW",
  "vat.file.description": "BTW-aangifte{period} — overdracht naar {account} ({direction})",
  "vat.settle.description": "Betaling BTW-aangifte{period} — {account} (afrondingsverschil {amount})",
  "email.reminderSubject": "Aanmaning factuur {number}",
  "report.undistributedResult": "te bestemmen resultaat",
  "invlist.reminder": "aanmaning",
};
