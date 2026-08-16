/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Compliance calendar (Phase 5) — per-jurisdiction SME filing deadlines (profile-driven):
//   OB quarterly (Q1 30 Apr, Q2 31 Jul, Q3 31 Oct, Q4 31 Jan next year),
//   ICP quarterly (same deadlines), jaarrekening deposit within 13 months of
//   the fiscal year end (art. 2:394 BW). Statuses come from vat_returns (OB)
//   and the filings registry (ICP, JAARREKENING).
import { record } from '../audit/index.js';
import { resolveProfile } from '../jurisdictions/index.js';

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
  // (art. 2:394 BW: deposit within 13 months after the balance sheet date).
  // Interpretation: the 13th MONTH's last day (common KvK/Belastingdienst
  // practice, and the calendar-friendly reading) — not the exact
  // calendar-13-months date, which for a 06-30 FY would be 07-30, one day
  // earlier than the 07-31 used here.
  const fy = company.fiscal_year_end || '12-31';
  // tolerant parse like fiscalYearWindow: take the SECOND-TO-LAST part so a
  // full 'YYYY-MM-DD' value works too (first-part would read 2026 as the
  // month and compute a deadline ~170 years out)
  const parts = String(fy).split('-');
  const mm = Number(parts[parts.length - 2]);
  const total = mm + 13;
  const y = Number(year) + Math.floor((total - 1) / 12);
  const m = ((total - 1) % 12) + 1;
  const lastDay = new Date(Date.UTC(y, m, 0)).getUTCDate();
  return `${y}-${String(m).padStart(2, '0')}-${String(lastDay).padStart(2, '0')}`;
}

// Deadline rules keyed by profile.compliance.filingTypes[].deadlineRule.
// Rules return a 'YYYY-MM-DD' deadline string. Phase B5 adds the LU rules
// (TVA: 15th of the month after the period; annual 1 March; annual accounts
// deposit within ~7 months of the FY end per the LSC 2002 law).
function luQuarterDeadline(period) {
  const m = String(period).match(/^(\d{4})-Q([1-4])$/);
  if (!m) throw complianceError('INVALID_PERIOD', `period '${period}' must be YYYY-Qn`);
  const [, year, qn] = m;
  const months = { 1: '04-15', 2: '07-15', 3: '10-15', 4: '01-15' };
  return `${qn === '4' ? Number(year) + 1 : year}-${months[qn]}`;
}
function luMonthlyDeadline(period) {
  const m = String(period).match(/^(\d{4})-(\d{2})$/);
  if (!m) throw complianceError('INVALID_PERIOD', `period '${period}' must be YYYY-MM`);
  const year = Number(m[1]);
  const month = Number(m[2]);
  if (month < 1 || month > 12) throw complianceError('INVALID_PERIOD', `period '${period}' must be YYYY-MM`);
  const [y, mm] = month === 12 ? [year + 1, 1] : [year, month + 1];
  return `${y}-${String(mm).padStart(2, '0')}-15`;
}

// quarter rules operate on YYYY-Qn; guard exported rule functions against
// malformed input (complianceStatus only ever feeds Q1-4, but the rules are
// public API)
function assertQuarter(period) {
  if (!/^\d{4}-Q[1-4]$/.test(String(period))) {
    throw complianceError('INVALID_PERIOD_SHAPE', `expected YYYY-Qn, got '${period}'`);
  }
}

// day D of the month following YYYY-MM
function dayOfNextMonth(period, day) {
  if (!/^\d{4}-\d{2}$/.test(String(period)) || Number(String(period).slice(5, 7)) < 1 || Number(String(period).slice(5, 7)) > 12) {
    throw complianceError('INVALID_PERIOD_SHAPE', `dayOfNextMonth expects YYYY-MM, got '${period}'`);
  }
  const [y, m] = String(period).split('-').map(Number);
  const ny = m === 12 ? y + 1 : y;
  const nm = m === 12 ? 1 : m + 1;
  return `${ny}-${String(nm).padStart(2, '0')}-${String(day).padStart(2, '0')}`;
}
function luAnnualAccountsDeadline(company, year) {
  // LSC (loi 19.12.2002): accounts approved within 6 months of the FY end,
  // deposited with the RCS within 1 month of approval (~7 months). Last day
  // of the month 7 months after the FY-end month.
  const fy = company.fiscal_year_end || '12-31';
  const parts = String(fy).split('-');
  const mm = Number(parts[parts.length - 2]);
  const total = mm + 7;
  const y = Number(year) + Math.floor((total - 1) / 12);
  const m = ((total - 1) % 12) + 1;
  const lastDay = new Date(Date.UTC(y, m, 0)).getUTCDate();
  return `${y}-${String(m).padStart(2, '0')}-${String(lastDay).padStart(2, '0')}`;
}
// Last day of the month N months after the fiscal-year-end month of the
// fiscal year ENDING in `year` (Companies House / HMRC style deadlines).
function monthsAfterFyEnd(company, year, n) {
  const fy = company.fiscal_year_end || '12-31';
  const parts = String(fy).split('-');
  const mm = Number(parts[parts.length - 2]);
  const total = mm + n;
  const y = Number(year) + Math.floor((total - 1) / 12);
  const m = ((total - 1) % 12) + 1;
  const lastDay = new Date(Date.UTC(y, m, 0)).getUTCDate();
  return `${y}-${String(m).padStart(2, '0')}-${String(lastDay).padStart(2, '0')}`;
}
const DEADLINE_RULES = {
  'nl-quarterly': (period) => quarterDeadline(period).deadline,
  'nl-13-months': (company, year) => jaarrekeningDeadline(company, year),
  'lu-quarterly': luQuarterDeadline,
  'lu-monthly': luMonthlyDeadline,
  'lu-annual': (company, year) => `${Number(year) + 1}-03-01`, // TVA annual return, 1 March next year
  'lu-7-months': luAnnualAccountsDeadline,
  // GB (Companies House, CA 2006 s.442): annual accounts 9 months after
  // FYE; CT600 corporation tax return 12 months after the period end
  'gb-9-months': (company, year) => monthsAfterFyEnd(company, year, 9),
  'gb-ct600': (company, year) => monthsAfterFyEnd(company, year, 12),
  // US: Form 1120/1120-S due the 15th day of the 4th month after the tax
  // year end (April 15 for calendar-year); Form 941 quarterly, due the
  // last day of the month after the quarter
  'us-1120': (company, year) => {
    const fy = company.fiscal_year_end || '12-31';
    const parts = String(fy).split('-');
    const mm = Number(parts[parts.length - 2]);
    const total = mm + 4;
    const y = Number(year) + Math.floor((total - 1) / 12);
    const m = ((total - 1) % 12) + 1;
    return `${y}-${String(m).padStart(2, '0')}-15`;
  },
  'us-941': (period) => quarterDeadline(period).deadline, // same month-end shape as NL
  // BE (research §10): VAT monthly due the 20th of the following month;
  // quarterly option (25th of the month after the quarter); annual
  // accounts with the NBB within 7 months of FYE
  'be-vat-monthly': (period) => dayOfNextMonth(period, 20),
  'be-quarterly': (period) => {
    assertQuarter(period);
    const q = Number(String(period).split('-')[1].replace('Q', ''));
    const m = q * 3 + 1;
    const y = Number(String(period).split('-')[0]) + (m > 12 ? 1 : 0);
    return `${y}-${String(((m - 1) % 12) + 1).padStart(2, '0')}-25`;
  },
  'be-7-months': (company, year) => monthsAfterFyEnd(company, year, 7),
  // DE (research §10): UStVA due the 10th of the month after the quarter;
  // annual VAT return 31 July of the following year (§ 149 AO); annual
  // accounts filed (Offenlegung) 12 months after the balance-sheet date
  // (§ 325 HGB)
  'de-ustva-quarterly': (period) => {
    assertQuarter(period);
    const q = Number(String(period).split('-')[1].replace('Q', ''));
    const m = q * 3 + 1;
    const y = Number(String(period).split('-')[0]) + (m > 12 ? 1 : 0);
    return `${y}-${String(((m - 1) % 12) + 1).padStart(2, '0')}-10`;
  },
  'de-annual-vat': (company, year) => `${Number(year) + 1}-07-31`,
  'de-12-months': (company, year) => monthsAfterFyEnd(company, year, 12),
  // DK (research §10, SKAT): quarterly VAT due the 1st day of the 3rd
  // following month (Q2 -> 1 Sep); annual report (class B) within 5 months
  // of FYE
  'dk-quarterly': (period) => {
    assertQuarter(period);
    const q = Number(String(period).split('-')[1].replace('Q', ''));
    const m = q * 3 + 3;
    const y = Number(String(period).split('-')[0]) + (m > 12 ? 1 : 0);
    return `${y}-${String(((m - 1) % 12) + 1).padStart(2, '0')}-01`;
  },
  'dk-5-months': (company, year) => monthsAfterFyEnd(company, year, 5),
  // FI (research §10): quarterly VAT due the 12th of the second month
  // after the quarter (Q1 -> 12 May); annual accounts FILED within 8
  // months of FYE (PRH; prepared within 4)
  'fi-quarterly': (period) => {
    assertQuarter(period);
    const q = Number(String(period).split('-')[1].replace('Q', ''));
    const m = q * 3 + 2;
    const y = Number(String(period).split('-')[0]) + (m > 12 ? 1 : 0);
    return `${y}-${String(((m - 1) % 12) + 1).padStart(2, '0')}-12`;
  },
  'fi-8-months': (company, year) => monthsAfterFyEnd(company, year, 8),
  // NO (research §10, Altinn): bi-monthly mva-meldingen — 6 periods per
  // year, due 1 month + 10 days after the period end (P3 May/Jun is the
  // 31 Aug summer exception, P6 Nov/Dec due 10 Feb next year); annual
  // accounts approved ≤ 6 months + filed ≤ 1 month after FYE (31 July
  // for calendar year)
  'no-bimonthly': (period) => {
    const m = String(period).match(/^(\d{4})-P([1-6])$/);
    if (!m) throw complianceError('INVALID_PERIOD', `period '${period}' must be YYYY-Pn`);
    const schedule = { 1: '04-10', 2: '06-10', 3: '08-31', 4: '10-10', 5: '12-10' };
    if (m[2] === '6') return `${Number(m[1]) + 1}-02-10`;
    return `${m[1]}-${schedule[m[2]]}`;
  },
  'no-7-months': (company, year) => monthsAfterFyEnd(company, year, 7),
  // SE (research §10, Skatteverket): quarterly momsredovisning due the
  // 12th of the SECOND month after the quarter (August shifts to the
  // 17th; Q1 -> 12 May, Q2 -> 17 Aug, Q3 -> 12 Nov, Q4 -> 12 Feb next
  // year); annual report filed with Bolagsverket within 7 months of FYE
  'se-quarterly': (period) => {
    assertQuarter(period);
    const q = Number(String(period).split('-')[1].replace('Q', ''));
    const m = q * 3 + 2;
    const y = Number(String(period).split('-')[0]) + (m > 12 ? 1 : 0);
    const month = ((m - 1) % 12) + 1;
    const day = month === 8 ? 17 : 12; // August exception
    return `${y}-${String(month).padStart(2, '0')}-${day}`;
  },
  'se-7-months': (company, year) => monthsAfterFyEnd(company, year, 7),
  // AT (research §10, BMF/usp.gv.at): UVA due the 15th of the SECOND
  // following month — quarterly when prior-year turnover ≤ €100,000, monthly
  // above (§ 21 UStG); the annual USt-Erklärung is due 30 June of the
  // following year (mandatory electronic filing)
  'at-uva-quarterly': (period) => {
    assertQuarter(period);
    const q = Number(String(period).split('-')[1].replace('Q', ''));
    const m = q * 3 + 2;
    const y = Number(String(period).split('-')[0]) + (m > 12 ? 1 : 0);
    return `${y}-${String(((m - 1) % 12) + 1).padStart(2, '0')}-15`;
  },
  'at-annual-vat': (company, year) => `${Number(year) + 1}-06-30`,
  // IE (research §8, Revenue): VAT3 bi-monthly returns — six two-month
  // periods per year, due the 23rd of the month after the period end
  // (P1 Jan/Feb -> 23 Mar ... P6 Nov/Dec -> 23 Jan next year); annual
  // accounts + Form B1 to the CRO and Form CT1 to Revenue within 9 months
  // of the FYE / period end
  'ie-bimonthly': (period) => {
    const m = String(period).match(/^(\d{4})-P([1-6])$/);
    if (!m) throw complianceError('INVALID_PERIOD', `period '${period}' must be YYYY-Pn`);
    const schedule = { 1: '03-23', 2: '05-23', 3: '07-23', 4: '09-23', 5: '11-23' };
    if (m[2] === '6') return `${Number(m[1]) + 1}-01-23`;
    return `${m[1]}-${schedule[m[2]]}`;
  },
  'ie-9-months': (company, year) => monthsAfterFyEnd(company, year, 9),
  // IT (research §8, Agenzia Entrate): liquidazione IVA quarterly versamento
  // by the 16th of the SECOND month after the quarter (Q1 -> 16 May; +1%
  // interest vs monthly; monthly above €400K turnover is the 16th of the
  // following month — the quarterly rule is the SME default); Dichiarazione
  // IVA 30 April of the following year; bilancio approved within 120 days
  // and deposited within 30 days of approval (~5 months after FYE)
  'it-liquidazione-quarterly': (period) => {
    assertQuarter(period);
    const q = Number(String(period).split('-')[1].replace('Q', ''));
    const m = q * 3 + 2;
    const y = Number(String(period).split('-')[0]) + (m > 12 ? 1 : 0);
    return `${y}-${String(((m - 1) % 12) + 1).padStart(2, '0')}-16`;
  },
  'it-dichiarazione-iva': (company, year) => `${Number(year) + 1}-04-30`,
  'it-bilancio': (company, year) => monthsAfterFyEnd(company, year, 5),
  // ES (research §8, AEAT): Modelo 303 quarterly within the first 20 days
  // after the quarter (Q1 20 Apr, Q2 20 Jul, Q3 20 Oct, Q4 30 Jan next
  // year); Modelo 390 annual summary 30 Jan; Modelo 200 (Impuesto sobre
  // Sociedades, 25%) within 25 days of the 6 months after FYE; cuentas
  // anuales deposited at the Registro Mercantil 7 months after FYE
  'es-303-quarterly': (period) => {
    assertQuarter(period);
    const q = Number(String(period).split('-')[1].replace('Q', ''));
    if (q === 4) return `${Number(String(period).split('-')[0]) + 1}-01-30`;
    const m = q * 3 + 1;
    return `${String(period).split('-')[0]}-${String(m).padStart(2, '0')}-20`;
  },
  'es-390': (company, year) => `${Number(year) + 1}-01-30`,
  'es-200': (company, year) => {
    // Modelo 200 (Impuesto sobre Sociedades, 25%): due within the first 25
    // days of the month FOLLOWING the 6-month point after FYE (calendar
    // year -> 1-25 July of the following year)
    const fy = company.fiscal_year_end || '12-31';
    const parts = String(fy).split('-');
    const mm = Number(parts[parts.length - 2]);
    const total = mm + 7;
    const y = Number(year) + Math.floor((total - 1) / 12);
    const m = ((total - 1) % 12) + 1;
    return `${y}-${String(m).padStart(2, '0')}-25`;
  },
  'es-7-months': (company, year) => monthsAfterFyEnd(company, year, 7),
  // PT (research §8, AT/PwC): Declaração Periódica due the 20th of the
  // SECOND month after the period (quarterly for SMEs: Q1 20 May, Q2 20
  // Aug, Q3 20 Nov, Q4 20 Feb next year; monthly above €650K / intra-EC);
  // Modelo 22 (IRC, 21%) 31 May; annual accounts via IES 15 July
  'pt-dp-quarterly': (period) => {
    assertQuarter(period);
    const q = Number(String(period).split('-')[1].replace('Q', ''));
    const m = q * 3 + 2;
    const y = Number(String(period).split('-')[0]) + (m > 12 ? 1 : 0);
    return `${y}-${String(((m - 1) % 12) + 1).padStart(2, '0')}-20`;
  },
  'pt-irc': (company, year) => `${Number(year) + 1}-05-31`,
  'pt-ies': (company, year) => `${Number(year) + 1}-07-15`,
  // Phase E (research briefs docs-research/{bg,hr,si,ee,lv,lt,mt,cy}-profile.md):
  // BG — monthly ДДС return by the 14th of the following month; annual
  // accounts + CIT return (form 1010) both due 30 June of the following year
  'bg-vat-monthly': (period) => dayOfNextMonth(period, 14),
  'bg-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 6),
  'bg-cit': (company, year) => monthsAfterFyEnd(company, year, 6), // form 1010
  // HR — monthly PDV return by the 20th of the following month (quarterly
  // option for small taxpayers); annual accounts (RGFI) + CIT return both
  // due 30 April of the following year
  'hr-vat-monthly': (period) => dayOfNextMonth(period, 20),
  'hr-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 4),
  'hr-cit': (company, year) => monthsAfterFyEnd(company, year, 4),
  // SI — monthly DDV-O by the 20th of the following month (quarterly for
  // small filers with an annual DDV-O 31 March); annual report to AJPES
  // within 8 months of FYE (31 August); DDPO (CIT) by 31 March
  'si-vat-monthly': (period) => dayOfNextMonth(period, 20),
  'si-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 8),
  'si-ddpo': (company, year) => `${Number(year) + 1}-03-31`,
  // EE — monthly KMD by the 20th of the following month; annual report to
  // the e-Äriregister within 6 months of FYE (30 June); CIT on
  // distributions only (10th of the month after the distribution) — no
  // annual CIT return
  'ee-vat-monthly': (period) => dayOfNextMonth(period, 20),
  'ee-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 6),
  // LV — monthly PVN by the 20th of the following month (no quarterly
  // option); annual report approved within 6 months of FYE and filed
  // within 1 month of approval (≈ 31 July); CIT on distributions only —
  // no annual CIT return
  'lv-vat-monthly': (period) => dayOfNextMonth(period, 20),
  'lv-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 7),
  // LT — monthly PVM by the 25th of the following month (i.SAF); annual
  // accounts approved within 4 months of FYE and filed shortly after
  // (~30 April); CIT annual return due 1 October of the following year
  'lt-vat-monthly': (period) => dayOfNextMonth(period, 25),
  'lt-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 4),
  'lt-cit': (company, year) => `${Number(year) + 1}-10-01`,
  // MT (research §5, MTCA/myTax): quarterly VAT return due the 15th of the
  // SECOND month after the quarter (22nd when filed online — Q1 -> 15/22
  // May); annual accounts with MBR within 10 months of FYE; CIT form C on
  // self-assessment 9 months after FYE
  'mt-vat-quarterly': (period) => {
    assertQuarter(period);
    const q = Number(String(period).split('-')[1].replace('Q', ''));
    const m = q * 3 + 2;
    const y = Number(String(period).split('-')[0]) + (m > 12 ? 1 : 0);
    return `${y}-${String(((m - 1) % 12) + 1).padStart(2, '0')}-15`;
  },
  'mt-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 10),
  'mt-cit': (company, year) => monthsAfterFyEnd(company, year, 9),
  // CY (research §5, Tax For All portal): quarterly VAT 4 return due the
  // 10th of the SECOND month after the quarter (Q1 -> 10 May; monthly
  // above €1M turnover); annual accounts 42 days after the AGM (~10-13
  // months after FYE in practice); CIT TD4 permanent rule from TY2026:
  // 31 January of the second year following the tax year (transitional
  // decree dates for TY2023-25 — e.g. TY2024 -> 30 Nov 2026 — are NOT
  // modelled: historical-year views render the permanent y+2-01-31 rule,
  // a documented simplification)
  'cy-vat-quarterly': (period) => {
    assertQuarter(period);
    const q = Number(String(period).split('-')[1].replace('Q', ''));
    const m = q * 3 + 2;
    const y = Number(String(period).split('-')[0]) + (m > 12 ? 1 : 0);
    return `${y}-${String(((m - 1) % 12) + 1).padStart(2, '0')}-10`;
  },
  'cy-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 10),
  'cy-td4': (company, year) => `${Number(year) + 2}-01-31`,
  // Phase F (research briefs docs-research/{cz,sk,gr,pl,hu,ro}-profile.md):
  // CZ — DPH monthly/quarterly by the 25th of the following month; annual
  // accounts approved within 6 months of FYE (+30 days filing); CIT within
  // 3 months of FYE (31 March; 1 July for audited companies)
  'cz-vat-monthly': (period) => dayOfNextMonth(period, 25),
  'cz-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 6),
  'cz-cit': (company, year) => monthsAfterFyEnd(company, year, 3),
  // SK — DPH monthly/quarterly by the 25th; annual accounts within 6
  // months of FYE; CIT 31 March (6-month extension with fee)
  'sk-vat-monthly': (period) => dayOfNextMonth(period, 25),
  'sk-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 6),
  'sk-cit': (company, year) => `${Number(year) + 1}-03-31`,
  // GR — ΦΠΑ monthly by the 26th of the following month (quarterly filers:
  // 30th of the month after the quarter); annual accounts published/filed
  // (GEMI) within 10 months of FYE; CIT return (ΝΠΟ) 30 June
  'gr-vat-monthly': (period) => dayOfNextMonth(period, 26),
  'gr-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 10),
  'gr-cit': (company, year) => `${Number(year) + 1}-06-30`,
  // PL — JPK_V7M monthly by the 25th (quarterly JPK_V7K for small);
  // annual accounts approved within 6 months + filed with KRS within 15
  // days of approval; CIT-8 by 31 March
  'pl-vat-monthly': (period) => dayOfNextMonth(period, 25),
  'pl-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 6),
  'pl-cit': (company, year) => `${Number(year) + 1}-03-31`,
  // HU — ÁFA monthly by the 20th (quarterly/annual options by turnover);
  // annual report (beszámoló) within 5 months of FYE (31 May); TAO (CIT)
  // by 31 May
  'hu-vat-monthly': (period) => dayOfNextMonth(period, 20),
  'hu-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 5),
  'hu-cit': (company, year) => `${Number(year) + 1}-05-31`,
  // RO — D300 monthly by the 25th (quarterly for small); annual accounts
  // filed within 150 days of FYE (~30 May); D101 annual CIT return +
  // payment 25 June of the following year
  'ro-vat-monthly': (period) => dayOfNextMonth(period, 25),
  'ro-annual-accounts': (company, year) => monthsAfterFyEnd(company, year, 5),
  'ro-cit': (company, year) => `${Number(year) + 1}-06-25`,
  // Phase G — Kosovo (docs-research/xk-profile.md): monthly VAT by the 20th,
  // annual CIT + financial statements by 31 March (10% flat CIT; ATK/Fryti
  // 2026 calendars). Non-EU: eu-invoice-vereisten baseline applies.
  'xk-vat-monthly': (period) => dayOfNextMonth(period, 20),
  'xk-annual-accounts': (company, year) => `${Number(year) + 1}-03-31`,
  'xk-cit': (company, year) => `${Number(year) + 1}-03-31`,
};

export function isFiled(db, type, period) {
  if (type === 'OB') {
    return Boolean(db.prepare("SELECT 1 FROM vat_returns WHERE type = 'OB' AND period = ? AND status = 'filed'").get(period));
  }
  return Boolean(db.prepare('SELECT 1 FROM filings WHERE type = ? AND period = ?').get(type, period));
}

export function markFiled(db, { type, period, date = null, actor = 'human', dryRun = false }) {
  const { compliance } = resolveProfile(db);
  const knownTypes = compliance.filingTypes.map((ft) => ft.type);
  if (!knownTypes.includes(type)) {
    throw complianceError('INVALID_TYPE', `type must be one of ${knownTypes.join(', ')} (OB uses 'vat readout --mark-filed')`);
  }
  if (type === 'OB') {
    throw complianceError('INVALID_TYPE', "OB filings are recorded with 'bukio vat readout --period ... --mark-filed'");
  }
  if (type === 'ICP' && !/^\d{4}-Q[1-4]$/.test(period)) throw complianceError('INVALID_PERIOD', 'ICP period must be YYYY-Qn');
  if (type === 'JAARREKENING' && !/^\d{4}$/.test(period)) throw complianceError('INVALID_PERIOD', 'JAARREKENING period must be YYYY');
  const filedDate = date ?? new Date().toISOString().slice(0, 10);
  if (dryRun) {
    return { action: 'compliance.mark', type, period, filed_at: filedDate, dryRun: true };
  }
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

  // obligations from the profile's filing types (NL: OB + ICP quarterly,
  // JAARREKENING yearly) — deadlines via the per-rule registry
  const { compliance } = resolveProfile(db);
  for (const ft of compliance.filingTypes) {
    const rule = DEADLINE_RULES[ft.deadlineRule];
    if (!rule) {
      throw complianceError('DEADLINE_RULE_NOT_FOUND', `deadline rule '${ft.deadlineRule}' is not implemented (registered: ${Object.keys(DEADLINE_RULES).join(', ')})`);
    }
    if (ft.periodShape === 'YYYY-Qn') {
      for (const qn of ['1', '2', '3', '4']) {
        const period = `${year}-Q${qn}`;
        const deadline = rule(period);
        if (deadline < `${year}-01-01`) continue;
        push(ft.type, period, deadline);
      }
      // the Q4 obligation of the previous year falls in this calendar year
      const prevQ4 = rule(`${Number(year) - 1}-Q4`);
      if (prevQ4 >= `${year}-01-01`) push(ft.type, `${Number(year) - 1}-Q4`, prevQ4);
    } else if (ft.periodShape === 'YYYY-MM') {
      // monthly filings (Phase B5, LU > €620K band): months 01-11 have
      // deadlines in this calendar year; the previous December's return is
      // due on 15 January of this year
      for (const mm of ['01', '02', '03', '04', '05', '06', '07', '08', '09', '10', '11']) {
        const deadline = rule(`${year}-${mm}`);
        if (deadline < `${year}-01-01`) continue;
        push(ft.type, `${year}-${mm}`, deadline);
      }
      const prevDec = rule(`${Number(year) - 1}-12`);
      if (prevDec >= `${year}-01-01`) push(ft.type, `${Number(year) - 1}-12`, prevDec);
    } else if (ft.periodShape === 'YYYY-Pn') {
      // bi-monthly filings (Phase B, NO): 6 periods per year with a fixed
      // schedule (P1 -> 10 Apr ... P6 -> 10 Feb next year); the previous
      // year's P6 falls in this calendar year.
      // NOTE (attribution convention): like the Q4 mirror in YYYY-Qn, P6 is
      // listed under BOTH its period year (deadline in the next year) and
      // the following year as prevP6 — the year view answers "what do I owe
      // for this year's periods"; YYYY-MM instead single-lists December via
      // prevDec. The two branches answer period-year vs deadline-year; both
      // list every obligation exactly once per deadline. Kept as-is: the
      // Q4/P6 mirror predates Phase B (NL quarterly behaviour).
      for (const pn of ['1', '2', '3', '4', '5', '6']) {
        const deadline = rule(`${year}-P${pn}`);
        if (deadline < `${year}-01-01`) continue;
        push(ft.type, `${year}-P${pn}`, deadline);
      }
      const prevP6 = rule(`${Number(year) - 1}-P6`);
      if (prevP6 >= `${year}-01-01`) push(ft.type, `${Number(year) - 1}-P6`, prevP6);
    } else if (ft.periodShape === 'YYYY') {
      const deadline = rule(company, year);
      push(ft.type, String(year), deadline, { books_closed: isBooksClosed(db, year) });
      const prevDeadline = rule(company, Number(year) - 1);
      if (!isFiled(db, ft.type, String(Number(year) - 1)) && prevDeadline < deadline) {
        push(ft.type, String(Number(year) - 1), prevDeadline, { books_closed: isBooksClosed(db, Number(year) - 1) });
      }
    } else {
      throw complianceError('INVALID_PERIOD_SHAPE', `period shape '${ft.periodShape}' is not supported (registered: YYYY-Qn, YYYY-MM, YYYY-Pn, YYYY)`);
    }
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
    note: 'Deadlines follow the country profile\'s filingTypes (NL quarterly, BE monthly 20th, DE quarterly UStVA, NO bi-monthly, ...). Never auto-files.',
  };
}

function isBooksClosed(db, year) {
  // same semantics as isYearClosed: reversed closing entries do not keep the
  // year closed (the documented undo is entry reverse on the closing entries)
  return Boolean(db.prepare(`
    SELECT 1 FROM journal_entries e
    WHERE e.source = 'closing' AND e.source_ref = ?
      AND NOT EXISTS (
        SELECT 1 FROM journal_entries r
        WHERE r.reversed_from_id = e.id
      )
    LIMIT 1
  `).get(`fy:${year}`));
}
