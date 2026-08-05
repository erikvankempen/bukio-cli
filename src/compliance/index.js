// Compliance calendar (Phase 5) — the Dutch SME filing deadlines:
//   OB quarterly (Q1 30 Apr, Q2 31 Jul, Q3 31 Oct, Q4 31 Jan next year),
//   ICP quarterly (same deadlines), jaarrekening deposit within 13 months of
//   the fiscal year end (art. 2:394 BW). Statuses come from vat_returns (OB)
//   and the filings registry (ICP, JAARREKENING).
import { record } from '../audit/index.js';

export function complianceError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

export function quarterDeadline(period) {
  const m = String(period).match(/^(\d{4})-Q([1-4])$/);
  if (!m) throw complianceError('INVALID_PERIOD', `period '${period}' must be YYYY-Qn`);
  const [, year, qn] = m;
  const months = { 1: '04-30', 2: '07-31', 3: '10-31', 4: '01-31' };
  const deadline = `${qn === '4' ? Number(year) + 1 : year}-${months[qn]}`;
  return { period, deadline };
}

export function jaarrekeningDeadline(company, year) {
  // last day of the month that is 13 months after the fiscal-year-end month
  // (art. 2:394 BW: deposit within 13 months after the balance sheet date)
  const fy = company.fiscal_year_end || '12-31';
  const [mm] = fy.split('-').map(Number);
  const total = mm + 13;
  const y = Number(year) + Math.floor((total - 1) / 12);
  const m = ((total - 1) % 12) + 1;
  const lastDay = new Date(Date.UTC(y, m, 0)).getUTCDate();
  return `${y}-${String(m).padStart(2, '0')}-${String(lastDay).padStart(2, '0')}`;
}

export function isFiled(db, type, period) {
  if (type === 'OB') {
    return Boolean(db.prepare("SELECT 1 FROM vat_returns WHERE type = 'OB' AND period = ? AND status = 'filed'").get(period));
  }
  return Boolean(db.prepare('SELECT 1 FROM filings WHERE type = ? AND period = ?').get(type, period));
}

export function markFiled(db, { type, period, date = null, actor = 'human' }) {
  if (!['OB', 'ICP', 'JAARREKENING'].includes(type)) {
    throw complianceError('INVALID_TYPE', "type must be OB, ICP or JAARREKENING (OB uses 'vat readout --mark-filed')");
  }
  if (type === 'OB') {
    throw complianceError('INVALID_TYPE', "OB filings are recorded with 'bukio vat readout --period ... --mark-filed'");
  }
  if (type === 'ICP' && !/^\d{4}-Q[1-4]$/.test(period)) throw complianceError('INVALID_PERIOD', 'ICP period must be YYYY-Qn');
  if (type === 'JAARREKENING' && !/^\d{4}$/.test(period)) throw complianceError('INVALID_PERIOD', 'JAARREKENING period must be YYYY');
  const filedDate = date ?? new Date().toISOString().slice(0, 10);
  db.prepare(`
    INSERT INTO filings (type, period, filed_at, created_by)
    VALUES (?, ?, ?, ?)
    ON CONFLICT(type, period) DO UPDATE SET filed_at = excluded.filed_at
  `).run(type, period, filedDate, actor);
  record(db, {
    actor, action: 'compliance.mark', command: 'compliance mark',
    args: { type, period, filed_at: filedDate }, outcome: 'ok',
  });
  return { type, period, filed_at: filedDate };
}

export function complianceStatus(db, { year }) {
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get();
  if (!company) throw complianceError('NOT_INITIALISED', 'company database not initialised');

  const obligations = [];
  const today = new Date().toISOString().slice(0, 10);
  const push = (type, period, deadline, extra = {}) => {
    const filed = isFiled(db, type, period);
    obligations.push({
      type, period, deadline,
      status: filed ? 'filed' : deadline < today ? 'overdue' : 'open',
      ...extra,
    });
  };

  // OB + ICP per quarter with a deadline in or after the calendar year
  for (const qn of ['1', '2', '3', '4']) {
    const period = `${year}-Q${qn}`;
    const { deadline } = quarterDeadline(period);
    if (deadline < `${year}-01-01`) continue;
    push('OB', period, deadline);
    push('ICP', period, deadline);
  }
  // the Q4 obligation of the previous year falls in this calendar year
  const prevQ4 = quarterDeadline(`${Number(year) - 1}-Q4`);
  if (prevQ4.deadline >= `${year}-01-01`) {
    push('OB', `${Number(year) - 1}-Q4`, prevQ4.deadline);
    push('ICP', `${Number(year) - 1}-Q4`, prevQ4.deadline);
  }

  // jaarrekening: FY `year` deposit (13 months after FY end) + prior FY if still open
  const deadline = jaarrekeningDeadline(company, year);
  push('JAARREKENING', String(year), deadline, { books_closed: isBooksClosed(db, year) });
  const prevDeadline = jaarrekeningDeadline(company, Number(year) - 1);
  if (!isFiled(db, 'JAARREKENING', String(Number(year) - 1)) && prevDeadline < deadline) {
    push('JAARREKENING', String(Number(year) - 1), prevDeadline, { books_closed: isBooksClosed(db, Number(year) - 1) });
  }

  return {
    year,
    company: company.name,
    as_of: today,
    obligations,
    summary: {
      filed: obligations.filter((o) => o.status === 'filed').length,
      overdue: obligations.filter((o) => o.status === 'overdue').length,
      open: obligations.filter((o) => o.status === 'open').length,
    },
    note: 'Assumes quarterly OB/ICP filing; adapt if you file monthly. Never auto-files.',
  };
}

function isBooksClosed(db, year) {
  return Boolean(db.prepare("SELECT 1 FROM journal_entries WHERE source = 'closing' AND source_ref = ?").get(`fy:${year}`));
}
