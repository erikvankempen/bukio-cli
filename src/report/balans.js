/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Balans (balance sheet) as of a date, grouped by RGS hoofdgroep.
// Integrity invariant: total assets == total liabilities + equity + result.
import { rgsLabel } from '../core/chart.js';

const BALANCE_TYPES = ['asset', 'liability', 'equity'];
// Display order: activa first, then passiva.
const ASSET_GROUPS = ['BMVA.02', 'BFVA.03', 'BVRD.30', 'BVOR.11', 'BLIM.10'];
const PASSIVA_GROUPS = ['BEIV.05', 'BVRZ.07', 'BLAS.08', 'BSCH.12'];

function netPerAccount(db, { asOf, types }) {
  const placeholders = types.map(() => '?').join(',');
  return db.prepare(`
    SELECT a.id, a.code, a.name, a.type, a.taxonomy_code,
      COALESCE(SUM(p.amount_cents), 0) AS net_cents
    FROM accounts a
    LEFT JOIN (
      SELECT p.account_id, p.amount_cents
      FROM postings p
      JOIN journal_entries e ON e.id = p.entry_id
      WHERE e.state = 'posted' AND e.date <= ?
    ) p ON p.account_id = a.id
    WHERE a.type IN (${placeholders})
    GROUP BY a.id
    ORDER BY a.code
  `).all(asOf, ...types);
}

/**
 * Balans as of `asOf` (inclusive).
 * - Balance accounts (asset/liability/equity): net as of asOf.
 * - "Nog te verdelen resultaat": net result of income/expense accounts as of
 *   asOf (until closing entries exist — Phase 4 — result accumulates since
 *   inception; the balans always balances either way).
 */
export function balans(db, { asOf }) {
  // validate asOf: a garbage date would silently read as "as of forever"
  // ('2026-01-15' <= 'garbage' is TRUE in string comparison) and return the
  // all-time balans with a misleading as_of label
  if (typeof asOf !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(asOf)) {
    const e = new Error(`as-of '${asOf}' must be YYYY-MM-DD`);
    e.code = 'INVALID_DATE';
    throw e;
  }
  {
    const d = new Date(`${asOf}T00:00:00Z`);
    if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== asOf) {
      const e = new Error(`as-of '${asOf}' is not a valid calendar date`);
      e.code = 'INVALID_DATE';
      throw e;
    }
  }
  const rows = netPerAccount(db, { asOf, types: BALANCE_TYPES });

  const resultRow = db.prepare(`
    SELECT COALESCE(SUM(p.amount_cents), 0) AS net_cents
    FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id
    JOIN accounts a ON a.id = p.account_id
    WHERE e.state = 'posted' AND e.date <= ? AND a.type IN ('income','expense')
  `).get(asOf);
  // income nets are negative (credit); result = -(expense nets) - (income nets)
  const resultCents = -resultRow.net_cents || 0;

  const sectionize = (groups, types) => {
    const known = new Set(groups);
    const sideRows = rows.filter((r) => types.includes(r.type));
    const sections = groups.map((code) => {
      const accounts = sideRows
        .filter((r) => (r.taxonomy_code || 'overig') === code)
        .map((r) => ({
          code: r.code,
          name: r.name,
          type: r.type,
          balance_cents: r.type === 'asset' ? r.net_cents : -r.net_cents,
        }))
        .filter((a) => a.balance_cents !== 0);
      return {
        taxonomy_code: code,
        label: rgsLabel(code),
        accounts,
        total_cents: accounts.reduce((s, a) => s + a.balance_cents, 0),
      };
    }).filter((s) => s.accounts.length > 0);

    // catch-all: side accounts whose taxonomy_code is not in the known group list
    const leftover = sideRows
      .filter((r) => !known.has(r.taxonomy_code || 'overig'))
      .map((r) => ({
        code: r.code,
        name: r.name,
        type: r.type,
        balance_cents: r.type === 'asset' ? r.net_cents : -r.net_cents,
      }))
      .filter((a) => a.balance_cents !== 0);
    if (leftover.length) {
      sections.push({
        taxonomy_code: null,
        label: 'Overig',
        accounts: leftover,
        total_cents: leftover.reduce((s, a) => s + a.balance_cents, 0),
      });
    }
    return sections;
  };

  const assetSections = sectionize(ASSET_GROUPS, ['asset']);
  const passivaSections = sectionize(PASSIVA_GROUPS, ['liability', 'equity']);
  const totalAssets = assetSections.reduce((s, g) => s + g.total_cents, 0);
  const totalPassiva = passivaSections.reduce((s, g) => s + g.total_cents, 0) + resultCents;

  return {
    as_of: asOf,
    assets: {
      total_cents: totalAssets,
      sections: assetSections,
    },
    liabilities_and_equity: {
      total_cents: totalPassiva,
      sections: passivaSections,
      result_cents: resultCents,
    },
    balanced: totalAssets === totalPassiva,
  };
}
