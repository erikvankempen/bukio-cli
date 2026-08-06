// Month-end close check — the "is the month ready to close?" report.
// Read-only: unposted drafts, unmatched bank transactions, the OB readout for
// the quarter containing the month, draft + overdue invoices, due recurring
// templates, period totals and the month's profit. Designed to be run by an
// agent on a schedule; never writes, so no --dry-run is needed.
import { formatAmount } from '../core/money.js';
import { listEntries } from '../core/entries.js';
import { listInvoices } from '../invoice/index.js';
import { listTemplates } from '../recurring/index.js';
import { listTransactions } from '../bank/index.js';
import { isVatEnabled, obReadout } from '../vat/index.js';
import { runDue } from '../assets/index.js';

const MONTH_RE = /^\d{4}-(0[1-9]|1[0-2])$/;

export function monthEndError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

function monthBounds(period) {
  const [year, month] = period.split('-').map(Number);
  const from = `${period}-01`;
  const lastDay = new Date(Date.UTC(year, month, 0)).getUTCDate();
  return { from, to: `${period}-${String(lastDay).padStart(2, '0')}` };
}

function quarterOf(period) {
  const month = Number(period.split('-')[1]);
  return `${period.slice(0, 4)}-Q${Math.ceil(month / 3)}`;
}

export function monthEnd(db, { period }) {
  if (!MONTH_RE.test(period)) {
    throw monthEndError('INVALID_PERIOD', `period '${period}' must be yyyy-mm`);
  }
  const { from, to } = monthBounds(period);

  const drafts = listEntries(db, { state: 'draft', dateFrom: from, dateTo: to });
  const bankUnmatched = listTransactions(db, { state: 'unmatched', limit: 100000 })
    .filter((t) => t.date >= from && t.date <= to);
  const invoices = listInvoices(db);
  const draftInvoices = invoices.filter((i) => i.status === 'draft');
  const overdueInvoices = invoices.filter((i) => i.status === 'overdue');
  const recurring = listTemplates(db, { status: 'active' })
    .filter((t) => t.next_run_date && t.next_run_date <= to);

  // fixed assets: depreciation runs due in this period (books stay open until
  // the agent runs `assets run --period <month>`)
  const assetsDue = (() => {
    try {
      const plan = runDue(db, { period, dryRun: true });
      return plan.plan.length;
    } catch {
      return 0; // pre-009 databases: no assets tables yet
    }
  })();

  let vat = null;
  if (isVatEnabled(db)) {
    const readout = obReadout(db, { period: quarterOf(period) });
    vat = {
      quarter: readout.period,
      from: readout.from,
      to: readout.to,
      fields: readout.fields,
      to_pay_cents: readout.to_pay_cents,
      to_pay: readout.to_pay,
    };
  }

  // period totals over posted entries
  const rows = db.prepare(`
    SELECT a.code, a.type, SUM(p.amount_cents) AS net
    FROM postings p
    JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
    JOIN accounts a ON a.id = p.account_id
    WHERE e.date >= ? AND e.date <= ?
    GROUP BY a.id
  `).all(from, to);
  const debit = rows.filter((r) => r.net > 0).reduce((acc, r) => acc + r.net, 0);
  const credit = rows.filter((r) => r.net < 0).reduce((acc, r) => acc - r.net, 0);
  const profitCents = rows
    .filter((r) => r.type === 'income' || r.type === 'expense')
    .reduce((acc, r) => acc - r.net, 0);

  const warnings = [];
  if (drafts.length) warnings.push(`${drafts.length} draft entr${drafts.length === 1 ? 'y' : 'ies'} not posted`);
  if (bankUnmatched.length) warnings.push(`${bankUnmatched.length} unmatched bank transaction${bankUnmatched.length === 1 ? '' : 's'}`);
  if (overdueInvoices.length) warnings.push(`${overdueInvoices.length} overdue invoice${overdueInvoices.length === 1 ? '' : 's'}`);
  if (recurring.length) warnings.push(`${recurring.length} recurring template${recurring.length === 1 ? '' : 's'} due by ${to}`);
  if (assetsDue) warnings.push(`${assetsDue} depreciation run${assetsDue === 1 ? '' : 's'} due — run 'assets run --period ${period}'`);
  if (!drafts.length && !bankUnmatched.length && !overdueInvoices.length && !recurring.length && !assetsDue) {
    warnings.push('all clear — the month can be closed');
  }

  return {
    period,
    from,
    to,
    entries: {
      draft: drafts.length,
      draft_ids: drafts.map((e) => e.id),
    },
    bank: {
      unmatched: bankUnmatched.length,
      unmatched_ids: bankUnmatched.map((t) => t.id),
    },
    vat,
    invoices: {
      draft: draftInvoices.length,
      draft_ids: draftInvoices.map((i) => i.id),
      overdue: overdueInvoices.length,
      overdue_ids: overdueInvoices.map((i) => i.id),
      overdue_total_cents: overdueInvoices.reduce((acc, i) => acc + (i.gross_cents - i.paid_cents), 0),
    },
    recurring: {
      due: recurring.length,
      due_ids: recurring.map((t) => t.id),
    },
    totals: {
      debit_cents: debit,
      credit_cents: credit,
      balanced: debit === credit,
      profit_cents: profitCents,
      profit: formatAmount(profitCents),
    },
    warnings,
  };
}
