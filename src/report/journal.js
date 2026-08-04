// Journal export — every posting in a date range, with account info.
// One row per posting (entries without postings appear as a single row).
export function journal(db, { from, to }) {
  return db.prepare(`
    SELECT e.id AS entry_id, e.date, e.description, e.source, e.state, e.created_by,
      p.amount_cents, a.code AS account_code, a.name AS account_name, a.type AS account_type
    FROM journal_entries e
    LEFT JOIN postings p ON p.entry_id = e.id
    LEFT JOIN accounts a ON a.id = p.account_id
    WHERE e.date >= ? AND e.date <= ?
    ORDER BY e.date, e.id, p.id
  `).all(from, to);
}
