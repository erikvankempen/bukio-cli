// bukio init — create the company database (file, company row, default chart).
import { existsSync } from 'node:fs';
import { openDb } from '../core/db.js';
import { DEFAULT_CHART } from '../core/chart.js';
import { listAccounts, seedDefaultChart } from '../core/accounts.js';
import { record } from '../audit/index.js';
import { enableVatModule } from '../vat/index.js';
import { dbError, ensureDb, makeCtx, output, fail } from './util.js';

const LEGAL_FORMS = ['eenmanszaak', 'vof', 'bv', 'nv', 'stichting', 'vereniging'];
const FY_END_RE = /^\d{2}-\d{2}$/;

export function make(program) {
  program
    .command('init')
    .description('initialise a company database (file, company row, default chart)')
    .requiredOption('--name <name>', 'company name')
    .option('--kvk <kvk>', 'KVK number')
    .option('--legal-form <form>', `legal form (${LEGAL_FORMS.join('|')})`, 'eenmanszaak')
    .option('--btw-id <id>', 'BTW identification number')
    .option('--iban <iban>', 'bank account (IBAN)')
    .option('--vat <on|off>', 'enable the VAT module (Phase 2)', 'off')
    .option('--kor', 'small business scheme (KOR) — implies --vat off')
    .option('--fiscal-year-end <mm-dd>', 'fiscal year end', '12-31')
    .option('--dry-run', 'show the plan without writing anything')
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
  if (!LEGAL_FORMS.includes(opts.legalForm)) {
    throw dbError('INVALID_LEGAL_FORM', `legal form '${opts.legalForm}' must be one of ${LEGAL_FORMS.join(', ')}`);
  }
  if (!FY_END_RE.test(opts.fiscalYearEnd)) {
    throw dbError('INVALID_FISCAL_YEAR_END', `fiscal year end '${opts.fiscalYearEnd}' must be mm-dd`);
  }
  return {
    name: opts.name,
    kvk: opts.kvk ?? null,
    legal_form: opts.legalForm,
    btw_id: opts.btwId ?? null,
    iban: opts.iban ?? null,
    vat_module: opts.kor ? 0 : opts.vat === 'on' ? 1 : 0,
    kor_flag: opts.kor ? 1 : 0,
    fiscal_year_end: opts.fiscalYearEnd,
  };
}

function renderInit(data) {
  console.log(`company:  ${data.company.name} (${data.company.legal_form})`);
  console.log(`kvk:      ${data.company.kvk ?? '-'}`);
  console.log(`btw-id:   ${data.company.btw_id ?? '-'}`);
  console.log(`vat:      ${data.company.vat_module ? 'on' : 'off'}${data.company.kor_flag ? ' (KOR)' : ''}`);
  console.log(`db:       ${data.db}`);
  console.log(`chart:    ${data.chart.accounts} accounts (default chart)`);
  if (data.chart.created != null) console.log(`seeded:   ${data.chart.created} new`);
  if (data.company.vat_module) console.log('vat:      module enabled (incl. 1500/2500)');
  console.log(data.dryRun ? '(dry run — nothing written)' : 'initialised.');
}

function initAction(ctx, opts) {
  const company = buildCompany(opts);

  if (ctx.dryRun) {
    output(ctx, {
      action: 'create company + seed default chart',
      company,
      db: ctx.dbPath,
      db_exists: existsSync(ctx.dbPath),
      chart: { accounts: DEFAULT_CHART.length + (company.vat_module ? 2 : 0) },
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
      `INSERT INTO company (name, kvk, legal_form, btw_id, iban, vat_module, kor_flag, fiscal_year_end)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
    ).run(company.name, company.kvk, company.legal_form, company.btw_id, company.iban,
      company.vat_module, company.kor_flag, company.fiscal_year_end);
    const created = seedDefaultChart(db);
    let vatCreated = 0;
    if (company.vat_module) {
      const vatResult = enableVatModule(db, { actor: ctx.actor });
      vatCreated = vatResult.accounts.length;
    }
    record(db, {
      actor: ctx.actor, action: 'company.init', command: 'init',
      args: company, outcome: 'ok',
    });
    output(ctx, {
      company,
      db: ctx.dbPath,
      chart: { accounts: listAccounts(db).length, created: created + vatCreated },
      dryRun: false,
    }, renderInit);
  } finally {
    db.close();
  }
}
