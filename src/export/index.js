/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Exports — Auditfile Financieel (XAF) 4.0 export for external advisors.
//
// Produces the same XML shape the importer (src/import/index.js) reads, so a
// bookkeeper, tax advisor or auditor can pull the file straight into their
// own software (SnelStart, Exact, ...):
//
//   <Xaf> XafHeader (company, fiscal year) -> Rekeningen (chart) -> Mutaties
//
// Every POSTED entry in the year becomes one <Mutatie> with
// <Boekstuknummer> = entry id (unique, stable). The postings are emitted as
// <Boeking> pairs exactly like the importer consumes them: a Boeking with
// RekeningCode=X, TegenrekeningCode=Y, Bedrag=+B imports as X:+B, Y:-B.
// A balanced entry decomposes into such pairs (debit legs matched against
// credit legs); the export is lossless and round-trips.
//
// Read-only: nothing is written to the DB except one audit-log row recording
// that the export happened (who, when, which file).
import { readFileSync, writeFileSync } from 'node:fs';
import { resolveProfile } from '../jurisdictions/index.js';
import { formatAmount } from '../core/money.js';
import { record } from '../audit/index.js';
import { fiscalYearWindow } from '../year-end/index.js';

// single source of truth for the version stamped into the audit file —
// a hardcoded string here silently drifted from package.json on every bump
const PACKAGE_VERSION = JSON.parse(
  readFileSync(new URL('../../package.json', import.meta.url), 'utf8'),
).version;

function esc(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function todayIso() {
  return new Date().toISOString().slice(0, 10);
}

/** RekeningSoort from account type (as the importer expects). */
function rekeningSoort(type) {
  // Balans accounts: asset/liability (+ equity). Result accounts: Winst en Verlies.
  return (type === 'asset' || type === 'liability' || type === 'equity')
    ? 'Balans'
    : 'Winst en Verlies';
}

/**
 * Decompose an entry's postings into (rekening, tegenrekening, bedrag) pairs.
 * Each posting with amount>0 is a debit leg; amount<0 a credit leg. A Boeking
 * {rekening: D, tegenrekening: C, bedrag: +B} imports back as D:+B, C:-B, so
 * pairing every debit cent against credit cents is lossless. Entries are
 * always balanced (sum == 0), so debit total == credit total.
 */
function toBoekingen(postings) {
  const debits = postings.filter((p) => p.amount_cents > 0)
    .map((p) => ({ code: p.account_code, cents: p.amount_cents }));
  const credits = postings.filter((p) => p.amount_cents < 0)
    .map((p) => ({ code: p.account_code, cents: -p.amount_cents }));
  const boekingen = [];
  let ci = 0; // cursor into credits (credits are split across debits)
  for (const d of debits) {
    let remaining = d.cents;
    while (remaining > 0) {
      while (ci < credits.length && credits[ci].cents === 0) ci += 1;
      if (ci >= credits.length) break; // defensive: unbalanced entry
      const c = credits[ci];
      const take = Math.min(remaining, c.cents);
      boekingen.push({ rekening: d.code, tegenrekening: c.code, bedrag: take });
      c.cents -= take;
      remaining -= take;
    }
  }
  return boekingen;
}

/**
 * Export an Auditfile Financieel 4.0 XML for a fiscal year.
 *
 * Returns { path, year, company, rekeningen, mutaties } — mutaties counts
 * POSTED entries (drafts are not part of the books as they stand). Writes the
 * file; records an `export.xaf` audit row. Throws EXPORT_EMPTY_YEAR when the
 * year has no posted entries.
 */
// audit-file builders keyed by profile.documents.auditFile. NL is the only
// format in Phase A ('xaf-auditfile-4.0'); future markets register theirs.
const XAF_BUILDERS = {
  'xaf-auditfile-4.0': buildXafAuditfile40,
};

export function exportXaf(db, { year, out, actor = 'human', dryRun = false }) {
  const { documents } = resolveProfile(db);
  const builder = XAF_BUILDERS[documents.auditFile] ?? XAF_BUILDERS['xaf-auditfile-4.0'];
  return builder(db, { year, out, actor, dryRun });
}

function buildXafAuditfile40(db, { year, out, actor, dryRun }) {
  if (!/^\d{4}$/.test(String(year))) {
    const e = new Error(`year '${year}' must be YYYY`);
    e.code = 'INVALID_YEAR';
    throw e;
  }
  const company = db.prepare('SELECT * FROM company WHERE id = 1').get() ?? null;
  if (!company) {
    const e = new Error('no company in this database — run `bukio init` first');
    e.code = 'NO_COMPANY';
    throw e;
  }

  // the export covers the FISCAL year the closing year refers to — the
  // same window as year-end close / jaarrekening (default: calendar year)
  const [fyFrom, fyTo] = fiscalYearWindow(db, year);

  const accounts = db.prepare('SELECT * FROM accounts ORDER BY code').all();
  const entries = db.prepare(`
    SELECT e.id, e.date, e.description, e.source, e.state, e.source_ref,
           e.created_by, e.posted_at
    FROM journal_entries e
    WHERE e.date >= ? AND e.date <= ?
    ORDER BY e.date, e.id
  `).all(fyFrom, fyTo);
  const postings = db.prepare(`
    SELECT p.entry_id, p.amount_cents, p.fx_currency, p.fx_amount_cents,
           a.code AS account_code
    FROM postings p
    JOIN accounts a ON a.id = p.account_id
    JOIN journal_entries e ON e.id = p.entry_id
    WHERE e.date >= ? AND e.date <= ?
    ORDER BY p.id
  `).all(fyFrom, fyTo);
  const byEntry = new Map();
  for (const p of postings) {
    if (!byEntry.has(p.entry_id)) byEntry.set(p.entry_id, []);
    byEntry.get(p.entry_id).push(p);
  }

  const posted = entries.filter((e) => e.state === 'posted');
  if (posted.length === 0) {
    const e = new Error(`no posted entries in fiscal year ${year} — nothing to export`);
    e.code = 'EXPORT_EMPTY_YEAR';
    throw e;
  }

  if (dryRun) {
    return {
      ok: true, path: out, year, dryRun: true,
      company: { name: company.name, registration_id: company.registration_id },
      rekeningen: accounts.length,
      mutaties: posted.filter((e) => (byEntry.get(e.id) ?? []).length > 0).length,
    };
  }

  const parts = [];
  parts.push('<?xml version="1.0" encoding="UTF-8"?>');
  parts.push('<Xaf xmlns="http://www.auditfiles.nl/XAF/4.0">');
  parts.push('  <XafHeader>');
  parts.push('    <Version>4.0</Version>');
  parts.push(`    <CompanyName>${esc(company.name)}</CompanyName>`);
  parts.push(`    <CompanyID>${esc(company.registration_id || '')}</CompanyID>`);
  parts.push(`    <FiscalYear>${esc(year)}</FiscalYear>`);
  parts.push(`    <StartDate>${fyFrom}</StartDate>`);
  parts.push(`    <EndDate>${fyTo}</EndDate>`);
  parts.push('    <SoftwareName>bukio-cli</SoftwareName>');
  parts.push(`    <SoftwareVersion>${esc(PACKAGE_VERSION)}</SoftwareVersion>`);
  parts.push('  </XafHeader>');

  parts.push('  <Rekeningen>');
  for (const a of accounts) {
    parts.push('    <Rekening>');
    parts.push(`      <RekeningCode>${esc(a.code)}</RekeningCode>`);
    parts.push(`      <RekeningOmschrijving>${esc(a.name)}</RekeningOmschrijving>`);
    parts.push(`      <RekeningSoort>${rekeningSoort(a.type)}</RekeningSoort>`);
    parts.push('    </Rekening>');
  }
  parts.push('  </Rekeningen>');

  parts.push('  <Mutaties>');
  const mutatieIds = [];
  for (const e of posted) {
    const rows = byEntry.get(e.id) ?? [];
    if (rows.length === 0) continue;
    mutatieIds.push(e.id);
    const desc = e.description || `Boeking ${e.id}`;
    const boekingen = toBoekingen(rows);
    parts.push('    <Mutatie>');
    parts.push(`      <Boekstuknummer>${esc(String(e.id))}</Boekstuknummer>`);
    parts.push(`      <Datum>${esc(String(e.date).slice(0, 10))}</Datum>`);
    parts.push(`      <Omschrijving>${esc(desc)}</Omschrijving>`);
    parts.push('      <Boekingen>');
    for (const b of boekingen) {
      parts.push('        <Boeking>');
      parts.push(`          <RekeningCode>${esc(b.rekening)}</RekeningCode>`);
      parts.push(`          <TegenrekeningCode>${esc(b.tegenrekening)}</TegenrekeningCode>`);
      parts.push(`          <Bedrag>${formatAmount(b.bedrag)}</Bedrag>`);
      parts.push(`          <Omschrijving>${esc(desc)}</Omschrijving>`);
      parts.push('        </Boeking>');
    }
    parts.push('      </Boekingen>');
    parts.push('    </Mutatie>');
  }
  parts.push('  </Mutaties>');
  parts.push('</Xaf>');

  writeFileSync(out, parts.join('\n') + '\n');

  record(db, {
    actor, action: 'export.xaf', command: 'export xaf',
    args: { year, out }, outcome: 'ok',
  });

  return {
    ok: true,
    path: out,
    year,
    company: { name: company.name, registration_id: company.registration_id },
    rekeningen: accounts.length,
    mutaties: mutatieIds.length,
  };
}
