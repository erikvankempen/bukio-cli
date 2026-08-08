/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Recurring entries & period automation (FR3A).
// Templates are validated at creation; generation replays resolved postings.
// Deterministic, dry-run friendly, fully audited — bukio never generates
// entries on its own; the agent or a cron job triggers `run --due`.
import { getAccountByCode } from '../core/accounts.js';
import { createEntry, parsePostingSpecs, postEntry, reverseEntry } from '../core/entries.js';
import { record } from '../audit/index.js';
import { expandVatPostings, parseVatPostingSpecs } from '../vat/index.js';
import { createInvoice, getContact, parseItemSpec, validateInvoiceLines } from '../invoice/index.js';
import { getItem } from '../items/index.js';

export function recurringError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

export const FREQUENCIES = ['monthly', 'quarterly', 'yearly'];
const KINDS = ['entry', 'invoice'];

const ISO = /^\d{4}-\d{2}-\d{2}$/;

function isValidDate(s) {
  if (!ISO.test(s)) return false;
  const [y, m, d] = s.split('-').map(Number);
  const dt = new Date(Date.UTC(y, m - 1, d));
  return dt.getUTCFullYear() === y && dt.getUTCMonth() === m - 1 && dt.getUTCDate() === d;
}

function todayIso() {
  return new Date().toISOString().slice(0, 10);
}

/** Advance a YYYY-MM-DD date by one frequency period, keeping `day`. */
function addDays(dateStr, days) {
  const [y, m, d] = dateStr.split('-').map(Number);
  return new Date(Date.UTC(y, m - 1, d + days)).toISOString().slice(0, 10);
}

export function addPeriod(dateStr, frequency, dayOfPeriod) {
  const [y, m] = dateStr.split('-').map(Number);
  const months = frequency === 'monthly' ? 1 : frequency === 'quarterly' ? 3 : 12;
  const total = y * 12 + (m - 1) + months;
  const ny = Math.floor(total / 12);
  const nm = (total % 12) + 1;
  return `${ny}-${String(nm).padStart(2, '0')}-${String(dayOfPeriod).padStart(2, '0')}`;
}

/** Validate the resolved posting set: accounts exist/active, non-zero, balanced. */
export function validatePostings(db, postings) {
  if (!Array.isArray(postings) || postings.length < 2) {
    throw recurringError('INVALID_POSTINGS', 'a template needs at least two postings');
  }
  let sum = 0;
  for (const p of postings) {
    if (!p.code || !Number.isInteger(p.amountCents) || p.amountCents === 0) {
      throw recurringError('INVALID_AMOUNT_CENTS', `posting for account ${p.code ?? '?'} must be a non-zero integer amount (cents)`);
    }
    const account = getAccountByCode(db, p.code);
    if (!account) throw recurringError('ACCOUNT_NOT_FOUND', `account ${p.code} does not exist`);
    if (account.active === 0) throw recurringError('ACCOUNT_INACTIVE', `account ${p.code} is deactivated`);
    sum += p.amountCents;
  }
  if (sum !== 0) throw recurringError('UNBALANCED', `postings do not sum to zero (sum = ${sum} cents)`);
  return postings;
}

/**
 * Create a recurring template. `postings` are posting specs; any with
 * `vatCode` are expanded via the VAT module (requires vat_module on).
 * Returns the template row.
 */
export function createTemplate(db, {
  name, description = null, frequency, dayOfPeriod = 1, startDate,
  endDate = null, runs = null, postings, reversePrevious = false, actor = 'human',
  kind = 'entry', contactId = null, invoiceLines = null, invoiceItems = null,
  dueDays = 30, dryRun = false,
}) {
  if (!name || typeof name !== 'string') throw recurringError('INVALID_NAME', 'template needs a name');
  if (!FREQUENCIES.includes(frequency)) {
    throw recurringError('INVALID_FREQUENCY', `frequency must be one of ${FREQUENCIES.join(', ')}`);
  }
  if (!KINDS.includes(kind)) throw recurringError('INVALID_KIND', `kind must be one of ${KINDS.join(', ')}`);
  if (kind === 'invoice' && reversePrevious) {
    throw recurringError('INVALID_REVERSE', 'reverse-previous only applies to entry templates (accrual pattern)');
  }
  if (!Number.isInteger(dayOfPeriod) || dayOfPeriod < 1 || dayOfPeriod > 28) {
    throw recurringError('INVALID_DATE', 'day of period must be between 1 and 28');
  }
  if (!isValidDate(startDate)) throw recurringError('INVALID_DATE', `start date '${startDate}' must be a valid YYYY-MM-DD`);
  if (endDate && !isValidDate(endDate)) throw recurringError('INVALID_DATE', `end date '${endDate}' must be a valid YYYY-MM-DD`);
  if (endDate && endDate < startDate) throw recurringError('INVALID_RANGE', 'end date must be on or after the start date');
  if (runs != null && (!Number.isInteger(runs) || runs < 1)) throw recurringError('INVALID_RUNS', 'runs must be a positive integer');
  if (!Number.isInteger(dueDays) || dueDays < 0) {
    throw recurringError('INVALID_DUE_DAYS', `due-days must be a non-negative integer, got '${dueDays}'`);
  }

  let postingsJson = '[]';
  let invoiceItemsJson = null;
  let vatAware = false;
  if (kind === 'invoice') {
    // invoice templates: validate the line/item specs at creation (no insert)
    if (!contactId) throw recurringError('INVALID_KIND', 'invoice templates need --contact');
    if (!getContact(db, contactId)) throw recurringError('CONTACT_NOT_FOUND', `contact ${contactId} does not exist`);
    const hasLines = Array.isArray(invoiceLines) ? invoiceLines.length > 0 : Boolean(invoiceLines);
    const hasItems = Array.isArray(invoiceItems) ? invoiceItems.length > 0 : Boolean(invoiceItems);
    if (!hasLines && !hasItems) throw recurringError('INVALID_KIND', 'invoice templates need --lines or --items');
    if (hasLines && hasItems) throw recurringError('INVALID_KIND', 'pass either --lines or --items, not both');
    if (hasItems) {
      // item specs are stored verbatim; catalog prices are snapshotted at
      // each generation (recurring run resolves them like invoice create)
      const specs = invoiceItems.flatMap((s) => (typeof s === 'string' ? [s] : [s]));
      for (const spec of specs) {
        const p = parseItemSpec(spec);
        const item = getItem(db, p.itemId);
        if (!item) throw recurringError('ITEM_NOT_FOUND', `item ${p.itemId} does not exist`);
        if (item.active !== 1) throw recurringError('ITEM_INACTIVE', `item ${p.itemId} is deactivated`);
        if (p.vatCode || item.vat_code) vatAware = true;
      }
      invoiceItemsJson = JSON.stringify(specs);
      postingsJson = '[]';
    } else {
      const parsed = validateInvoiceLines(db, invoiceLines);
      vatAware = parsed.some((l) => l.vatCode);
      postingsJson = JSON.stringify(parsed.map((l) => ({
        description: l.description, quantity: l.qtyMilli, priceCents: l.priceCents, vatCode: l.vatCode,
      })));
    }
  } else {
    // Resolve posting specs (CODE:AMOUNT[@VAT]) and expand VAT legs when tagged.
    // Strings are parsed; plain objects { code, amountCents } pass through.
    const raw = postings.flatMap((p) => (typeof p === 'string' ? [p] : [p]));
    vatAware = raw.some((p) => typeof p === 'string' && /@/.test(p));
    let resolved;
    if (vatAware) {
      // Expansion through the VAT module validates codes + computes VAT legs.
      resolved = expandVatPostings(db, parseVatPostingSpecs(raw.filter((p) => typeof p === 'string')));
    } else {
      resolved = raw.flatMap((p) => {
        if (typeof p === 'string') {
          // one string may carry several comma-separated postings
          return parsePostingSpecs([p]).map((s) => ({
            code: s.code, amountCents: s.amountCents, vatCode: null, vatAmountCents: null,
          }));
        }
        return [{
          code: p.code, amountCents: p.amountCents,
          vatCode: p.vatCode ?? null, vatAmountCents: p.vatAmountCents ?? null,
        }];
      });
    }
    validatePostings(db, resolved);
    postingsJson = JSON.stringify(resolved.map((p) => ({
      code: p.code, amountCents: p.amountCents,
      vatCode: p.vatCode ?? null, vatAmountCents: p.vatAmountCents ?? null,
    })));
  }

  // Normalize the first run to day_of_period (never backwards).
  let nextRun = startDate;
  const [, sy, sm, sd] = startDate.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (Number(sd) > dayOfPeriod) {
    nextRun = `${sy}-${sm}-${String(dayOfPeriod).padStart(2, '0')}`;
    nextRun = addPeriod(nextRun, frequency, dayOfPeriod);
  } else {
    nextRun = `${sy}-${sm}-${String(dayOfPeriod).padStart(2, '0')}`;
  }
  if (endDate && nextRun > endDate) {
    throw recurringError('INVALID_RANGE', 'the first run date falls after the end date');
  }

  if (dryRun) {
    // validate-everything-first, write nothing: same checks as the real path
    // (all thrown above), returns the plan for the CLI to render.
    return {
      action: 'recurring.create', kind, name, description, frequency,
      day_of_period: dayOfPeriod, start_date: startDate, end_date: endDate, runs,
      due_days: dueDays, reverse_previous: reversePrevious, contact_id: contactId,
      vat_aware: vatAware,
      postings: postingsJson === '[]' ? null : postings.map((p) => (typeof p === 'string' ? p : JSON.stringify(p))).join(', '),
      lines: invoiceLines ? invoiceLines.join(' + ') : null,
      items: invoiceItems ? invoiceItems.join(' + ') : null,
      next_run_date: nextRun, dryRun: true,
    };
  }

  const info = db.prepare(`
    INSERT INTO recurring_templates
      (name, description, frequency, day_of_period, start_date, end_date, runs,
       postings_json, reverse_previous, next_run_date, vat_aware, created_by,
       kind, contact_id, invoice_lines_json, invoice_items_json, due_days)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(name, description, frequency, dayOfPeriod, startDate, endDate, runs,
    postingsJson,
    reversePrevious ? 1 : 0, nextRun, vatAware ? 1 : 0, actor,
    kind, kind === 'invoice' ? contactId : null,
    kind === 'invoice' && !invoiceItemsJson ? postingsJson : null,
    invoiceItemsJson,
    kind === 'invoice' ? dueDays : null);

  record(db, {
    actor, action: 'recurring.template_add', command: 'recurring add',
    args: { name, frequency, dayOfPeriod, startDate, endDate, runs, reversePrevious, kind },
    outcome: 'ok',
  });
  return getTemplate(db, info.lastInsertRowid);
}

export function getTemplate(db, id) {
  const tpl = db.prepare('SELECT * FROM recurring_templates WHERE id = ?').get(id);
  if (!tpl) return null;
  tpl.postings = JSON.parse(tpl.postings_json);
  tpl.final_postings = tpl.final_postings_json ? JSON.parse(tpl.final_postings_json) : null;
  tpl.invoice_lines = tpl.invoice_lines_json ? JSON.parse(tpl.invoice_lines_json) : null;
  tpl.invoice_items = tpl.invoice_items_json ? JSON.parse(tpl.invoice_items_json) : null;
  return tpl;
}

export function listTemplates(db, { status = 'active' } = {}) {
  const where = status && status !== 'all' ? 'WHERE status = ?' : '';
  const rows = db.prepare(`SELECT * FROM recurring_templates ${where} ORDER BY next_run_date, id`).all(...(status && status !== 'all' ? [status] : []));
  for (const r of rows) {
    r.postings = JSON.parse(r.postings_json);
    r.invoice_lines = r.invoice_lines_json ? JSON.parse(r.invoice_lines_json) : null;
    r.invoice_items = r.invoice_items_json ? JSON.parse(r.invoice_items_json) : null;
  }
  return rows;
}

export function setTemplateStatus(db, { id, status, actor = 'human', dryRun = false }) {
  const tpl = getTemplate(db, id);
  if (!tpl) throw recurringError('NOT_FOUND', `recurring template ${id} does not exist`);
  if (tpl.status === 'completed') throw recurringError('ALREADY_COMPLETED', 'a completed template cannot be re-activated');
  if (dryRun) {
    return { action: `recurring.${status}`, id, from: tpl.status, to: status, dryRun: true };
  }
  db.prepare('UPDATE recurring_templates SET status = ? WHERE id = ?').run(status, id);
  record(db, { actor, action: `recurring.${status}`, command: 'recurring', args: { id }, outcome: 'ok' });
  return getTemplate(db, id);
}

/** Is the upcoming run the final one (uses final_postings_json)? */
function isFinalRun(tpl) {
  return Boolean(tpl.final_postings && tpl.runs && tpl.runs_done + 1 >= tpl.runs);
}

function runTemplateOnce(db, tpl, actor) {
  const generated = [];
  // Entries/invoices are machine-generated: created_by is always 'recurring'
  // (the trigger actor lives in the audit log of the run itself).
  const entryActor = 'recurring';

  let lastEntryId = null;
  if (tpl.kind === 'invoice') {
    // subscription invoice: generate a DRAFT invoice dated next_run_date.
    // Finalizing stays a separate audited action — never auto-finalize.
    // Item-based templates re-resolve the catalog at generation time, so a
    // price change applies from the next run (snapshot semantics).
    const inv = tpl.invoice_items
      ? createInvoice(db, {
          contactId: tpl.contact_id,
          items: tpl.invoice_items,
          date: tpl.next_run_date,
          dueDays: tpl.due_days ?? 30,
          description: tpl.name,
          actor: entryActor,
        })
      : createInvoice(db, {
          contactId: tpl.contact_id,
          lines: tpl.invoice_lines.map((l) => ({
            qtyMilli: l.quantity, description: l.description, priceCents: l.priceCents, vatCode: l.vatCode,
          })),
          date: tpl.next_run_date,
          dueDays: tpl.due_days ?? 30,
          description: tpl.name,
          actor: entryActor,
        });
    generated.push({ kind: 'invoice', invoice: { id: inv.id, invoice_number: null, status: 'draft', date: inv.date } });
  } else {
    const postings = isFinalRun(tpl) ? tpl.final_postings : tpl.postings;

    // accrual pattern: reverse the previous generated entry first
    if (tpl.reverse_previous && tpl.last_entry_id) {
      try {
        const reversal = reverseEntry(db, {
          id: tpl.last_entry_id, actor: entryActor, reason: `recurring template "${tpl.name}" — previous period reversal`,
        });
        generated.push({ kind: 'reversal', entry: reversal });
      } catch (err) {
        if (err.code === 'ALREADY_REVERSED' || err.code === 'NOT_POSTED') {
          // the previous accrual was already handled (manually reversed) — fine
        } else {
          throw err;
        }
      }
    }

    const entry = createEntry(db, {
      date: tpl.next_run_date,
      description: `${tpl.name} ${tpl.next_run_date}`,
      postings,
      source: 'recurring',
      sourceRef: `tpl:${tpl.id}`,
      actor: entryActor,
    });
    const posted = postEntry(db, { id: entry.id, actor: entryActor });
    lastEntryId = posted.id;
    generated.push({ kind: 'entry', entry: posted });
  }

  const nextRun = addPeriod(tpl.next_run_date, tpl.frequency, tpl.day_of_period);
  let status = tpl.status;
  const runsDone = tpl.runs_done + 1;
  if ((tpl.runs && runsDone >= tpl.runs) || (tpl.end_date && nextRun > tpl.end_date)) {
    status = 'completed';
  }
  db.prepare(`
    UPDATE recurring_templates
    SET next_run_date = ?, last_run_date = ?, last_entry_id = ?, runs_done = ?, status = ?
    WHERE id = ?
  `).run(nextRun, tpl.next_run_date, lastEntryId, runsDone, status, tpl.id);

  return { generated, status, runs_done: runsDone };
}

/**
 * Generate all due runs. Backfills: loops while next_run_date <= asOf
 * (capped at maxRunsPerTemplate). Each template runs in its own
 * transaction — a failing template is reported and skipped, the others
 * still run. dryRun returns the plan without writing anything.
 */
export function runDue(db, { asOf = null, templateId = null, actor = 'human', dryRun = false, maxRunsPerTemplate = 120 } = {}) {
  if (asOf != null && !isValidDate(asOf)) {
    throw recurringError('INVALID_DATE', `as-of '${asOf}' must be yyyy-mm-dd`);
  }
  const date = asOf || todayIso();
  let templates;
  if (templateId) {
    const tpl = getTemplate(db, templateId);
    templates = tpl && tpl.status === 'active' && tpl.next_run_date <= date ? [tpl] : [];
  } else {
    templates = db.prepare(`
      SELECT * FROM recurring_templates
      WHERE status = 'active' AND next_run_date <= ?
      ORDER BY next_run_date, id
    `).all(date);
  }
  // normalize: rows from raw SELECTs need their JSON columns parsed
  for (const t of templates) {
    if (!t.postings) t.postings = JSON.parse(t.postings_json);
    if (t.final_postings_json) t.final_postings = JSON.parse(t.final_postings_json);
    else t.final_postings = null;
    if (t.invoice_lines_json) t.invoice_lines = JSON.parse(t.invoice_lines_json);
    if (t.invoice_items_json) t.invoice_items = JSON.parse(t.invoice_items_json);
  }

  const results = [];
  for (const tpl of templates) {
    const tplResult = { template_id: tpl.id, name: tpl.name, runs: [] };
    try {
      const tx = db.transaction(() => {
        let current = tpl;
        for (let i = 0; i < maxRunsPerTemplate && current.next_run_date <= date; i += 1) {
          if (current.status !== 'active') break;
          tplResult.runs.push(runTemplateOnce(db, current, actor));
          current = getTemplate(db, tpl.id);
        }
      });
      if (!dryRun) tx();
      else {
        // dry-run: simulate without writing
        let sim = tpl;
        for (let i = 0; i < maxRunsPerTemplate && sim.next_run_date <= date; i += 1) {
          if (sim.status !== 'active') break;
          if (sim.kind === 'invoice') {
            const contact = getContact(db, sim.contact_id);
            tplResult.runs.push({
              kind: 'invoice',
              invoice: {
                date: sim.next_run_date,
                due_date: addDays(sim.next_run_date, sim.due_days ?? 0),
                contact_name: contact?.name ?? null,
                lines: sim.invoice_items ?? sim.invoice_lines ?? [],
              },
            });
          } else {
            const next = sim.runs && sim.runs_done + 1 >= sim.runs && sim.final_postings
              ? sim.final_postings : sim.postings;
            tplResult.runs.push({
              kind: sim.reverse_previous && sim.last_entry_id ? 'reversal' : null,
              entry: { date: sim.next_run_date, postings: next, description: `${sim.name} ${sim.next_run_date}` },
            });
          }
          sim = { ...sim, next_run_date: addPeriod(sim.next_run_date, sim.frequency, sim.day_of_period), runs_done: sim.runs_done + 1 };
          if ((sim.runs && sim.runs_done >= sim.runs) || (sim.end_date && sim.next_run_date > sim.end_date)) {
            sim.status = 'completed';
          }
        }
      }
      tplResult.ok = true;
    } catch (err) {
      tplResult.ok = false;
      tplResult.error = { code: err.code ?? 'RECURRING_ERROR', message: err.message };
    }
    if (tplResult.runs.length > 0 || !tplResult.ok) results.push(tplResult);
  }

  if (!dryRun && results.some((r) => r.ok && r.runs.length > 0)) {
    record(db, {
      actor, action: 'recurring.run', command: 'recurring run',
      args: { asOf: date, templates: results.filter((r) => r.ok).length }, outcome: 'ok',
      entryIds: results.flatMap((r) => r.runs.flatMap((run) => run.generated?.map((g) => g.entry?.id).filter(Boolean) ?? [])),
    });
  }
  return { as_of: date, dry_run: dryRun, templates: results };
}

/** What is due — same computation as runDue, but always read-only. */
export function previewDue(db, { asOf = null, templateId = null } = {}) {
  return runDue(db, { asOf, templateId, dryRun: true });
}

/**
 * Depreciation template builder (FR3A.2): linear monthly depreciation with a
 * remainder-adjusted final run so the asset fully depreciates to its residual
 * value in cents-exact totals.
 *   monthly = round((cost - residual) / life_months)
 *   final   = (cost - residual) - monthly * (life_months - 1)
 */
export function buildDepreciationTemplate(db, {
  name, assetCode = '1800', expenseCode = '4600', costCents, residualCents = 0,
  lifeMonths, startDate, description = null, actor = 'human', dryRun = false,
}) {
  if (!Number.isInteger(costCents) || costCents <= 0) {
    throw recurringError('INVALID_COST', 'cost must be a positive amount in cents');
  }
  if (!Number.isInteger(residualCents) || residualCents < 0 || residualCents >= costCents) {
    throw recurringError('INVALID_RESIDUAL', 'residual must be >= 0 and < cost');
  }
  if (!Number.isInteger(lifeMonths) || lifeMonths < 2) {
    throw recurringError('INVALID_LIFE', 'life-months must be an integer >= 2');
  }
  const depreciable = costCents - residualCents;
  const monthly = Math.round(depreciable / lifeMonths);
  if (monthly === 0) throw recurringError('INVALID_LIFE', 'life-months too long: monthly depreciation rounds to zero');
  const final = depreciable - monthly * (lifeMonths - 1);
  // the final run books `final` — a non-positive final would book zero or a
  // negative (reversing) depreciation leg; reject the template instead
  if (final <= 0) {
    throw recurringError('INVALID_LIFE', `life-months too long: the final run would be ${final} cents — shorten the life or raise the cost`);
  }
  if (dryRun) {
    return {
      template: null,
      monthly_cents: monthly, final_cents: final,
      total_cents: monthly * (lifeMonths - 1) + final,
      monthly: (monthly / 100).toFixed(2), final: (final / 100).toFixed(2),
      dryRun: true,
    };
  }
  const normalPostings = [
    { code: expenseCode, amountCents: monthly },
    { code: assetCode, amountCents: -monthly },
  ];
  const finalPostings = [
    { code: expenseCode, amountCents: final },
    { code: assetCode, amountCents: -final },
  ];
  const tpl = createTemplate(db, {
    name: name || `Afschrijving ${assetCode}`,
    description: description ?? `${lifeMonths} months linear, cost ${(costCents / 100).toFixed(2)} residual ${(residualCents / 100).toFixed(2)}`,
    frequency: 'monthly', dayOfPeriod: 1, startDate, runs: lifeMonths,
    postings: normalPostings, actor,
  });
  db.prepare('UPDATE recurring_templates SET final_postings_json = ? WHERE id = ?')
    .run(JSON.stringify(finalPostings), tpl.id);
  const updated = getTemplate(db, tpl.id);
  return {
    template: updated,
    monthly_cents: monthly, final_cents: final,
    total_cents: monthly * (lifeMonths - 1) + final,
    monthly: (monthly / 100).toFixed(2), final: (final / 100).toFixed(2),
  };
}
