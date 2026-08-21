/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio init — create the company database (file, company row, default chart).
import { existsSync } from 'node:fs';
import { openDb } from '../core/db.js';
import { DEFAULT_CHART } from '../core/chart.js';
import { listAccounts, seedDefaultChart } from '../core/accounts.js';
import { record } from '../audit/index.js';
import { enableVatModule } from '../vat/index.js';
import { isValidIban } from '../core/iban.js';
import { dbError, ensureDb, makeCtx, output, fail, withDb } from './util.js';
import { getProfile } from '../jurisdictions/index.js';

const LEGAL_FORMS = ['eenmanszaak', 'vof', 'bv', 'nv', 'stichting', 'vereniging'];
const FY_END_RE = /^\d{2}-\d{2}$/;

export function make(program) {
  program
    .command('init')
    .description('initialise a company database (file, company row, default chart)')
    .requiredOption('--name <name>', 'company name')
    .option('--country <CC>', 'country (ISO 3166-1 alpha-2)', 'NL')
    .option('--registration-id <id>', 'company registration number (KVK for NL)')
    .option('--tax-id <id>', 'tax identification number (btw-id for NL)')
    .option('--kvk <kvk>', '[deprecated] alias for --registration-id')
    .option('--legal-form <form>', `legal form (${LEGAL_FORMS.join('|')})`, 'eenmanszaak')
    .option('--btw-id <id>', '[deprecated] alias for --tax-id')
    .option('--iban <iban>', 'bank account (IBAN)')
    .option('--address <address>', 'street address (for compliant invoices)')
    .option('--postal-code <code>', 'postal code')
    .option('--city <city>', 'city')
    .option('--vat <on|off>', 'enable the VAT module (Phase 2)', 'off')
    .option('--kor', 'small business scheme (KOR) — implies --vat off')
    .option('--fiscal-year-end <mm-dd>', "fiscal year end (default: the country profile's)")
    .option('--dry-run', 'show the plan without writing anything')
    // NB: not withDb — init manages its own database lifecycle inside initAction
    // (--dry-run must not create or open anything on disk)
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        initAction(ctx, opts);
      } catch (err) {
        fail(ctx, err);
      }
    });
}

function buildCompany(opts) {
  // jurisdiction profile: validates country (INVALID_COUNTRY /
  // COUNTRY_NOT_SUPPORTED) and drives legal forms, the KOR gate, and the
  // stored country/base_currency/locale/profile_version
  const profile = getProfile(opts.country ?? 'NL');
  // fiscal year end defaults from the country profile (NL 12-31, GB 03-31)
  if (opts.fiscalYearEnd === undefined) opts.fiscalYearEnd = profile.meta.defaultFiscalYearEnd;
  if (!profile.meta.legalForms.includes(opts.legalForm)) {
    throw dbError('INVALID_LEGAL_FORM', `legal form '${opts.legalForm}' must be one of ${profile.meta.legalForms.join(', ')}`);
  }
  if (opts.kor && profile.tax.smallBusinessScheme !== 'kor') {
    throw dbError('INVALID_VAT_CHOICE', `--kor is not available for country ${profile.meta.country}`);
  }
  if (opts.vat !== 'on' && opts.vat !== 'off') {
    throw dbError('INVALID_VAT_CHOICE', `--vat must be 'on' or 'off', got '${opts.vat}'`);
  }
  if (!FY_END_RE.test(opts.fiscalYearEnd)) {
    throw dbError('INVALID_FISCAL_YEAR_END', `fiscal year end '${opts.fiscalYearEnd}' must be mm-dd`);
  }
  // calendar check: 99-99 and 02-30 pass the regex but break the jaarrekening
  // as-of date and the compliance periods ('2026-99-99' sorts after every
  // real date → silently empty annual accounts)
  const [fmm, fdd] = opts.fiscalYearEnd.split('-').map(Number);
  const fye = new Date(Date.UTC(2000, fmm - 1, fdd));
  if (Number.isNaN(fye.getTime()) || fye.getUTCMonth() !== fmm - 1 || fye.getUTCDate() !== fdd) {
    throw dbError('INVALID_FISCAL_YEAR_END', `fiscal year end '${opts.fiscalYearEnd}' is not a valid calendar date`);
  }
  if (opts.iban && !isValidIban(opts.iban)) {
    throw dbError('INVALID_IBAN', `'${opts.iban}' is not a valid IBAN`);
  }
  // deprecated aliases: --kvk -> --registration-id, --btw-id -> --tax-id
  const warnings = [];
  if (opts.registrationId != null && opts.kvk != null) {
    warnings.push('--kvk ignored because --registration-id was also given');
  }
  if (opts.taxId != null && opts.btwId != null) {
    warnings.push('--btw-id ignored because --tax-id was also given');
  }
  const registrationId = opts.registrationId != null
    ? opts.registrationId
    : (opts.kvk != null ? (warnings.push('--kvk is deprecated — use --registration-id'), opts.kvk) : null);
  const taxId = opts.taxId != null
    ? opts.taxId
    : (opts.btwId != null ? (warnings.push('--btw-id is deprecated — use --tax-id'), opts.btwId) : null);
  const company = {
    name: opts.name,
    registration_id: registrationId,
    legal_form: opts.legalForm,
    tax_id: taxId,
    iban: opts.iban ?? null,
    address: opts.address ?? null,
    postal_code: opts.postalCode ?? null,
    city: opts.city ?? null,
    vat_module: opts.kor ? 0 : opts.vat === 'on' ? 1 : 0,
    kor_flag: opts.kor ? 1 : 0,
    fiscal_year_end: opts.fiscalYearEnd,
    country: profile.meta.country,
    base_currency: profile.meta.baseCurrency,
    locale: profile.meta.locale,
    profile_version: 1,
  };
  return { company, warnings };
}

function renderInit(data) {
  console.log(`company:  ${data.company.name} (${data.company.legal_form})`);
  console.log(`country:  ${data.company.country} (${data.company.base_currency}, ${data.company.locale})`);
  console.log(`reg-id:   ${data.company.registration_id ?? '-'}`);
  console.log(`tax-id:   ${data.company.tax_id ?? '-'}`);
  console.log(`vat:      ${data.company.vat_module ? 'on' : 'off'}${data.company.kor_flag ? ' (KOR)' : ''}`);
  console.log(`db:       ${data.db}`);
  console.log(`chart:    ${data.chart.accounts} accounts (default chart)`);
  if (data.chart.created != null) console.log(`seeded:   ${data.chart.created} new`);
  if (data.company.vat_module) {
    const profile = getProfile(data.company.country);
    const ledger = profile.tax.accounts?.ledger ?? [];
    if (ledger.length >= 2) {
      console.log(`vat:      module enabled (clearing ${ledger[0].code}/${ledger[1].code})`);
    } else {
      // no VAT ledger (e.g. the US profile: system 'none', ledger []) —
      // the module is on but there are no clearing accounts to name
      console.log('vat:      module enabled');
    }
  }
  for (const w of data.warnings ?? []) console.error(`warning: ${w}`);
  console.log(data.dryRun ? '(dry run — nothing written)' : 'initialised.');
}

function initAction(ctx, opts) {
  const { company, warnings } = buildCompany(opts);

  if (ctx.dryRun) {
    output(ctx, {
      action: 'create company + seed default chart',
      company,
      db: ctx.dbPath,
      db_exists: existsSync(ctx.dbPath),
      chart: { accounts: DEFAULT_CHART.length + (company.vat_module ? 2 : 0) },
      ...(warnings.length ? { warnings } : {}),
      dryRun: true,
    }, renderInit);
    return;
  }

  if (existsSync(ctx.dbPath)) {
    const existing = openDb(ctx.dbPath);
    const row = existing.prepare('SELECT id FROM company').get();
    existing.close();
    if (row) throw dbError('ALREADY_INITIALISED', `database ${ctx.dbPath} already has a company`);
  }

  const db = ensureDb(ctx, { create: true });
  try {
    db.prepare(
      `INSERT INTO company (name, registration_id, legal_form, tax_id, iban, address, postal_code, city, vat_module, kor_flag, fiscal_year_end, country, base_currency, locale, profile_version)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    ).run(company.name, company.registration_id, company.legal_form, company.tax_id, company.iban,
      company.address, company.postal_code, company.city,
      company.vat_module, company.kor_flag, company.fiscal_year_end,
      company.country, company.base_currency, company.locale, company.profile_version);
    const created = seedDefaultChart(db);
    let vatCreated = 0;
    if (company.vat_module) {
      const vatResult = enableVatModule(db, { actor: ctx.actor });
      vatCreated = vatResult.accounts.length;
    }
    const { _warnings, ...auditArgs } = company;
    record(db, {
      actor: ctx.actor, action: 'company.init', command: 'init',
      args: auditArgs, outcome: 'ok',
    });
    output(ctx, {
      company,
      db: ctx.dbPath,
      chart: { accounts: listAccounts(db).length, created: created + vatCreated },
      ...(warnings.length ? { warnings } : {}),
      dryRun: false,
    }, renderInit);
  } finally {
    db.close();
  }
}
