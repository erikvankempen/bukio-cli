/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
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
  // XML 1.0 valid chars: strip control chars (0x00-0x08, 0x0B, 0x0C,
  // 0x0E-0x1F) that would make the document schema-invalid, then escape
  // the five special characters (same contract as the UBL builder)
  return String(s ?? '')
    .replace(/[\x00-\x08\x0B\x0C\x0E-\x1F]/g, '')
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
  // Phase B3: Luxembourg FAIA 2.01 reduced version B (accounting-only, no
  // namespace) — the AED electronic audit file. Spec: docs-research/lu-faia.md
  // (local XSD copies in docs-research/faia-src/).
  'faia-2.01-reduced-b': buildFaiaReducedB,
};

export function exportXaf(db, { year, out, actor = 'human', dryRun = false }) {
  const { documents } = resolveProfile(db);
  const builder = XAF_BUILDERS[documents.auditFile];
  if (!builder) {
    throw Object.assign(new Error(`audit file format '${documents.auditFile}' has no builder (registered: ${Object.keys(XAF_BUILDERS).join(', ')})`), { code: 'FORMAT_NOT_SUPPORTED' });
  }
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

/**
 * FAIA 2.01 reduced version B (Phase B3) — the Luxembourg AED audit file.
 *
 * Target: accounting-only systems (no invoicing module) — the schema WITHOUT
 * namespace, id 'FAIA'. Structure per the official XSD (docs-research/
 * faia-src/): AuditFile > Header (version 2.01, country LU, company with RCS
 * registration number + AED tax registration, selection = the CIVIL year per
 * the XSD annotation — truncated periods are not permitted) + MasterFiles >
 * GeneralLedgerAccounts (PCN accounts with French AccountType values
 * Actif/Passif/Produit/Charge and opening/closing balances — the balance
 * choices are REQUIRED per the XSD) + GeneralLedgerEntries (one Journal,
 * one Transaction per posted entry, per-posting Line with a
 * DebitAmount|CreditAmount pair — never signed amounts).
 *
 * The AED-issued tax registration number (TVA, LU+8) is reported as both
 * TaxRegistrationNumber and TaxNumber (the TVA number is the only AED number
 * the company row carries; the matricule has no schema field yet). Contact
 * person falls back to the company name (no contact field exists).
 */
function buildFaiaReducedB(db, { year, out, actor, dryRun }) {
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

  // FAIA requires a COMPLETE CIVIL year (XSD annotation: "It's not possible
  // to submit a FAIA with a period that is not matching the civil year") —
  // the selection is 01-01..12-31 of `year`, NOT the fiscal-year window.
  const selFrom = `${year}-01-01`;
  const selTo = `${year}-12-31`;

  const accounts = db.prepare('SELECT * FROM accounts ORDER BY code').all();
  const entries = db.prepare(`
    SELECT e.id, e.date, e.description, e.source, e.state, e.source_ref,
           e.created_by, e.posted_at
    FROM journal_entries e
    WHERE e.date >= ? AND e.date <= ?
    ORDER BY e.date, e.id
  `).all(selFrom, selTo);
  const postings = db.prepare(`
    SELECT p.id, p.entry_id, p.amount_cents, p.fx_currency, p.fx_amount_cents,
           a.code AS account_code
    FROM postings p
    JOIN accounts a ON a.id = p.account_id
    JOIN journal_entries e ON e.id = p.entry_id
    WHERE e.date >= ? AND e.date <= ?
    ORDER BY p.id
  `).all(selFrom, selTo);
  const byEntry = new Map();
  for (const p of postings) {
    if (!byEntry.has(p.entry_id)) byEntry.set(p.entry_id, []);
    byEntry.get(p.entry_id).push(p);
  }

  const posted = entries.filter((e) => e.state === 'posted');
  if (posted.length === 0) {
    const e = new Error(`no posted entries in ${year} — nothing to export`);
    e.code = 'EXPORT_EMPTY_YEAR';
    throw e;
  }

  // per-account opening (before 01-01) and closing (through 12-31) balances
  const balances = db.prepare(`
    SELECT a.code,
      COALESCE(SUM(CASE WHEN e.date < ? THEN p.amount_cents END), 0) AS opening,
      COALESCE(SUM(CASE WHEN e.date <= ? THEN p.amount_cents END), 0) AS closing
    FROM accounts a
    LEFT JOIN postings p ON p.account_id = a.id
    LEFT JOIN journal_entries e ON e.id = p.entry_id AND e.state = 'posted'
    GROUP BY a.code
  `).all(selFrom, selTo);
  const balanceByCode = new Map(balances.map((b) => [b.code, b]));

  if (dryRun) {
    return {
      ok: true, path: out, year, dryRun: true,
      company: { name: company.name, registration_id: company.registration_id },
      rekeningen: accounts.length,
      mutaties: posted.filter((e) => (byEntry.get(e.id) ?? []).length > 0).length,
    };
  }

  // French AccountType per the XSD annotation (Actif/Passif/Produit/Charge)
  const accountType = (type) => (type === 'asset' ? 'Actif'
    : (type === 'liability' || type === 'equity') ? 'Passif'
      : type === 'income' ? 'Produit' : 'Charge');
  const amount = (cents) => formatAmount(cents); // '117.00' — 2 fraction digits per FAIAmonetaryType
  const balanceEl = (side, cents) => {
    // the opening/closing balance choices are REQUIRED; zero is a debit 0.00
    if (cents >= 0) return `        <${side}DebitBalance>${amount(cents)}</${side}DebitBalance>`;
    return `        <${side}CreditBalance>${amount(-cents)}</${side}CreditBalance>`;
  };

  const parts = [];
  parts.push('<?xml version="1.0" encoding="UTF-8"?>');
  parts.push('<AuditFile>');
  parts.push('  <Header>');
  parts.push('    <AuditFileVersion>2.01</AuditFileVersion>');
  parts.push('    <AuditFileCountry>LU</AuditFileCountry>');
  parts.push(`    <AuditFileDateCreated>${todayIso()}</AuditFileDateCreated>`);
  parts.push('    <SoftwareCompanyName>bukio-cli</SoftwareCompanyName>');
  parts.push('    <SoftwareID>bukio-cli</SoftwareID>');
  parts.push(`    <SoftwareVersion>${esc(PACKAGE_VERSION)}</SoftwareVersion>`);
  parts.push('    <Company>');
  parts.push(`      <RegistrationNumber>${esc(company.registration_id || '')}</RegistrationNumber>`);
  parts.push(`      <Name>${esc(company.name)}</Name>`);
  parts.push('      <Address>');
  parts.push(`        <StreetName>${esc(company.address || '')}</StreetName>`);
  parts.push(`        <City>${esc(company.city || '')}</City>`);
  parts.push(`        <PostalCode>${esc(company.postal_code || '')}</PostalCode>`);
  parts.push('        <Country>LU</Country>');
  parts.push('      </Address>');
  parts.push('      <Contact>');
  // PersonNameStructure: the sequence starts with Title/FirstName — the
  // company name stands in for the (unavailable) contact person
  parts.push(`        <ContactPerson><FirstName>${esc(company.name)}</FirstName><LastName></LastName></ContactPerson>`);
  parts.push('        <Telephone></Telephone>');
  parts.push('      </Contact>');
  if (company.tax_id) {
    // the TVA number is the AED-issued registration number the company row
    // carries (the matricule has no schema field yet)
    parts.push('      <TaxRegistration>');
    parts.push(`        <TaxRegistrationNumber>${esc(company.tax_id)}</TaxRegistrationNumber>`);
    parts.push('        <TaxType>TVA</TaxType>');
    parts.push(`        <TaxNumber>${esc(company.tax_id)}</TaxNumber>`);
    parts.push('      </TaxRegistration>');
  }
  parts.push('    </Company>');
  parts.push(`    <DefaultCurrencyCode>${esc(company.base_currency || 'EUR')}</DefaultCurrencyCode>`);
  parts.push('    <SelectionCriteria>');
  parts.push(`      <SelectionStartDate>${selFrom}</SelectionStartDate>`);
  parts.push(`      <SelectionEndDate>${selTo}</SelectionEndDate>`);
  parts.push('    </SelectionCriteria>');
  parts.push('    <TaxAccountingBasis>Invoice Accounting</TaxAccountingBasis>');
  parts.push('  </Header>');

  parts.push('  <MasterFiles>');
  parts.push('    <GeneralLedgerAccounts>');
  for (const a of accounts) {
    const bal = balanceByCode.get(a.code) ?? { opening: 0, closing: 0 };
    parts.push('      <Account>');
    parts.push(`        <AccountID>${esc(a.code)}</AccountID>`);
    parts.push(`        <AccountDescription>${esc(a.name)}</AccountDescription>`);
    parts.push(`        <AccountType>${accountType(a.type)}</AccountType>`);
    parts.push(balanceEl('Opening', bal.opening));
    parts.push(balanceEl('Closing', bal.closing));
    parts.push('      </Account>');
  }
  parts.push('    </GeneralLedgerAccounts>');
  // TaxTable: the Company/TaxRegistration TaxType keyrefs this table — a
  // TaxRegistration without a matching TaxTableEntry fails XSD validation.
  // The table is only emitted when the company carries a TVA number: a
  // TVA-less company must not declare a TVA entry it cannot back with a
  // TaxRegistration (keyref consistency for the TVA-less edge case).
  const taxCodes = resolveProfile(db).tax.codes;
  if (company.tax_id) {
    parts.push('    <TaxTable>');
    parts.push('      <TaxTableEntry>');
    parts.push('        <TaxType>TVA</TaxType>');
    parts.push('        <Description>Taxe sur la valeur ajoutée</Description>');
    for (const c of taxCodes) {
      parts.push('        <TaxCodeDetails>');
      parts.push(`          <TaxCode>${esc(c.code)}</TaxCode>`);
      parts.push(`          <Description>${esc(c.description)}</Description>`);
      if (c.type === 'standard') {
        parts.push(`          <TaxPercentage>${(c.rateBp / 100).toFixed(2)}</TaxPercentage>`);
      }
      parts.push('          <Country>LU</Country>');
      parts.push('        </TaxCodeDetails>');
    }
    parts.push('      </TaxTableEntry>');
    parts.push('    </TaxTable>');
  }
  parts.push('  </MasterFiles>');

  let totalDebit = 0;
  let totalCredit = 0;
  parts.push('  <GeneralLedgerEntries>');
  parts.push(`    <NumberOfEntries>${posted.length}</NumberOfEntries>`);
  const allLines = [];
  for (const e of posted) {
    const rows = byEntry.get(e.id) ?? [];
    if (rows.length === 0) continue;
    for (const p of rows) allLines.push({ ...p, description: e.description });
  }
  for (const p of allLines) {
    if (p.amount_cents > 0) totalDebit += p.amount_cents;
    else totalCredit += -p.amount_cents;
  }
  parts.push(`    <TotalDebit>${amount(totalDebit)}</TotalDebit>`);
  parts.push(`    <TotalCredit>${amount(totalCredit)}</TotalCredit>`);
  parts.push('    <Journal>');
  parts.push('      <JournalID>1</JournalID>');
  parts.push('      <Description>Journal général</Description>');
  parts.push('      <Type>GR</Type>');
  for (const e of posted) {
    const rows = byEntry.get(e.id) ?? [];
    if (rows.length === 0) continue;
    const d = String(e.date).slice(0, 10);
    const mm = Number(d.slice(5, 7));
    const postedDate = String(e.posted_at ?? '').slice(0, 10) || d;
    const desc = e.description || `Écriture ${e.id}`;
    parts.push('      <Transaction>');
    parts.push(`        <TransactionID>${e.id}</TransactionID>`);
    parts.push(`        <Period>${mm}</Period>`);
    parts.push(`        <PeriodYear>${d.slice(0, 4)}</PeriodYear>`);
    parts.push(`        <TransactionDate>${d}</TransactionDate>`);
    parts.push(`        <Description>${esc(desc)}</Description>`);
    parts.push(`        <SystemEntryDate>${postedDate}</SystemEntryDate>`);
    parts.push(`        <GLPostingDate>${postedDate}</GLPostingDate>`);
    for (const p of rows) {
      parts.push('        <Line>');
      parts.push(`          <RecordID>${p.id}</RecordID>`);
      parts.push(`          <AccountID>${esc(p.account_code)}</AccountID>`);
      parts.push(`          <Description>${esc(desc)}</Description>`);
      if (p.amount_cents > 0) {
        parts.push(`          <DebitAmount><Amount>${amount(p.amount_cents)}</Amount></DebitAmount>`);
      } else if (p.amount_cents < 0) {
        parts.push(`          <CreditAmount><Amount>${amount(-p.amount_cents)}</Amount></CreditAmount>`);
      } else {
        parts.push('          <DebitAmount><Amount>0.00</Amount></DebitAmount>');
      }
      parts.push('        </Line>');
    }
    parts.push('      </Transaction>');
  }
  parts.push('    </Journal>');
  parts.push('  </GeneralLedgerEntries>');
  parts.push('</AuditFile>');

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
    mutaties: allLines.length ? posted.length : 0,
  };
}
