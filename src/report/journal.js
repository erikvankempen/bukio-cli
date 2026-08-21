/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Journal export — every posting in a date range, with account info.
// One row per posting (entries without postings appear as a single row).
// `limit` is optional: when set, the query is capped (used by the MCP journal
// tool so a bounded response is actually bounded — the CLI never truncates).
import { validateLabeledDate } from '../core/dates.js';

export function journal(db, { from, to, limit = null }) {
  // garbage dates build ranges like 'abc-01-01' — reject before the query
  for (const [label, v] of [['from', from], ['to', to]]) validateLabeledDate(v, label);
  return db.prepare(`
    SELECT e.id AS entry_id, e.date, e.description, e.source, e.state, e.created_by,
      p.amount_cents, a.code AS account_code, a.name AS account_name, a.type AS account_type
    FROM journal_entries e
    LEFT JOIN postings p ON p.entry_id = e.id
    LEFT JOIN accounts a ON a.id = p.account_id
    WHERE e.date >= ? AND e.date <= ?
    ORDER BY e.date, e.id, p.id
    ${limit != null ? 'LIMIT ?' : ''}
  `).all(from, to, ...(limit != null ? [limit] : []));
}
