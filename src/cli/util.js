// CLI helpers — context, JSON/human output, db access.
import { existsSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { openDb } from '../core/db.js';

export function dbError(code, message) {
  const e = new Error(message);
  e.code = code;
  return e;
}

/** Build the shared command context from global+local options. */
export function makeCtx(command) {
  const o = command.optsWithGlobals();
  return {
    json: Boolean(o.json),
    dbPath: o.db,
    actor: o.actor,
    dryRun: Boolean(o.dryRun),
  };
}

/** Emit results: JSON (stable schema) or human-rendered text. */
export function output(ctx, data, render) {
  if (ctx.json) {
    console.log(JSON.stringify({ ok: true, data }, null, 2));
  } else {
    render(data);
  }
}

/** Emit errors: JSON { ok:false, error:{code,message,details?} } or stderr lines. */
export function fail(ctx, err) {
  const code = err.code || 'ERROR';
  if (ctx.json) {
    const error = { code, message: err.message };
    if (err.details) error.details = err.details;
    console.log(JSON.stringify({ ok: false, error }, null, 2));
  } else {
    console.error(`error [${code}]: ${err.message}`);
    if (err.details) {
      for (const d of err.details) {
        console.error(`  line ${d.line || '-'}: ${d.error}`);
      }
    }
  }
  process.exitCode = 1;
}

/**
 * Open the database. create=true → mkdir + create file (for init).
 * mustExist=true (default) → throw NO_DATABASE when the file is absent.
 * dryRun handled by callers: pass mustExist=false and open only if present.
 */
export function ensureDb(ctx, { create = false, mustExist = true } = {}) {
  const exists = existsSync(ctx.dbPath);
  if (!exists) {
    if (create) {
      mkdirSync(path.dirname(ctx.dbPath), { recursive: true });
    } else if (mustExist) {
      throw dbError('NO_DATABASE', `no database at ${ctx.dbPath} — run 'bukio init' first`);
    }
  }
  return openDb(ctx.dbPath);
}

/** Simple aligned text table for human output. cols: [{ key, label }] */
export function table(rows, cols) {
  const widths = cols.map((c) => Math.max(
    c.label.length,
    ...rows.map((r) => String(r[c.key] ?? '').length),
  ));
  const line = (cells) => cells.map((c, i) => String(c).padEnd(widths[i])).join('  ');
  console.log(line(cols.map((c) => c.label)));
  console.log(widths.map((w) => '-'.repeat(w)).join('  '));
  for (const r of rows) console.log(line(cols.map((c) => r[c.key] ?? '')));
}
