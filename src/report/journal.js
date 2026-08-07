// Journal export — every posting in a date range, with account info.
// One row per posting (entries without postings appear as a single row).
// `limit` is optional: when set, the query is capped (used by the MCP journal
// tool so a bounded response is actually bounded — the CLI never truncates).
export function journal(db, { from, to, limit = null }) {
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
