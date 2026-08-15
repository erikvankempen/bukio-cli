/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Items catalog (v0.13.0) — reusable products/services with a quantity unit,
// unit price, default VAT code and optional revenue account. Invoice lines
// snapshot item values at creation; later item edits never rewrite invoices.
import { record } from '../audit/index.js';
import { getAccountByCode } from '../core/accounts.js';
import { isVatEnabled, listVatCodes } from '../vat/index.js';
import { UNIT_CODES } from '../invoice/i18n.js';

export function itemError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

export function getItem(db, id) {
  return db.prepare('SELECT * FROM items WHERE id = ?').get(id) ?? null;
}

export function listItems(db, { activeOnly = true } = {}) {
  return db.prepare(
    `SELECT * FROM items ${activeOnly ? 'WHERE active = 1' : ''} ORDER BY name`,
  ).all();
}

function assertVatCode(db, vatCode) {
  if (!vatCode) return;
  if (!isVatEnabled(db)) {
    throw itemError('VAT_MODULE_OFF', 'item has a VAT code but the VAT module is off for this company');
  }
  const known = listVatCodes(db).some((c) => c.code === vatCode);
  if (!known) throw itemError('VAT_CODE_NOT_FOUND', `vat code '${vatCode}' does not exist`);
}

function assertAccount(db, glAccount) {
  if (!glAccount) return;
  if (!getAccountByCode(db, glAccount)) {
    throw itemError('ACCOUNT_NOT_FOUND', `account '${glAccount}' does not exist`);
  }
}

function validateItem({ name, unit, unitPriceCents, vatCode }) {
  if (!name || typeof name !== 'string' || !name.trim()) {
    throw itemError('INVALID_NAME', 'item needs a name');
  }
  if (!UNIT_CODES.includes(unit)) {
    throw itemError('INVALID_UNIT', `unit '${unit}' must be one of: ${UNIT_CODES.join(', ')}`);
  }
  if (!Number.isInteger(unitPriceCents) || unitPriceCents <= 0) {
    throw itemError('INVALID_PRICE', 'unit price must be positive cents');
  }
  // format check only — semantic validation against the ACTIVE profile's
  // code list happens downstream (VAT_CODE_NOT_FOUND). The regex accepts
  // dotted rates (FR '5.5'/'2.1') since the line-spec parser recognises
  // them; '21', 'V', 'R', 'RE' all still pass.
  if (vatCode != null && !/^[A-Z0-9]+(\.[0-9]{1,2})?$/.test(vatCode)) {
    throw itemError('INVALID_VAT_CODE', `vat code '${vatCode}' is malformed`);
  }
}

export function createItem(db, {
  name, description = null, unit = 'unit', unitPriceCents, vatCode = null,
  glAccount = null, actor = 'human', dryRun = false,
}) {
  validateItem({ name, unit, unitPriceCents, vatCode });
  assertVatCode(db, vatCode);
  assertAccount(db, glAccount);
  if (dryRun) {
    return {
      action: 'item.create', name, description, unit, unit_price_cents: unitPriceCents,
      vat_code: vatCode, gl_account: glAccount, dryRun: true,
    };
  }
  const info = db.prepare(`
    INSERT INTO items (name, description, unit, unit_price_cents, vat_code, gl_account, created_by)
    VALUES (?, ?, ?, ?, ?, ?, ?)
  `).run(name.trim(), description?.trim() ?? null, unit, unitPriceCents, vatCode, glAccount, actor);
  record(db, {
    actor, action: 'item.create', command: 'item add',
    args: { name: name.trim(), unit, unit_price_cents: unitPriceCents, vat_code: vatCode, gl_account: glAccount },
    outcome: 'ok',
  });
  return getItem(db, info.lastInsertRowid);
}

export function updateItem(db, {
  id, name = null, description = null, unit = null, unitPriceCents = null,
  vatCode = null, glAccount = null, deactivate = false, actor = 'human', dryRun = false,
}) {
  const existing = getItem(db, id);
  if (!existing) throw itemError('ITEM_NOT_FOUND', `item ${id} does not exist`);

  // empty strings mean "clear" for the optional fields — a caller passing
  // --vat '' or --gl '' must be able to UNSET them (previously the value
  // was kept for vatCode and '' was stored verbatim for glAccount, creating
  // a distinct 'gl_account ?? ""' group key downstream). Use a sentinel so
  // "not passed" (null → keep existing) is distinct from "explicitly clear".
  const CLEAR = Symbol('clear');
  const vat = vatCode === '' ? CLEAR : vatCode;
  const gl = glAccount === '' ? CLEAR : glAccount;
  const next = {
    name: name ?? existing.name,
    description: description !== null ? description : existing.description,
    unit: unit ?? existing.unit,
    unitPriceCents: unitPriceCents ?? existing.unit_price_cents,
    vatCode: vat === CLEAR ? null : vat ?? existing.vat_code,
    glAccount: gl === CLEAR ? null : gl ?? existing.gl_account,
    active: deactivate ? 0 : existing.active,
  };
  validateItem(next);
  assertVatCode(db, next.vatCode);
  assertAccount(db, next.glAccount);

  const changes = {
    name: next.name, description: next.description, unit: next.unit,
    unit_price_cents: next.unitPriceCents, vat_code: next.vatCode,
    gl_account: next.glAccount, active: next.active,
  };
  if (dryRun) {
    return { action: 'item.update', id, changes, dryRun: true };
  }
  db.prepare(`
    UPDATE items SET name = ?, description = ?, unit = ?, unit_price_cents = ?,
      vat_code = ?, gl_account = ?, active = ?, updated_by = ?,
      updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
    WHERE id = ?
  `).run(next.name, next.description, next.unit, next.unitPriceCents,
    next.vatCode, next.glAccount, next.active, actor, id);
  record(db, {
    actor, action: 'item.update', command: 'item update', args: { id, ...changes }, outcome: 'ok',
  });
  return getItem(db, id);
}
