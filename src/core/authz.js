/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Tier 0.5 per-actor authorizations (segregation of duties): the
// role→capability map, command→capability mapping and the authz gate.
//
// Design (memo ~/memos/bukio/actor-authorizations-plan.md): capability
// FAMILIES + roles, NOT a per-command matrix. SoD lives at the
// business-action level — create vs post vs file vs pay vs close. The
// role→capability map is CODE (versioned with the binary), not DB. The
// gate is one lookup: authzMode on && !canAct(actor, capabilityOf(cmd))
// → AUTHZ_DENIED before anything is written.
//
// Fail-closed: a command with NO capability mapping denies under authz
// (an unmapped command is a mapping bug, and the safe failure is a
// refusal, never a silent grant). The Task-2 coverage test asserts every
// documented command maps to exactly one capability.

import { getRoles, hasRole, getAuthz } from './actor-registry.js';

export function authzError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

// --- roles → capabilities ---------------------------------------------------

export const ROLES = ['owner', 'bookkeeper', 'payments', 'tax', 'assets', 'readonly'];

/** Capability families (v1). Key = capability, value = human description. */
export const CAPABILITY_DESCRIPTIONS = {
  'admin.company': 'company lifecycle + imports (init, company update/logo, bukio update, all imports)',
  'admin.chart': 'chart of accounts changes (account add/import/deactivate/reactivate)',
  'admin.actor': 'identity & permission management (roles grant/revoke/list, authz on/off, owner revoke, enforce on/off)',
  'admin.backup': 'backup, restore, attach',
  'entry.draft': 'create drafts & catalogues (entry add without --post, item add/update)',
  'entry.post': 'irreversible ledger movement (entry post, entry reverse, entry add --post)',
  'contacts.manage': 'contact data (contact add/update)',
  'invoice.manage': 'receivables (invoice create/finalize/pdf/ubl/credit/email/reminders/pay/peppol-send)',
  'bank.import': 'bank data in (bank add/import/ignore/unignore)',
  'bank.match': 'turning bank transactions into entries (bank match auto/suggest/link/post)',
  'payments.sepa': 'money OUT (payments mandate/payables/batch create/export/delete)',
  'vat.book': 'VAT bookkeeping (vat book/readout, icp readout, compliance status/mark)',
  'vat.file': 'tax filing & settlement (vat file, vat settle)',
  'assets.manage': 'fixed assets (assets add/dispose/register/run/scheme, depreciation add)',
  'recurring.manage': 'recurring entries (recurring add/pause/resume/run)',
  'close.month': 'month close (month-end)',
  'close.year': 'year close (year-end status/close, jaarrekening)',
  'export.manage': 'data leaving the system (export xaf)',
  'fx.manage': 'FX rate control (fx fetch/set)',
  'report.read': 'read-only (all reports, audit, audit verify, list/show commands, contact statement)',
};

/**
 * Role → capabilities. `owner` = null → everything (checked in canAct).
 * Keep in sync with AGENTS.md §6.21's role table.
 */
export const ROLE_CAPABILITIES = {
  owner: null,
  bookkeeper: [
    'admin.chart', 'entry.draft', 'entry.post', 'contacts.manage', 'invoice.manage',
    'bank.import', 'bank.match', 'vat.book', 'assets.manage', 'recurring.manage',
    'fx.manage', 'report.read',
  ],
  payments: ['bank.import', 'bank.match', 'payments.sepa', 'report.read'],
  tax: ['vat.book', 'vat.file', 'close.month', 'close.year', 'export.manage', 'report.read'],
  assets: ['assets.manage', 'report.read'],
  readonly: ['report.read'],
};

/**
 * Documented SoD conflict pairs. Soft warning on grant — legitimate
 * single-operator setups exist (the owner has everything anyway); the
 * warning keeps the review in front of the eye. The owner role is exempt.
 */
export const SOD_PAIRS = [
  { roles: ['bookkeeper', 'payments'], message: 'bookkeeper + payments: the same actor books AND authorises money out' },
  { roles: ['bookkeeper', 'tax'], message: 'bookkeeper + tax: the same actor books AND files tax' },
  { roles: ['payments', 'tax'], message: 'payments + tax: the same actor authorises money out AND files tax' },
];

/** Capability-level pair — the strongest split (moves ledger & money out). */
export const SOD_CAPABILITY_PAIR = {
  capabilities: ['entry.post', 'payments.sepa'],
  message: 'entry.post + payments.sepa: the same actor moves the ledger AND authorises money out (strongest pair)',
};

/**
 * @param {Array<string>} roles - the resulting role set of an actor.
 * @returns {Array<string>} human-readable SoD warnings (empty = no conflict).
 *   The owner role is exempt by definition (the operator is allowed
 *   everything — warning on owner would be noise on every grant).
 */
export function sodWarnings(roles) {
  if (roles.includes('owner')) return [];
  const roleSet = new Set(roles);
  const caps = new Set();
  for (const r of roles) {
    if (r === 'owner') continue;
    for (const c of ROLE_CAPABILITIES[r] ?? []) caps.add(c);
  }
  const warnings = [];
  for (const pair of SOD_PAIRS) {
    if (pair.roles.every((r) => roleSet.has(r))) warnings.push(pair.message);
  }
  if (SOD_CAPABILITY_PAIR.capabilities.every((c) => caps.has(c))) {
    warnings.push(SOD_CAPABILITY_PAIR.message);
  }
  return warnings;
}

// --- command → capability ---------------------------------------------------

/** CLI command path → capability. `null` = resolved dynamically (entry add). */
const CLI_CAPABILITIES = {
  // admin.company — company lifecycle + imports
  init: 'admin.company',
  'company update': 'admin.company',
  'company logo': 'admin.company',
  update: 'admin.company',
  'import opening-balances': 'admin.company',
  'import journal': 'admin.company',
  'import xaf': 'admin.company',
  'import invoice': 'admin.company',
  'import contacts': 'admin.company',
  'vat enable': 'admin.company',
  // admin.chart
  'account add': 'admin.chart',
  'account import': 'admin.chart',
  'account deactivate': 'admin.chart',
  'account reactivate': 'admin.chart',
  // admin.actor (effectively owner-only: no non-owner role carries it)
  'actor roles grant': 'admin.actor',
  'actor roles revoke': 'admin.actor',
  'actor roles': 'admin.actor',
  'actor can': 'admin.actor',
  'actor who-can': 'admin.actor',
  'actor authz': 'admin.actor',
  'actor enforce': 'admin.actor',
  'actor list': 'admin.actor',
  'actor revoke': 'admin.actor',
  // admin.backup
  backup: 'admin.backup',
  restore: 'admin.backup',
  'attach add': 'admin.backup',
  'attach remove': 'admin.backup',
  // entry
  'entry add': null, // resolved by opts.post: entry.post | entry.draft
  'entry post': 'entry.post',
  'entry reverse': 'entry.post',
  'item add': 'entry.draft',
  'item update': 'entry.draft',
  // contacts / receivables
  'contact add': 'contacts.manage',
  'contact update': 'contacts.manage',
  'invoice create': 'invoice.manage',
  'invoice finalize': 'invoice.manage',
  'invoice pdf': 'invoice.manage',
  'invoice ubl': 'invoice.manage',
  'invoice credit': 'invoice.manage',
  'invoice email': 'invoice.manage',
  'invoice reminders': 'invoice.manage',
  'invoice pay': 'invoice.manage',
  'invoice peppol-send': 'invoice.manage',
  // bank
  'bank add': 'bank.import',
  'bank import': 'bank.import',
  'bank ignore': 'bank.import',
  'bank unignore': 'bank.import',
  'bank match auto': 'bank.match',
  'bank match suggest': 'bank.match',
  'bank match link': 'bank.match',
  'bank match post': 'bank.match',
  // payments (money OUT)
  'payments payables add': 'payments.sepa',
  'payments payables pay': 'payments.sepa',
  'payments mandate add': 'payments.sepa',
  'payments mandate remove': 'payments.sepa',
  'payments batch create': 'payments.sepa',
  'payments batch export': 'payments.sepa',
  'payments batch delete': 'payments.sepa',
  // VAT
  'vat book': 'vat.book',
  'vat readout': 'vat.book',
  'icp readout': 'vat.book',
  'compliance status': 'vat.book',
  'compliance mark': 'vat.book',
  'vat file': 'vat.file',
  'vat settle': 'vat.file',
  // assets / recurring
  'assets scheme add': 'assets.manage',
  'assets add': 'assets.manage',
  'assets run': 'assets.manage',
  'assets register': 'assets.manage',
  'assets dispose': 'assets.manage',
  'assets pause': 'assets.manage',
  'assets resume': 'assets.manage',
  'depreciation add': 'assets.manage',
  'recurring add': 'recurring.manage',
  'recurring pause': 'recurring.manage',
  'recurring resume': 'recurring.manage',
  'recurring run': 'recurring.manage',
  // closes
  'month-end': 'close.month',
  'year-end status': 'close.year',
  'year-end close': 'close.year',
  'year-end report': 'close.year',
  'jaarrekening report': 'close.year',
  // export / fx
  'export xaf': 'export.manage',
  'fx set': 'fx.manage',
  'fx fetch': 'fx.manage',
  // report.read — all reads
  'report trial-balance': 'report.read',
  'report balance-sheet': 'report.read',
  'report balans': 'report.read',
  'report pnl': 'report.read',
  'report journal': 'report.read',
  'report aging': 'report.read',
  'report sales': 'report.read',
  audit: 'report.read',
  'audit verify': 'report.read',
  'company show': 'report.read',
  'attach list': 'report.read',
  'attach show': 'report.read',
  'account list': 'report.read',
  'account show': 'report.read',
  'entry list': 'report.read',
  'entry show': 'report.read',
  'item list': 'report.read',
  'item show': 'report.read',
  'contact list': 'report.read',
  'contact statement': 'report.read',
  'invoice list': 'report.read',
  'invoice show': 'report.read',
  'bank list': 'report.read',
  'bank transactions': 'report.read',
  'payments payables list': 'report.read',
  'payments mandate list': 'report.read',
  'payments batch list': 'report.read',
  'payments batch show': 'report.read',
  'vat codes': 'report.read',
  'assets scheme list': 'report.read',
  'assets list': 'report.read',
  'assets show': 'report.read',
  'recurring preview': 'report.read',
  'recurring list': 'report.read',
  'recurring show': 'report.read',
  'fx list': 'report.read',
  'fx show': 'report.read',
};

/** MCP tool → capability. `null` = resolved dynamically (entry_add). */
const MCP_CAPABILITIES = {
  company_info: 'report.read',
  trial_balance: 'report.read',
  balance_sheet: 'report.read',
  pnl: 'report.read',
  journal: 'report.read',
  accounts: 'report.read',
  audit: 'report.read',
  compliance: 'vat.book',
  invoices: 'report.read',
  entry_add: null, // post ? entry.post : entry.draft
  entry_post: 'entry.post',
  entry_reverse: 'entry.post',
  vat_book: 'vat.book',
  vat_readout: 'vat.book',
  icp_readout: 'vat.book',
  invoice_create: 'invoice.manage',
  invoice_finalize: 'invoice.manage',
  invoice_credit: 'invoice.manage',
  invoice_pay: 'invoice.manage',
  invoice_email: 'invoice.manage',
  invoice_import: 'admin.company',
  item_add: 'entry.draft',
  item_update: 'entry.draft',
  item_list: 'report.read',
  attachment_add: 'admin.backup',
  attachment_remove: 'admin.backup',
  attachment_list: 'report.read',
  report_aging: 'report.read',
  report_sales: 'report.read',
  payments_mandate_add: 'payments.sepa',
  payments_mandate_list: 'report.read',
  payments_batch_create: 'payments.sepa',
  payments_batch_export: 'payments.sepa',
  recurring_run: 'recurring.manage',
  year_end_close: 'close.year',
  year_end_status: 'close.year',
  fx_set: 'fx.manage',
  contact_add: 'contacts.manage',
  assets_register: 'assets.manage',
  asset_add: 'assets.manage',
  assets_run: 'assets.manage',
  asset_dispose: 'assets.manage',
};

/**
 * Map a command path (CLI 'entry add' or MCP 'mcp:entry_add') to its
 * capability, using the ACTUAL mutation (opts.post → entry.post).
 *
 * @param {string} cmd - command path as stored on audit rows.
 * @param {object} [opts={}] - signed args (CLI opts or MCP tool args).
 * @returns {string|null} capability, or null when unmapped (fail closed).
 */
export function capabilityOf(cmd, opts = {}) {
  if (cmd.startsWith('mcp:')) {
    const tool = cmd.slice(4);
    if (tool === 'entry_add') return opts.post === true ? 'entry.post' : 'entry.draft';
    return MCP_CAPABILITIES[tool] ?? null;
  }
  if (cmd === 'entry add') return opts.post === true ? 'entry.post' : 'entry.draft';
  return CLI_CAPABILITIES[cmd] ?? null;
}

// --- the authz gate ---------------------------------------------------------

/**
 * "May this actor perform this capability in this company?" — owner passes
 * everything; any other role passes when it carries the capability.
 *
 * @param {object} db
 * @param {string} actor
 * @param {string|null} capability
 * @returns {boolean}
 */
export function canAct(db, actor, capability) {
  if (!capability) return false; // fail closed: unmapped = denied
  for (const role of getRoles(db, actor)) {
    if (role === 'owner') return true;
    if (ROLE_CAPABILITIES[role]?.includes(capability)) return true;
  }
  return false;
}

/**
 * Commands that are NOT authz-checked even when authz is on. Mirrors the
 * signing-exempt set (keygen/unlock/lock/mcp — pure bootstrap / bridge)
 * plus the Tier 0.5 self-service commands (D7/D10): `actor register`
 * (self-service enrolment), `actor verify`, `actor roles`/`actor can` for
 * SELF, and SELF-revoke. Listing/granting OTHERS is owner territory.
 *
 * @param {string} cmd - command path.
 * @param {object} [args={}] - signed args.
 * @returns {boolean}
 */
export function isAuthzExempt(cmd, args = {}) {
  if (['actor keygen', 'actor unlock', 'actor lock', 'mcp', 'server start', 'server token'].includes(cmd)) return true;
  if (cmd === 'actor register') return true;
  if (cmd === 'actor verify') return true;
  // NOTE: the "other actor" option is `--for <who>` (the identity flag
  // `--actor` is the root commander option and cannot be re-declared on a
  // subcommand — commander would silently bind the value to the root and
  // corrupt the signing identity). args.for is undefined for self-checks.
  if (cmd === 'actor roles' && !args.for) return true; // own roles
  if (cmd === 'actor can' && !args.for) return true; // own capability
  if (cmd === 'actor revoke' && !args.target) return true; // self-revoke stays as-is
  return false;
}

/**
 * The authz gate. Called from signPayload AFTER signature verification,
 * so a refusal happens before anything is written (dry-run included).
 *
 * Refusals (all AUTHZ_DENIED, before any mutation):
 *  - authz on, command mapped, actor lacks the capability;
 *  - authz on, command NOT mapped (fail closed);
 *  - `actor revoke --target <who>` — the owner-mediated key kill needs the
 *    OWNER role REGARDLESS of authz mode (D8: SoD needs an owner to kill a
 *    compromised key, even before authz is switched on).
 *
 * @param {object|null} db - open company DB (null → no authz state).
 * @param {string} actor - the signed actor.
 * @param {string} cmd - command path.
 * @param {object} [args={}] - signed args.
 * @throws AUTHZ_DENIED.
 */
export function checkAuthz(db, actor, cmd, args = {}) {
  if (!db) return; // no DB (init on a fresh file) → no authz state
  const isOwnerKill = cmd === 'actor revoke' && Boolean(args.target);
  if (!isOwnerKill && isAuthzExempt(cmd, args)) return;
  if (getAuthz(db) !== 'on' && !isOwnerKill) return;
  if (isOwnerKill) {
    if (!hasRole(db, actor, 'owner')) {
      throw authzError(
        'AUTHZ_DENIED',
        `actor ${actor} needs the owner role to revoke another actor's key (--target) — ask the owner`,
      );
    }
    return;
  }
  const capability = capabilityOf(cmd, args);
  if (!capability || !canAct(db, actor, capability)) {
    const roles = getRoles(db, actor);
    throw authzError(
      'AUTHZ_DENIED',
      `actor ${actor} has no capability '${capability ?? '?'}' in this company`
      + `${roles.length ? ` (roles: ${roles.join(', ')})` : ' (no roles)'}`
      + `${capability ? '' : ' — the command has no capability mapping (fail closed)'}`
      + ' — ask the owner to grant it',
    );
  }
}
