/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// bukio company — show the company record, update company details.
import { t, resolveLocale } from '../i18n/index.js';
import { readFileSync, writeFileSync } from 'node:fs';
import {ensureDb, makeCtx, output, fail, table, dbError} from './util.js';
import { isValidIban } from '../core/iban.js';
import { record } from '../audit/index.js';

const LOGO_MAX_BYTES = 1_000_000; // 1 MB
const LOGO_MAX_DIMENSION = 2048;  // px, width AND height

/** Parse PNG/JPEG dimensions from the file header (SVG from width/height/viewBox). */
function logoDimensions(mime, bytes) {
  if (mime === 'image/png' && bytes.length >= 24) {
    return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) };
  }
  if (mime === 'image/jpeg') {
    let i = 2;
    while (i + 9 < bytes.length) {
      if (bytes[i] !== 0xff) { i++; continue; }
      const marker = bytes[i + 1];
      if (marker >= 0xc0 && marker <= 0xcf && ![0xc4, 0xc8, 0xcc].includes(marker)) {
        return { width: bytes.readUInt16BE(i + 7), height: bytes.readUInt16BE(i + 5) };
      }
      i += 2 + bytes.readUInt16BE(i + 2);
    }
    return null;
  }
  if (mime === 'image/svg+xml') {
    const head = bytes.subarray(0, 4096).toString('utf8');
    const viewBox = head.match(/viewBox=["']\s*[\d.-]+\s+[\d.-]+\s+([\d.]+)\s+([\d.]+)\s*["']/);
    if (viewBox) return { width: Math.ceil(Number(viewBox[1])), height: Math.ceil(Number(viewBox[2])) };
    const w = head.match(/width=["']\s*([\d.]+)/);
    const h = head.match(/height=["']\s*([\d.]+)/);
    if (w && h) return { width: Math.ceil(Number(w[1])), height: Math.ceil(Number(h[1])) };
    return null; // no parsable dimensions — accept, renders at natural size
  }
  return null;
}

/**
 * Validate a logo file: PNG / JPEG / SVG only, max 1 MB, max 2048×2048 px.
 * Returns { mime, bytes, width, height } or throws LOGO_* errors.
 */
function readLogo(file) {
  let bytes;
  try {
    bytes = readFileSync(file);
  } catch (err) {
    if (err.code === 'ENOENT') {
      throw Object.assign(new Error(`logo file '${file}' not found`), { code: 'LOGO_FILE_NOT_FOUND' });
    }
    throw err;
  }
  if (bytes.length > LOGO_MAX_BYTES) {
    throw Object.assign(new Error(`logo file is ${bytes.length} bytes — the maximum is ${LOGO_MAX_BYTES}`), { code: 'LOGO_TOO_LARGE' });
  }
  let mime = null;
  if (bytes.length >= 8 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) {
    mime = 'image/png';
  } else if (bytes.length >= 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) {
    mime = 'image/jpeg';
  } else {
    // scan a wide window: XML declarations and comment blocks push <svg deep
    const head = bytes.subarray(0, 4096).toString('utf8').replace(/^\uFEFF/, '').trimStart();
    const svgish = head.startsWith('<svg') || (head.startsWith('<?xml') && head.includes('<svg'));
    if (svgish) mime = 'image/svg+xml';
  }
  if (!mime) {
    throw Object.assign(new Error('unsupported logo format — use PNG, JPEG or SVG'), { code: 'LOGO_UNSUPPORTED_FORMAT' });
  }
  const dims = logoDimensions(mime, bytes);
  if (dims && (dims.width > LOGO_MAX_DIMENSION || dims.height > LOGO_MAX_DIMENSION)) {
    throw Object.assign(
      new Error(`logo is ${dims.width}×${dims.height} px — the maximum is ${LOGO_MAX_DIMENSION}×${LOGO_MAX_DIMENSION}`),
      { code: 'LOGO_DIMENSIONS_TOO_LARGE' },
    );
  }
  return { mime, bytes, width: dims?.width ?? null, height: dims?.height ?? null };
}

const COMPANY_FIELDS = [
  ['name', 'name', 'company name'],
  ['registrationId', 'registration_id', 'registration id (KVK for NL)'],
  ['taxId', 'tax_id', 'tax id (btw-id for NL)'],
  ['iban', 'iban', 'bank account (IBAN)'],
  ['address', 'address', 'street address (for compliant invoices)'],
  ['postalCode', 'postal_code', 'postal code'],
  ['city', 'city', 'city'],
];

function serializeCompany(row) {
  return {
    id: row.id, name: row.name, registration_id: row.registration_id, legal_form: row.legal_form,
    tax_id: row.tax_id, iban: row.iban, address: row.address,
    postal_code: row.postal_code, city: row.city, vat_module: row.vat_module,
    kor_flag: row.kor_flag, fiscal_year_end: row.fiscal_year_end,
    country: row.country, base_currency: row.base_currency, locale: row.locale,
    profile_version: row.profile_version,
    logo_mime: row.logo_mime ?? null,
    logo_bytes: row.logo ? row.logo.length : null,
  };
}

function getCompany(db) {
  return db.prepare('SELECT * FROM company WHERE id = 1').get() ?? null;
}

export function make(program) {
  const company = program.command('company').description('company record: show, update');

  company
    .command('show')
    .description('show the company record')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        const locale = resolveLocale(ctx, db);
        try {
          const row = getCompany(db);
          if (!row) throw dbError('NO_COMPANY', 'no company — run bukio init first');
          output(ctx, { company: serializeCompany(row) }, (d) => {
            table([d.company], [
              { key: 'name', label: t('company.name', {}, locale) },
              { key: 'country', label: t('company.country', {}, locale) },
              { key: 'legal_form', label: t('company.legalForm', {}, locale) },
              { key: 'registration_id', label: t('company.regId', {}, locale) },
              { key: 'tax_id', label: t('company.taxId', {}, locale) },
              { key: 'iban', label: t('company.iban', {}, locale) },
              { key: 'address', label: t('company.address', {}, locale) },
              { key: 'postal_code', label: t('company.postalCode', {}, locale) },
              { key: 'city', label: t('company.city', {}, locale) },
              { key: 'base_currency', label: t('company.currency', {}, locale) },
              { key: 'locale', label: t('company.language', {}, locale) },
            ]);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  company
    .command('update')
    .description('update company details (address, IBAN, btw-id, name, kvk, logo)')
    .option('--name <name>', 'company name')
    .option('--registration-id <id>', 'company registration number (KVK for NL)')
    .option('--tax-id <id>', 'tax identification number (btw-id for NL)')
    .option('--kvk <kvk>', '[deprecated] alias for --registration-id')
    .option('--btw-id <id>', '[deprecated] alias for --tax-id')
    .option('--country <CC>', 'country — immutable after init (rejected)')
    .option('--iban <iban>', 'bank account (IBAN)')
    .option('--address <address>', 'street address (for compliant invoices)')
    .option('--postal-code <code>', 'postal code')
    .option('--city <city>', 'city')
    .option('--logo <file>', 'set the invoice logo (PNG/JPEG/SVG, max 1 MB, stored in the database)')
    .option('--remove-logo', 'remove the logo')
    .option('--dry-run', 'show the plan without writing')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const row = getCompany(db);
          if (!row) throw dbError('NO_COMPANY', 'no company — run bukio init first');

          // deprecated aliases: --kvk -> --registration-id, --btw-id -> --tax-id
          if (opts.kvk !== undefined && opts.registrationId !== undefined) {
            opts._warnings = [...(opts._warnings ?? []), '--kvk ignored because --registration-id was also given'];
          } else if (opts.kvk !== undefined) {
            opts.registrationId = opts.kvk;
            opts._warnings = [...(opts._warnings ?? []), '--kvk is deprecated — use --registration-id'];
          }
          if (opts.btwId !== undefined && opts.taxId !== undefined) {
            opts._warnings = [...(opts._warnings ?? []), '--btw-id ignored because --tax-id was also given'];
          } else if (opts.btwId !== undefined) {
            opts.taxId = opts.btwId;
            opts._warnings = [...(opts._warnings ?? []), '--btw-id is deprecated — use --tax-id'];
          }
          // country is immutable after init (decision §9.1.5)
          if (opts.country !== undefined) {
            const want = String(opts.country).trim().toUpperCase();
            if (want !== (row.country ?? 'NL')) {
              throw Object.assign(new Error(`country is immutable after init — company stays ${row.country ?? 'NL'} (re-init a new DB for another country)`), { code: 'COUNTRY_IMMUTABLE' });
            }
          }
          const changes = {};
          for (const [opt, col, label] of COMPANY_FIELDS) {
            if (opts[opt] !== undefined) changes[col] = String(opts[opt]).trim();
          }
          let logo = null;
          if (opts.logo !== undefined) logo = readLogo(opts.logo);
          if (opts.removeLogo) logo = { mime: null, bytes: null };
          if (Object.keys(changes).length === 0 && logo === null) {
            throw Object.assign(new Error('nothing to update — pass at least one of --name/--registration-id/--tax-id/--iban/--address/--postal-code/--city/--logo/--remove-logo'), { code: 'NOTHING_TO_UPDATE' });
          }
          for (const [opt, col, label] of COMPANY_FIELDS) {
            if (changes[col] === '' && col !== 'tax_id') {
              throw Object.assign(new Error(`${label} cannot be empty`), { code: 'INVALID_VALUE' });
            }
          }
          if (changes.iban && !isValidIban(changes.iban)) {
            throw Object.assign(new Error(`invalid IBAN '${changes.iban}'`), { code: 'INVALID_IBAN' });
          }

          const plan = {
            company: { ...serializeCompany(row), ...changes },
            changes,
            logo: logo === null ? null
              : { mime: logo.mime, bytes: logo.bytes ? logo.bytes.length : 0, width: logo.width, height: logo.height },
            ...(opts._warnings?.length ? { warnings: opts._warnings } : {}),
            dryRun: true,
          };
          if (ctx.dryRun) {
            output(ctx, plan, (d) => {
              console.log('plan: company update');
              for (const [k, v] of Object.entries(d.changes)) console.log(`  ${k}: '${row[k]}' -> '${v}'`);
              if (d.logo) console.log(`  logo: ${d.logo.mime} (${d.logo.bytes} bytes${d.logo.width ? `, ${d.logo.width}×${d.logo.height} px` : ''})${d.logo.bytes === 0 ? ' — removed' : ''}`);
              for (const w of d.warnings ?? []) console.error(`warning: ${w}`);
              console.log('(dry run — nothing written)');
            });
            return;
          }

          const sets = Object.keys(changes).map((c) => `${c} = ?`).join(', ');
          if (sets) {
            db.prepare(`UPDATE company SET ${sets} WHERE id = 1`).run(...Object.values(changes));
          }
          if (logo !== null) {
            db.prepare('UPDATE company SET logo = ?, logo_mime = ? WHERE id = 1')
              .run(logo.bytes, logo.mime);
          }
          const updated = getCompany(db);
          record(db, {
            actor: ctx.actor, action: 'company.update', command: 'company update',
            args: {
              ...changes,
              ...(logo !== null ? { logo: logo.mime ? `${logo.mime} (${logo.bytes.length} bytes)` : 'removed' } : {}),
            },
            outcome: 'ok',
          });
          output(ctx, {
            company: serializeCompany(updated), changes,
            ...(opts._warnings?.length ? { warnings: opts._warnings } : {}),
          }, (d) => {
            console.log('company updated:');
            for (const [k, v] of Object.entries(d.changes)) console.log(`  ${k}: '${row[k]}' -> '${v}'`);
            if (logo !== null && logo.mime) console.log(`  logo: ${logo.mime} (${logo.bytes.length} bytes)`);
            if (logo !== null && !logo.mime) console.log('  logo: removed');
            for (const w of d.warnings ?? []) console.error(`warning: ${w}`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });

  company
    .command('logo')
    .description('extract the stored logo to a file')
    .requiredOption('--out <file>', 'output file path')
    .action((opts, command) => {
      const ctx = makeCtx(command);
      try {
        const db = ensureDb(ctx);
        try {
          const row = getCompany(db);
          if (!row?.logo) throw Object.assign(new Error('no logo stored — set one with company update --logo'), { code: 'LOGO_NOT_SET' });
          writeFileSync(opts.out, row.logo);
          output(ctx, { out: opts.out, mime: row.logo_mime, bytes: row.logo.length }, (d) => {
            console.log(`logo written: ${d.out} (${d.mime}, ${d.bytes} bytes)`);
          });
        } finally {
          db.close();
        }
      } catch (err) {
        fail(ctx, err);
      }
    });
}
